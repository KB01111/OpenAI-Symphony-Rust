//! Engine discovery and the Graphify adapter.
//!
//! Graphify (Python, Apache-2.0/MIT) turns a directory into a graph with tree-sitter. It is run
//! as a separate process on a Symphony-owned snapshot directory, offline (`--code-only`,
//! `GRAPHIFY_BACKEND=none`, every LLM key removed from its environment), and its `graph.json` is
//! imported through [`import_graph`], which accepts the documented fields and ignores the rest.
//!
//! Symphony never installs an engine. Detection only reports what is on the machine and the
//! command the user can run to install it.

use crate::model::{Confidence, EngineStatus, GraphEdge, GraphNode, LanguageStat, NodeKind};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use symphony_core::{redact_secrets, SymphonyError, SymphonyResult};
use symphony_workspaces::{output_with_timeout, SECRET_ENV_VARS_TO_STRIP};

/// The Graphify release this build was verified against (`docs/KNOWLEDGE_BASE.md`).
pub const GRAPHIFY_VERIFIED_VERSION: &str = "0.9.97";
pub const GRAPHIFY_INSTALL_HINT: &str = "uv tool install graphifyy==0.9.97";
pub const SERENA_INSTALL_HINT: &str = "uv tool install -p 3.13 serena-agent && serena init";

/// Upper bound for a `graph.json` that is read into memory.
const MAX_GRAPH_JSON_BYTES: u64 = 256 * 1024 * 1024;
const DETECT_TIMEOUT: Duration = Duration::from_secs(20);

/// Environment variables that would let Graphify reach an LLM service. Removed for every run.
const LLM_ENV_VARS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_AUTH_TOKEN",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "MOONSHOT_API_KEY",
    "KIMI_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENROUTER_API_KEY",
    "GROQ_API_KEY",
    "MISTRAL_API_KEY",
    "AZURE_OPENAI_API_KEY",
];

// ---------------------------------------------------------------------------------------------
// Locating executables
// ---------------------------------------------------------------------------------------------

fn executable_names(name: &str) -> Vec<String> {
    if cfg!(windows) {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            format!("{name}.bat"),
            name.to_string(),
        ]
    } else {
        vec![name.to_string()]
    }
}

fn first_executable_in(dir: &Path, names: &[String]) -> Option<PathBuf> {
    names
        .iter()
        .map(|n| dir.join(n))
        .find(|candidate| candidate.is_file())
}

/// Directories a user-level tool installer (`uv tool`, `pipx`, `pip --user`) puts executables in.
/// A desktop app started from the Start menu does not always inherit the shell's `PATH`, so these
/// are searched too.
fn extra_tool_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".cargo").join("bin"));
    }
    if let Some(data) = dirs::data_dir() {
        // %APPDATA% on Windows (pip --user installs to Python3xx\Scripts below it).
        dirs.push(data.join("uv").join("bin"));
        if let Ok(entries) = std::fs::read_dir(data.join("Python")) {
            for entry in entries.flatten() {
                dirs.push(entry.path().join("Scripts"));
            }
        }
    }
    if let Some(local) = dirs::data_local_dir() {
        dirs.push(local.join("uv").join("bin"));
    }
    dirs
}

/// Resolve `name` (or an explicit override: a path, or a command name) to an executable file.
pub fn find_executable(name: &str, override_command: Option<&str>) -> Option<PathBuf> {
    if let Some(explicit) = override_command.map(str::trim).filter(|c| !c.is_empty()) {
        let path = PathBuf::from(explicit);
        if path.components().count() > 1 || path.is_absolute() {
            return path.is_file().then_some(path);
        }
        return find_executable(explicit, None);
    }
    let names = executable_names(name);
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if let Some(found) = first_executable_in(&dir, &names) {
                return Some(found);
            }
        }
    }
    extra_tool_dirs()
        .into_iter()
        .find_map(|dir| first_executable_in(&dir, &names))
}

pub(crate) fn command_for(path: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(path);
    for var in SECRET_ENV_VARS_TO_STRIP {
        cmd.env_remove(var);
    }
    cmd.env("NO_COLOR", "1")
        .env("PYTHONUTF8", "1")
        .env("PYTHONIOENCODING", "utf-8")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// First line of `<exe> --version`, with a leading program name removed (`graphify 0.9.97`).
async fn version_of(path: &Path) -> Option<String> {
    let mut cmd = command_for(path);
    cmd.arg("--version");
    let out = output_with_timeout(&mut cmd, DETECT_TIMEOUT).await?.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    Some(
        line.rsplit(|c: char| c.is_whitespace())
            .next()
            .unwrap_or(line)
            .trim_start_matches('v')
            .to_string(),
    )
}

/// Where the engines are, and the settings that override the search.
#[derive(Debug, Clone, Default)]
pub struct EngineConfig {
    pub graphify_command: Option<String>,
    pub serena_command: Option<String>,
}

impl EngineConfig {
    /// `SYMPHONY_GRAPHIFY` / `SYMPHONY_SERENA` name an executable explicitly.
    pub fn from_env() -> Self {
        let read = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            graphify_command: read("SYMPHONY_GRAPHIFY"),
            serena_command: read("SYMPHONY_SERENA"),
        }
    }

    pub fn graphify_path(&self) -> Option<PathBuf> {
        find_executable("graphify", self.graphify_command.as_deref())
    }

    pub fn serena_path(&self) -> Option<PathBuf> {
        find_executable("serena", self.serena_command.as_deref())
    }
}

/// Status of git, Graphify and Serena for the engines panel.
pub async fn detect_engines(config: &EngineConfig) -> Vec<EngineStatus> {
    let git = match find_executable("git", None) {
        Some(path) => EngineStatus {
            id: "git".into(),
            name: "Git".into(),
            available: true,
            required: true,
            version: version_of(&path).await,
            command: Some(path.display().to_string()),
            install_hint: None,
            detail: None,
        },
        None => EngineStatus {
            id: "git".into(),
            name: "Git".into(),
            available: false,
            required: true,
            version: None,
            command: None,
            install_hint: None,
            detail: Some("git was not found on PATH".into()),
        },
    };

    let graphify = match config.graphify_path() {
        Some(path) => {
            let version = version_of(&path).await;
            let detail = match version.as_deref() {
                Some(v) if v != GRAPHIFY_VERIFIED_VERSION => Some(format!(
                    "Verified with {GRAPHIFY_VERIFIED_VERSION}; this version may produce a graph.json the importer does not fully understand."
                )),
                None => Some("The executable did not report a version.".into()),
                _ => None,
            };
            EngineStatus {
                id: "graphify".into(),
                name: "Graphify".into(),
                available: version.is_some(),
                required: true,
                version,
                command: Some(path.display().to_string()),
                install_hint: Some(GRAPHIFY_INSTALL_HINT.into()),
                detail,
            }
        }
        None => EngineStatus {
            id: "graphify".into(),
            name: "Graphify".into(),
            available: false,
            required: true,
            version: None,
            command: config.graphify_command.clone(),
            install_hint: Some(GRAPHIFY_INSTALL_HINT.into()),
            detail: Some("Builds the code graph. Needs Python 3.10+ and uv; runs offline.".into()),
        },
    };

    let serena = match config.serena_path() {
        Some(path) => EngineStatus {
            id: "serena".into(),
            name: "Serena".into(),
            available: true,
            required: false,
            version: version_of(&path).await,
            command: Some(path.display().to_string()),
            install_hint: Some(SERENA_INSTALL_HINT.into()),
            detail: Some("Optional: precise symbol references. GPL-3.0, run only as a separate process you installed.".into()),
        },
        None => EngineStatus {
            id: "serena".into(),
            name: "Serena".into(),
            available: false,
            required: false,
            version: None,
            command: config.serena_command.clone(),
            install_hint: Some(SERENA_INSTALL_HINT.into()),
            detail: Some("Optional: precise symbol references through a language server. GPL-3.0, so Symphony only runs it as a separate process you installed.".into()),
        },
    };

    vec![git, graphify, serena]
}

// ---------------------------------------------------------------------------------------------
// Running Graphify
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct GraphifyEngine {
    path: PathBuf,
}

impl GraphifyEngine {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub async fn version(&self) -> Option<String> {
        version_of(&self.path).await
    }

    /// Extract the code graph of `snapshot` into `<out_dir>/graphify-out/graph.json` and return
    /// the file. Re-running on the same `out_dir` reuses Graphify's per-file cache, so only
    /// changed files are parsed again.
    pub async fn extract(
        &self,
        snapshot: &Path,
        out_dir: &Path,
        timeout: Duration,
    ) -> SymphonyResult<PathBuf> {
        std::fs::create_dir_all(out_dir)?;
        let mut cmd = command_for(&self.path);
        cmd.arg("extract")
            .arg(snapshot)
            .args(["--code-only", "--no-cluster", "--out"])
            .arg(out_dir)
            .env("GRAPHIFY_BACKEND", "none");
        for var in LLM_ENV_VARS {
            cmd.env_remove(var);
        }
        let output = match output_with_timeout(&mut cmd, timeout).await {
            None => {
                return Err(SymphonyError::GitWorktreeError(format!(
                    "graphify did not finish within {} seconds",
                    timeout.as_secs()
                )))
            }
            Some(Err(e)) => {
                return Err(SymphonyError::IoError(format!(
                    "could not start graphify: {e}"
                )))
            }
            Some(Ok(output)) => output,
        };
        if !output.status.success() {
            let tail = |bytes: &[u8]| {
                let text = String::from_utf8_lossy(bytes);
                let lines: Vec<&str> = text.lines().rev().take(12).collect();
                lines.into_iter().rev().collect::<Vec<_>>().join("\n")
            };
            let detail = format!(
                "graphify exited with {}: {}\n{}",
                output.status,
                tail(&output.stderr),
                tail(&output.stdout)
            );
            return Err(SymphonyError::IoError(redact_secrets(detail.trim(), &[])));
        }
        let graph = out_dir.join("graphify-out").join("graph.json");
        if !graph.is_file() {
            return Err(SymphonyError::IoError(
                "graphify finished but wrote no graph.json".into(),
            ));
        }
        Ok(graph)
    }
}

/// Read a `graph.json` (size-capped).
pub fn read_graph_file(path: &Path) -> SymphonyResult<Vec<u8>> {
    let size = std::fs::metadata(path)?.len();
    if size > MAX_GRAPH_JSON_BYTES {
        return Err(SymphonyError::IoError(format!(
            "graph.json is {size} bytes, over the {MAX_GRAPH_JSON_BYTES} byte limit"
        )));
    }
    Ok(std::fs::read(path)?)
}

// ---------------------------------------------------------------------------------------------
// Importing graph.json
// ---------------------------------------------------------------------------------------------

/// Edge relations that mean "this depends on that".
pub const DEPENDENCY_RELATIONS: &[&str] = &[
    "imports",
    "imports_from",
    "calls",
    "indirect_call",
    "references",
    "re_exports",
    "extends",
    "implements",
    "inherits",
    "mixes_in",
    "uses",
    "depends_on",
    "dynamic_import",
];

pub fn is_dependency_relation(relation: &str) -> bool {
    DEPENDENCY_RELATIONS.contains(&relation)
}

const SOURCE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "py", "go", "java", "kt", "kts",
    "swift", "c", "h", "cc", "cpp", "hpp", "cs", "rb", "php", "scala", "lua", "zig", "dart", "sh",
    "ps1", "ex", "exs", "erl", "hs", "ml", "jl", "vue", "svelte",
];

fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = name.rsplit_once('.')?;
    (!stem.is_empty()).then_some(ext)
}

pub fn is_source_file(path: &str) -> bool {
    extension(path)
        .map(|e| SOURCE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn language_of(path: &str) -> String {
    let ext = extension(path).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "Rust",
        "ts" | "tsx" | "mts" | "cts" => "TypeScript",
        "js" | "jsx" | "mjs" | "cjs" => "JavaScript",
        "py" => "Python",
        "go" => "Go",
        "java" => "Java",
        "kt" | "kts" => "Kotlin",
        "swift" => "Swift",
        "c" | "h" => "C",
        "cc" | "cpp" | "hpp" => "C++",
        "cs" => "C#",
        "rb" => "Ruby",
        "php" => "PHP",
        "sh" => "Shell",
        "ps1" => "PowerShell",
        "yml" | "yaml" => "YAML",
        "json" => "JSON",
        "toml" => "TOML",
        "css" => "CSS",
        "html" => "HTML",
        "md" => "Markdown",
        "sql" => "SQL",
        "" => "Other",
        other => return other.to_ascii_uppercase(),
    }
    .to_string()
}

/// `"L42"` -> 42.
fn parse_line(value: Option<&serde_json::Value>) -> Option<u32> {
    let text = value?.as_str()?;
    text.trim_start_matches(['L', 'l'])
        .parse()
        .ok()
        .filter(|n| *n > 0)
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Directories that belong to the package of a manifest at the repository root.
const ROOT_PACKAGE_DIRS: &[&str] = &["src", "lib", "app", "tests", "test", "benches", "examples"];

/// The module a file belongs to: the nearest directory with a package manifest, else the first
/// path component, else the repository root. A manifest at the root is the empty directory in
/// `manifest_dirs`: it makes the root the package of the files beside it and under `src/` and the
/// like (a monorepo's other directories keep their own names).
pub fn module_of(file: &str, manifest_dirs: &HashSet<String>) -> String {
    let mut dir = file.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    loop {
        if !dir.is_empty() && manifest_dirs.contains(dir) {
            return dir.to_string();
        }
        match dir.rsplit_once('/') {
            Some((parent, _)) => dir = parent,
            None => break,
        }
    }
    match file.split_once('/') {
        Some((first, _)) if manifest_dirs.contains("") && ROOT_PACKAGE_DIRS.contains(&first) => {
            ".".to_string()
        }
        Some((first, _)) => first.to_string(),
        None => ".".to_string(),
    }
}

/// The imported graph plus counts the index run reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub languages: Vec<LanguageStat>,
    pub file_count: u32,
    /// Edges dropped because an endpoint was not a node.
    pub dangling_edges: u32,
}

/// Convert Graphify's `graph.json` into Symphony's graph. `known_files` are the repository files
/// that were handed to the engine; a node whose file is not among them is an external dependency.
pub fn import_graph(json: &[u8], known_files: &HashSet<String>) -> SymphonyResult<ImportedGraph> {
    let value: serde_json::Value = serde_json::from_slice(json)
        .map_err(|e| SymphonyError::JsonError(format!("graph.json is not valid JSON: {e}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| SymphonyError::JsonError("graph.json is not an object".into()))?;
    let raw_nodes = object
        .get("nodes")
        .and_then(|v| v.as_array())
        .ok_or_else(|| SymphonyError::JsonError("graph.json has no `nodes` array".into()))?;
    let raw_edges = object
        .get("edges")
        .or_else(|| object.get("links"))
        .and_then(|v| v.as_array())
        .ok_or_else(|| SymphonyError::JsonError("graph.json has no `edges` array".into()))?;

    let manifest_dirs: HashSet<String> = known_files
        .iter()
        .filter(|f| matches!(file_name(f), "Cargo.toml" | "package.json"))
        .map(|f| f.rsplit_once('/').map(|(d, _)| d).unwrap_or("").to_string())
        .collect();

    // Pass 1: which nodes are methods / enum cases, from the structural edges.
    let mut method_targets: HashSet<&str> = HashSet::new();
    let mut case_targets: HashSet<&str> = HashSet::new();
    for edge in raw_edges {
        let (Some(target), Some(relation)) = (
            edge.get("target").and_then(|v| v.as_str()),
            edge.get("relation").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        match relation {
            "method" => {
                method_targets.insert(target);
            }
            "case_of" => {
                case_targets.insert(target);
            }
            _ => {}
        }
    }

    let mut nodes: Vec<GraphNode> = Vec::with_capacity(raw_nodes.len());
    let mut index: HashMap<String, usize> = HashMap::with_capacity(raw_nodes.len());
    let mut language_files: BTreeMap<String, u32> = BTreeMap::new();
    let mut file_count = 0u32;
    let mut dropped: HashSet<String> = HashSet::new();

    for raw in raw_nodes {
        let Some(id) = raw.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let file_type = raw.get("file_type").and_then(|v| v.as_str()).unwrap_or("");
        // Comments that explain code ("rationale") are not part of the structure.
        if file_type == "rationale" {
            dropped.insert(id.to_string());
            continue;
        }
        let label = raw
            .get("label")
            .and_then(|v| v.as_str())
            .unwrap_or(id)
            .to_string();
        let source_file = raw
            .get("source_file")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .replace('\\', "/");
        let node_type = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let external_flag = raw
            .get("external")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let line = parse_line(raw.get("source_location"));
        let in_repo = !source_file.is_empty() && known_files.contains(&source_file);

        let (kind, file) = if external_flag
            || matches!(node_type, "external" | "module")
            || file_type == "concept"
            || !in_repo
        {
            (NodeKind::Dependency, None)
        } else if node_type == "package" {
            (NodeKind::Package, Some(source_file.clone()))
        } else if label == file_name(&source_file) && line.map(|l| l <= 1).unwrap_or(true) {
            (NodeKind::File, Some(source_file.clone()))
        } else if !is_source_file(&source_file) || case_targets.contains(id) {
            (NodeKind::Other, Some(source_file.clone()))
        } else if label.ends_with("()") {
            if label.starts_with('.') || method_targets.contains(id) {
                (NodeKind::Method, Some(source_file.clone()))
            } else {
                (NodeKind::Function, Some(source_file.clone()))
            }
        } else {
            (NodeKind::Type, Some(source_file.clone()))
        };

        if kind == NodeKind::File {
            file_count += 1;
            *language_files.entry(language_of(&source_file)).or_default() += 1;
        }
        let module = file.as_deref().map(|f| module_of(f, &manifest_dirs));
        index.insert(id.to_string(), nodes.len());
        nodes.push(GraphNode {
            id: id.to_string(),
            label,
            kind,
            file,
            line,
            module,
            degree: 0,
            members: 1,
        });
    }

    let mut edges: Vec<GraphEdge> = Vec::with_capacity(raw_edges.len());
    let mut dangling = 0u32;
    for raw in raw_edges {
        let (Some(source), Some(target), Some(relation)) = (
            raw.get("source").and_then(|v| v.as_str()),
            raw.get("target").and_then(|v| v.as_str()),
            raw.get("relation").and_then(|v| v.as_str()),
        ) else {
            continue;
        };
        let (Some(&si), Some(&ti)) = (index.get(source), index.get(target)) else {
            // Edges from or to dropped rationale nodes are expected; anything else is dangling.
            if !(dropped.contains(source) || dropped.contains(target)) {
                dangling += 1;
            }
            continue;
        };
        if is_dependency_relation(relation) {
            nodes[si].degree += 1;
            nodes[ti].degree += 1;
        }
        edges.push(GraphEdge {
            source: source.to_string(),
            target: target.to_string(),
            relation: relation.to_string(),
            confidence: Confidence::parse(
                raw.get("confidence")
                    .and_then(|v| v.as_str())
                    .unwrap_or("INFERRED"),
            ),
            line: parse_line(raw.get("source_location")),
            weight: 1,
        });
    }

    let mut languages: Vec<LanguageStat> = language_files
        .into_iter()
        .map(|(language, files)| LanguageStat { language, files })
        .collect();
    languages.sort_by(|a, b| {
        b.files
            .cmp(&a.files)
            .then_with(|| a.language.cmp(&b.language))
    });

    Ok(ImportedGraph {
        nodes,
        edges,
        languages,
        file_count,
        dangling_edges: dangling,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn known(files: &[&str]) -> HashSet<String> {
        files.iter().map(|f| f.to_string()).collect()
    }

    #[test]
    fn nodes_are_classified_from_graphify_fields() {
        let graph = json!({
            "nodes": [
                {"id": "lib", "label": "lib.rs", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L1"},
                {"id": "lib_widget", "label": "Widget", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L10"},
                {"id": "lib_widget_run", "label": ".run()", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L14"},
                {"id": "lib_helper", "label": "helper()", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L30"},
                {"id": "lib_kind_big", "label": "Big", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L40"},
                {"id": "kind", "label": "Kind", "file_type": "code", "source_file": "crates/a/src/lib.rs", "source_location": "L38"},
                {"id": "pkg_a", "label": "a", "file_type": "code", "type": "package", "source_file": "crates/a/Cargo.toml", "source_location": "L1"},
                {"id": "ext_tokio", "label": "tokio", "file_type": "concept", "type": "external", "external": true, "source_file": ""},
                {"id": "note", "label": "TODO: later", "file_type": "rationale", "source_file": "crates/a/src/lib.rs", "source_location": "L50"},
                {"id": "ci_key", "label": "version", "file_type": "code", "source_file": ".github/ci.yml", "source_location": "L3"}
            ],
            "edges": [
                {"source": "lib", "target": "lib_widget", "relation": "contains", "confidence": "EXTRACTED", "source_location": "L10"},
                {"source": "lib_widget", "target": "lib_widget_run", "relation": "method", "confidence": "EXTRACTED", "source_location": "L14"},
                {"source": "lib_helper", "target": "lib_widget_run", "relation": "calls", "confidence": "INFERRED", "source_location": "L31"},
                {"source": "kind", "target": "lib_kind_big", "relation": "case_of", "confidence": "EXTRACTED"},
                {"source": "lib", "target": "ext_tokio", "relation": "imports_from", "confidence": "EXTRACTED", "source_location": "L2"},
                {"source": "note", "target": "lib_helper", "relation": "rationale_for", "confidence": "EXTRACTED"},
                {"source": "lib", "target": "ghost", "relation": "calls", "confidence": "EXTRACTED"}
            ]
        });
        let files = known(&[
            "crates/a/src/lib.rs",
            "crates/a/Cargo.toml",
            ".github/ci.yml",
        ]);
        let imported = import_graph(graph.to_string().as_bytes(), &files).unwrap();
        let kind = |id: &str| imported.nodes.iter().find(|n| n.id == id).unwrap().kind;

        assert_eq!(kind("lib"), NodeKind::File);
        assert_eq!(kind("lib_widget"), NodeKind::Type);
        assert_eq!(kind("lib_widget_run"), NodeKind::Method);
        assert_eq!(kind("lib_helper"), NodeKind::Function);
        assert_eq!(
            kind("lib_kind_big"),
            NodeKind::Other,
            "an enum case is not a type"
        );
        assert_eq!(kind("pkg_a"), NodeKind::Package);
        assert_eq!(kind("ext_tokio"), NodeKind::Dependency);
        assert_eq!(
            kind("ci_key"),
            NodeKind::Other,
            "a YAML key is not a code symbol"
        );
        assert!(
            imported.nodes.iter().all(|n| n.id != "note"),
            "rationale comments are dropped"
        );

        let widget_run = imported
            .nodes
            .iter()
            .find(|n| n.id == "lib_widget_run")
            .unwrap();
        assert_eq!(widget_run.line, Some(14));
        assert_eq!(widget_run.module.as_deref(), Some("crates/a"));
        // The edge to the missing node is dangling; edges to dropped rationale nodes are not counted.
        assert_eq!(imported.dangling_edges, 1);
        let call = imported
            .edges
            .iter()
            .find(|e| e.relation == "calls")
            .unwrap();
        assert_eq!(call.confidence, Confidence::Inferred);
        assert_eq!(call.line, Some(31));
        // `degree` counts dependency edges only (not `contains` / `method`).
        assert_eq!(widget_run.degree, 1);
        assert_eq!(imported.file_count, 1);
        assert_eq!(
            imported.languages[0],
            LanguageStat {
                language: "Rust".into(),
                files: 1
            }
        );
    }

    #[test]
    fn unexpected_shapes_are_reported_not_guessed() {
        assert!(import_graph(b"[]", &HashSet::new()).is_err());
        assert!(import_graph(b"{\"nodes\": []}", &HashSet::new()).is_err());
        assert!(import_graph(b"not json", &HashSet::new()).is_err());
        // `links` is accepted for the edge list.
        assert!(import_graph(b"{\"nodes\": [], \"links\": []}", &HashSet::new()).is_ok());
    }

    #[test]
    fn modules_follow_the_nearest_manifest_directory() {
        let manifests: HashSet<String> = ["crates/a", "packages/ui", "packages/ui/inner"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(module_of("crates/a/src/lib.rs", &manifests), "crates/a");
        assert_eq!(
            module_of("packages/ui/inner/x.ts", &manifests),
            "packages/ui/inner"
        );
        assert_eq!(module_of("docs/guide.md", &manifests), "docs");
        assert_eq!(module_of("README.md", &manifests), ".");
        // Without a root manifest `src/` is just a directory.
        assert_eq!(module_of("src/main.rs", &manifests), "src");
    }

    #[test]
    fn a_manifest_at_the_root_makes_the_root_the_package_of_its_source_directories() {
        let manifests: HashSet<String> = ["", "crates/a"].iter().map(|s| s.to_string()).collect();
        assert_eq!(module_of("src/main.rs", &manifests), ".");
        assert_eq!(module_of("tests/it.rs", &manifests), ".");
        assert_eq!(module_of("crates/a/src/lib.rs", &manifests), "crates/a");
        assert_eq!(module_of("docs/guide.md", &manifests), "docs");
    }

    #[test]
    fn line_locations_parse() {
        assert_eq!(parse_line(Some(&json!("L42"))), Some(42));
        assert_eq!(parse_line(Some(&json!(""))), None);
        assert_eq!(parse_line(None), None);
    }
}
