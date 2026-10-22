//! Checked OpenAPI contract.
//!
//! The OpenAPI document is generated from the `#[utoipa::path]` annotations of the handlers that
//! the router actually serves, so it cannot drift from the routes. This test fails when the
//! committed `openapi/symphony-api.json` (the input of the TypeScript client generator) is stale
//! and checks the invariants clients rely on.
//!
//! Regenerate with:
//! `UPDATE_OPENAPI=1 cargo test -p symphony-server --test openapi_contract`

use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::Value;

const METHODS: &[&str] = &["get", "put", "post", "delete", "patch"];

fn spec_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openapi/symphony-api.json")
}

fn document() -> Value {
    serde_json::to_value(symphony_server::openapi()).expect("serialize OpenAPI document")
}

fn operations(doc: &Value) -> Vec<(String, String, &Value)> {
    let mut out = Vec::new();
    for (path, item) in doc["paths"].as_object().expect("paths") {
        for method in METHODS {
            if let Some(op) = item.get(*method) {
                out.push((path.clone(), method.to_string(), op));
            }
        }
    }
    out
}

#[test]
fn openapi_document_is_up_to_date() {
    // Serialized directly (not via `serde_json::Value`) to keep declaration order.
    let generated = symphony_server::openapi()
        .to_pretty_json()
        .expect("serialize OpenAPI document")
        + "\n";
    let path = spec_path();
    if std::env::var("UPDATE_OPENAPI").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &generated).expect("write OpenAPI document");
        return;
    }
    let committed = std::fs::read_to_string(&path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        committed == generated,
        "openapi/symphony-api.json is out of date with the Rust API.\n\
         Run: UPDATE_OPENAPI=1 cargo test -p symphony-server --test openapi_contract\n\
         then: pnpm generate:api"
    );
}

#[test]
fn every_operation_has_a_unique_id_a_tag_and_an_error_response() {
    let doc = document();
    let mut ids = HashSet::new();
    for (path, method, op) in operations(&doc) {
        let id = op["operationId"]
            .as_str()
            .unwrap_or_else(|| panic!("{method} {path} has no operationId"));
        assert!(ids.insert(id.to_string()), "duplicate operationId {id}");
        assert!(
            op["tags"].as_array().is_some_and(|t| !t.is_empty()),
            "{method} {path} has no tag"
        );
        assert!(
            op["responses"]["default"].is_object(),
            "{method} {path} does not document the error body"
        );
    }
    assert!(
        ids.len() >= 40,
        "expected the full API, found {} operations",
        ids.len()
    );
}

#[test]
fn every_path_placeholder_is_a_documented_path_parameter() {
    let doc = document();
    for (path, method, op) in operations(&doc) {
        let documented: HashSet<&str> = op["parameters"]
            .as_array()
            .map(|params| {
                params
                    .iter()
                    .filter(|p| p["in"] == "path")
                    .filter_map(|p| p["name"].as_str())
                    .collect()
            })
            .unwrap_or_default();
        let placeholders: HashSet<&str> = path
            .split('/')
            .filter_map(|seg| seg.strip_prefix('{')?.strip_suffix('}'))
            .collect();
        assert_eq!(
            placeholders, documented,
            "{method} {path}: path parameters must match the placeholders"
        );
    }
}

#[test]
fn only_health_and_runtime_are_public() {
    let doc = document();
    assert_eq!(
        doc["components"]["securitySchemes"][symphony_server::BEARER_SCHEME]["scheme"],
        "bearer"
    );
    for (path, method, op) in operations(&doc) {
        let public = path == "/api/v1/health" || path == "/api/v1/runtime";
        let secured = op["security"].as_array().is_some_and(|reqs| {
            reqs.iter()
                .any(|r| r.get(symphony_server::BEARER_SCHEME).is_some())
        });
        assert_eq!(
            secured, !public,
            "{method} {path}: bearer security must be required exactly on protected routes"
        );
    }
}

#[test]
fn response_schemas_require_every_field() {
    // Clients rely on the server serializing every field (`null` for absent values).
    let doc = document();
    for name in [
        "Task",
        "Project",
        "AgentEvent",
        "AppSettings",
        "WorkflowDocument",
    ] {
        let schema = &doc["components"]["schemas"][name];
        // Fields added after 2.0 to schemas that are also request bodies stay optional so older
        // clients can keep omitting them (the server still always serializes them).
        let added_later: &[&str] = match name {
            "AppSettings" => &["remote_allowed_origins", "event_retention_days"],
            _ => &[],
        };
        let props: HashSet<&str> = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("{name} has no properties"))
            .keys()
            .map(String::as_str)
            .filter(|p| !added_later.contains(p))
            .collect();
        let required: HashSet<&str> = schema["required"]
            .as_array()
            .unwrap_or_else(|| panic!("{name} has no required list"))
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(
            props, required,
            "{name} must list every property as required"
        );
    }
    // Optional request fields stay optional.
    let create = &doc["components"]["schemas"]["CreateTaskRequest"]["required"];
    let required: Vec<&str> = create
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(required, ["project_id", "title"]);
}
