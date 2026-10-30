//! Serena as an optional, read-only symbol navigator.
//!
//! Serena (GPL-3.0-or-later) is run **only as a separate process the user installed**: it is never
//! bundled, linked or redistributed. It is spoken to over MCP's stdio transport (one JSON-RPC
//! message per line) and used for what a language server does better than a syntax parser: the
//! exact range of a symbol and who references it.
//!
//! It is held to read-only use twice over:
//!
//! - the mode file passed at launch excludes every tool that edits files, runs commands, reads
//!   arbitrary files or touches Serena's memories, so the server does not offer them;
//! - [`SerenaSession::call_tool`] only ever calls the tools in [`ALLOWED_TOOLS`], and after the
//!   handshake the session is refused if the server offers an editing tool anyway.
//!
//! Serena works in its own copy of the indexed commit (`<knowledge>/serena/<project>`), because it
//! writes a `.serena` directory next to the code it reads.

use crate::engine::command_for;
use crate::model::{SymbolReference, SymbolReferences};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use symphony_core::{SymphonyError, SymphonyResult};
use symphony_workspaces::{output_with_timeout, prepare_tree_command, ProcessTree};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};

/// The only Serena tools this crate calls.
pub const ALLOWED_TOOLS: &[&str] = &[
    "find_symbol",
    "find_referencing_symbols",
    "get_symbols_overview",
    "find_declaration",
    "find_implementations",
];

/// Tools that change files, run commands, read arbitrary files or write Serena's memories. They
/// are excluded in the mode file, and a server that offers one anyway is not used.
pub const EXCLUDED_TOOLS: &[&str] = &[
    "activate_project",
    "create_text_file",
    "delete_lines",
    "delete_memory",
    "edit_memory",
    "execute_shell_command",
    "find_file",
    "insert_after_symbol",
    "insert_at_line",
    "insert_before_symbol",
    "list_dir",
    "onboarding",
    "read_file",
    "rename_memory",
    "rename_symbol",
    "replace_content",
    "replace_in_files",
    "replace_lines",
    "replace_symbol_body",
    "safe_delete_symbol",
    "search_for_pattern",
    "write_memory",
];

/// How long the first call may take: the language server loads the project before it answers.
pub const WARMUP_TIMEOUT: Duration = Duration::from_secs(240);
pub const CALL_TIMEOUT: Duration = Duration::from_secs(60);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_REFERENCES: usize = 100;

/// Language names of the index mapped to Serena's language ids.
pub fn serena_language(language: &str) -> Option<&'static str> {
    Some(match language {
        "Rust" => "rust",
        "TypeScript" | "JavaScript" => "typescript",
        "Python" => "python",
        "Go" => "go",
        "Java" => "java",
        "Kotlin" => "kotlin",
        "C#" => "csharp",
        "Ruby" => "ruby",
        "PHP" => "php",
        "Swift" => "swift",
        "C" | "C++" => "cpp",
        _ => return None,
    })
}

/// The mode file that hides every tool but the read-only symbol ones.
pub fn mode_file_contents() -> String {
    let mut out = String::from("description: Symphony Knowledge Base read-only symbol navigation\nprompt: |\n  Read-only symbol navigation.\nexcluded_tools:\n");
    for tool in EXCLUDED_TOOLS {
        out.push_str(&format!("  - {tool}\n"));
    }
    out
}

/// Create Serena's project configuration for `dir` (`serena project create`), once.
pub async fn ensure_project(
    command: &Path,
    dir: &Path,
    name: &str,
    languages: &[&str],
) -> SymphonyResult<()> {
    let config = dir.join(".serena").join("project.yml");
    let existing = std::fs::read_to_string(&config).ok();
    if let Some(text) = &existing {
        // A repository that gained a language needs a new configuration; the old one only knows
        // the languages it was created for.
        if languages.iter().all(|l| config_lists_language(text, l)) {
            return Ok(());
        }
    }
    if languages.is_empty() {
        return Err(SymphonyError::Conflict(
            "none of the repository's languages is supported by Serena".into(),
        ));
    }
    if existing.is_some() {
        let _ = std::fs::remove_file(&config);
    }
    let mut cmd = command_for(command);
    cmd.arg("project")
        .arg("create")
        .arg(dir)
        .arg("--name")
        .arg(name);
    for language in languages {
        cmd.arg("--language").arg(language);
    }
    match output_with_timeout(&mut cmd, HANDSHAKE_TIMEOUT).await {
        Some(Ok(out)) if out.status.success() => Ok(()),
        Some(Ok(out)) => Err(SymphonyError::IoError(format!(
            "serena project create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ))),
        Some(Err(e)) => Err(SymphonyError::IoError(format!(
            "could not start serena: {e}"
        ))),
        None => Err(SymphonyError::IoError(
            "serena project create timed out".into(),
        )),
    }
}

/// Whether a `project.yml` written by `serena project create` names `language`.
fn config_lists_language(config: &str, language: &str) -> bool {
    config.lines().any(|line| {
        let line = line.trim().trim_start_matches('-').trim();
        line.trim_start_matches("language:")
            .trim()
            .trim_matches(['"', '\''])
            == language
    })
}

/// A running Serena MCP server on one project directory.
pub struct SerenaSession {
    // Declared before `child`: the tree is killed while the leader is still unreaped.
    tree: ProcessTree,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    calls: u64,
}

impl SerenaSession {
    pub async fn start(
        command: &Path,
        project_dir: &Path,
        mode_file: &Path,
    ) -> SymphonyResult<Self> {
        if let Some(parent) = mode_file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(mode_file, mode_file_contents())?;
        let mut cmd = command_for(command);
        cmd.arg("start-mcp-server")
            .arg("--project")
            .arg(project_dir)
            .args(["--context", "ide", "--mode"])
            .arg(mode_file)
            .args([
                "--enable-web-dashboard",
                "false",
                "--open-web-dashboard",
                "false",
                "--enable-gui-log-window",
                "false",
                "--log-level",
                "WARNING",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        prepare_tree_command(&mut cmd);
        let mut child = cmd
            .spawn()
            .map_err(|e| SymphonyError::IoError(format!("could not start serena: {e}")))?;
        let tree = ProcessTree::attach(&child);
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| SymphonyError::IoError("serena has no stdin".into()))?;
        let stdout = BufReader::new(
            child
                .stdout
                .take()
                .ok_or_else(|| SymphonyError::IoError("serena has no stdout".into()))?,
        );
        let mut session = Self {
            tree,
            child,
            stdin,
            stdout,
            next_id: 0,
            calls: 0,
        };
        session
            .request(
                "initialize",
                json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "clientInfo": { "name": "symphony-knowledge", "version": env!("CARGO_PKG_VERSION") }
                }),
                HANDSHAKE_TIMEOUT,
            )
            .await?;
        session.notify("notifications/initialized").await?;
        let tools = session
            .request("tools/list", json!({}), HANDSHAKE_TIMEOUT)
            .await?;
        let offered = offered_tools(&tools);
        if let Some(bad) = offered
            .iter()
            .find(|t| EXCLUDED_TOOLS.contains(&t.as_str()))
        {
            session.tree.kill();
            return Err(SymphonyError::Conflict(format!(
                "Serena offered the tool `{bad}`, which can change files or run commands; it will not be used"
            )));
        }
        Ok(session)
    }

    async fn write_line(&mut self, message: &Value) -> SymphonyResult<()> {
        let mut line = message.to_string();
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| SymphonyError::IoError(format!("serena closed its input: {e}")))?;
        self.stdin
            .flush()
            .await
            .map_err(|e| SymphonyError::IoError(format!("serena closed its input: {e}")))
    }

    async fn notify(&mut self, method: &str) -> SymphonyResult<()> {
        self.write_line(&json!({ "jsonrpc": "2.0", "method": method }))
            .await
    }

    /// Send a request and wait for its response; notifications and unrelated lines are skipped.
    async fn request(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> SymphonyResult<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.write_line(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;
        let read = async {
            let mut line = String::new();
            loop {
                line.clear();
                let n = self
                    .stdout
                    .read_line(&mut line)
                    .await
                    .map_err(|e| SymphonyError::IoError(format!("serena output: {e}")))?;
                if n == 0 {
                    return Err(SymphonyError::IoError("serena exited".into()));
                }
                let Ok(message) = serde_json::from_str::<Value>(line.trim()) else {
                    continue;
                };
                if message.get("id").and_then(Value::as_u64) != Some(id) {
                    continue;
                }
                if let Some(error) = message.get("error") {
                    return Err(SymphonyError::IoError(format!("serena: {error}")));
                }
                return Ok(message.get("result").cloned().unwrap_or(Value::Null));
            }
        };
        match tokio::time::timeout(timeout, read).await {
            Ok(result) => result,
            Err(_) => {
                self.tree.kill();
                Err(SymphonyError::IoError(format!(
                    "serena did not answer `{method}` within {} seconds",
                    timeout.as_secs()
                )))
            }
        }
    }

    /// Call one of the read-only tools and return its text output.
    pub async fn call_tool(&mut self, tool: &str, arguments: Value) -> SymphonyResult<String> {
        if !ALLOWED_TOOLS.contains(&tool) {
            return Err(SymphonyError::InvalidConfig(format!(
                "`{tool}` is not a read-only Serena tool"
            )));
        }
        let timeout = if self.calls == 0 {
            WARMUP_TIMEOUT
        } else {
            CALL_TIMEOUT
        };
        self.calls += 1;
        let result = self
            .request(
                "tools/call",
                json!({ "name": tool, "arguments": arguments }),
                timeout,
            )
            .await?;
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|parts| {
                parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(SymphonyError::IoError(format!(
                "serena {tool}: {}",
                text.trim()
            )));
        }
        Ok(text)
    }

    /// The process is still running.
    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

fn offered_tools(tools_list: &Value) -> Vec<String> {
    tools_list
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// Output parsing (pure)
// ---------------------------------------------------------------------------------------------

/// A symbol as `find_symbol` reports it. Serena's lines are 0-based.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundSymbol {
    pub name_path: String,
    pub kind: String,
    pub file: String,
    /// 1-based, inclusive; includes the symbol's doc comment and attributes.
    pub start_line: u32,
    pub end_line: u32,
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn lines_of(location: &Value) -> Option<(u32, u32)> {
    let start = location.get("start_line")?.as_u64()? as u32;
    let end = location.get("end_line")?.as_u64()? as u32;
    Some((start + 1, end + 1))
}

pub fn parse_find_symbol(text: &str) -> Vec<FoundSymbol> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            let (start_line, end_line) = lines_of(item.get("body_location")?)?;
            Some(FoundSymbol {
                name_path: item.get("name_path")?.as_str()?.to_string(),
                kind: item
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                file: normalize_path(item.get("relative_path")?.as_str()?),
                start_line,
                end_line,
            })
        })
        .collect()
}

/// The 1-based line of the reference in a `content_around_reference` excerpt, marked `>1386:`.
fn reference_line(excerpt: &str) -> Option<u32> {
    excerpt.lines().find_map(|line| {
        let rest = line.trim_start().strip_prefix('>')?;
        rest.split(':').next()?.trim().parse().ok()
    })
}

/// Referencing symbols grouped as `{file: {Kind: [{name_path, body_location, content_around_reference}]}}`.
pub fn parse_references(text: &str) -> Vec<SymbolReference> {
    let Ok(Value::Object(files)) = serde_json::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (file, kinds) in &files {
        let Some(kinds) = kinds.as_object() else {
            continue;
        };
        for (kind, items) in kinds {
            let Some(items) = items.as_array() else {
                continue;
            };
            for item in items {
                let Some(name_path) = item.get("name_path").and_then(Value::as_str) else {
                    continue;
                };
                let (start_line, end_line) = item
                    .get("body_location")
                    .and_then(lines_of)
                    .unwrap_or((0, 0));
                let snippet = item
                    .get("content_around_reference")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.push(SymbolReference {
                    file: normalize_path(file),
                    name_path: name_path.to_string(),
                    kind: kind.clone(),
                    start_line,
                    end_line,
                    reference_line: reference_line(&snippet),
                    snippet,
                });
            }
        }
    }
    out.sort_by(|a, b| {
        (&a.file, a.reference_line.unwrap_or(a.start_line))
            .cmp(&(&b.file, b.reference_line.unwrap_or(b.start_line)))
    });
    out
}

/// The symbol whose range contains the index's start line for the node (the doc comment above a
/// symbol belongs to its range, so the node's line lies inside it).
pub fn pick_definition(candidates: &[FoundSymbol], line: u32) -> Option<&FoundSymbol> {
    candidates
        .iter()
        .filter(|c| c.start_line <= line && line <= c.end_line)
        .min_by_key(|c| c.end_line - c.start_line)
}

/// What to say when the answer cannot be had.
pub fn unavailable(node_id: &str, reason: impl Into<String>) -> SymbolReferences {
    SymbolReferences {
        node_id: node_id.to_string(),
        available: false,
        reason: Some(reason.into()),
        references: Vec::new(),
        total: 0,
        truncated: false,
        definition_start_line: None,
        definition_end_line: None,
    }
}

/// Assemble the answer from a definition and the parsed references.
pub fn answer(
    node_id: &str,
    definition: &FoundSymbol,
    mut references: Vec<SymbolReference>,
) -> SymbolReferences {
    let total = references.len() as u32;
    let truncated = references.len() > MAX_REFERENCES;
    references.truncate(MAX_REFERENCES);
    SymbolReferences {
        node_id: node_id.to_string(),
        available: true,
        reason: None,
        references,
        total,
        truncated,
        definition_start_line: Some(definition.start_line),
        definition_end_line: Some(definition.end_line),
    }
}

/// `<knowledge>/serena/<project>/mode.yml`.
pub fn mode_path(root: &Path, project_dir_name: &str) -> PathBuf {
    root.join("serena")
        .join(format!("{project_dir_name}.mode.yml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_config_is_current_only_when_it_names_every_language() {
        let config = "project_name: demo\nlanguages:\n- rust\n- typescript\nignored_paths: []\n";
        assert!(config_lists_language(config, "rust"));
        assert!(config_lists_language(config, "typescript"));
        assert!(!config_lists_language(config, "python"));
        // The older single-language key is read too.
        assert!(config_lists_language("language: python\n", "python"));
        assert!(!config_lists_language("language: python\n", "rust"));
    }

    // Captured from Serena 1.28 on this repository (see docs/KNOWLEDGE_BASE.md).
    const FIND_SYMBOL: &str = r#"[{"name_path": "impl WorkspaceManager/is_worktree_dirty", "kind": "Method", "relative_path": "crates\\symphony-workspaces\\src\\lib.rs", "body_location": {"start_line": 599, "end_line": 618}}]"#;
    const REFERENCES: &str = r#"{"crates\\symphony-workspaces\\src\\lib.rs": {"Method": [{"name_path": "impl WorkspaceManager/cleanup_workspace", "body_location": {"start_line": 1370, "end_line": 1418}, "content_around_reference": "...1385:\n  >1386:        if !force && self.is_worktree_dirty(&validated).await {\n...1387:            return Err(SymphonyError::GitWorktreeError(format!("}], "Function": [{"name_path": "tests/test_git_worktree_lifecycle", "body_location": {"start_line": 1615, "end_line": 1727}, "content_around_reference": "...1676:        std::fs::write(allocated.path.join(\"feature.txt\"), \"hello world\\n\").unwrap();\n  >1677:        assert!(mgr.is_worktree_dirty(&allocated.path).await);\n...1678:"}]}, "crates\\symphony-orchestrator\\src\\lib.rs": {"Method": [{"name_path": "impl SymphonyOrchestrator/list_worktrees", "body_location": {"start_line": 1891, "end_line": 1936}, "content_around_reference": "...1914:            let exists = path.is_dir();\n  >1915:            let dirty = exists && self.workspaces.is_worktree_dirty(&path).await;\n...1916:            let project = projects.get(&task.project_id);"}]}}"#;

    #[test]
    fn find_symbol_output_becomes_one_based_ranges_with_forward_slashes() {
        let found = parse_find_symbol(FIND_SYMBOL);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file, "crates/symphony-workspaces/src/lib.rs");
        assert_eq!((found[0].start_line, found[0].end_line), (600, 619));
        assert_eq!(
            found[0].name_path,
            "impl WorkspaceManager/is_worktree_dirty"
        );
        // Graphify reports the `fn` line (601); it lies inside the range that starts at the doc comment.
        assert!(pick_definition(&found, 601).is_some());
        assert!(pick_definition(&found, 700).is_none());
        assert!(parse_find_symbol("not json").is_empty());
    }

    #[test]
    fn references_are_flattened_with_the_line_of_the_use() {
        let refs = parse_references(REFERENCES);
        assert_eq!(refs.len(), 3);
        let first = &refs[0];
        assert_eq!(first.file, "crates/symphony-orchestrator/src/lib.rs");
        assert_eq!(first.name_path, "impl SymphonyOrchestrator/list_worktrees");
        assert_eq!(first.reference_line, Some(1915));
        assert_eq!(first.kind, "Method");
        let in_test = refs.iter().find(|r| r.kind == "Function").unwrap();
        assert_eq!(in_test.reference_line, Some(1677));
        assert_eq!((in_test.start_line, in_test.end_line), (1616, 1728));
        assert!(parse_references("[]").is_empty());
    }

    #[test]
    fn the_smallest_enclosing_symbol_is_the_definition() {
        let outer = FoundSymbol {
            name_path: "A".into(),
            kind: "Class".into(),
            file: "a.ts".into(),
            start_line: 1,
            end_line: 100,
        };
        let inner = FoundSymbol {
            name_path: "A/f".into(),
            kind: "Method".into(),
            file: "a.ts".into(),
            start_line: 10,
            end_line: 20,
        };
        let both = [outer, inner.clone()];
        assert_eq!(pick_definition(&both, 15), Some(&inner));
    }

    #[test]
    fn the_answer_is_capped_and_says_so() {
        let definition = FoundSymbol {
            name_path: "f".into(),
            kind: "Function".into(),
            file: "a.rs".into(),
            start_line: 1,
            end_line: 5,
        };
        let many: Vec<SymbolReference> = (0..150)
            .map(|i| SymbolReference {
                file: "a.rs".into(),
                name_path: format!("g{i}"),
                kind: "Function".into(),
                start_line: i,
                end_line: i,
                reference_line: Some(i),
                snippet: String::new(),
            })
            .collect();
        let result = answer("n", &definition, many);
        assert_eq!(
            (result.total, result.references.len(), result.truncated),
            (150, 100, true)
        );
        assert_eq!(result.definition_start_line, Some(1));
        assert!(!unavailable("n", "no").available);
    }

    #[test]
    fn the_mode_file_hides_every_editing_tool_and_calls_are_allowlisted() {
        let mode = mode_file_contents();
        for tool in [
            "create_text_file",
            "replace_symbol_body",
            "execute_shell_command",
            "rename_symbol",
            "write_memory",
            "activate_project",
            "read_file",
        ] {
            assert!(
                mode.contains(&format!("  - {tool}\n")),
                "{tool} must be excluded"
            );
        }
        for tool in ALLOWED_TOOLS {
            assert!(
                !EXCLUDED_TOOLS.contains(tool),
                "{tool} is both allowed and excluded"
            );
        }
        assert_eq!(serena_language("Rust"), Some("rust"));
        assert_eq!(serena_language("JavaScript"), Some("typescript"));
        assert_eq!(serena_language("YAML"), None);
    }
}
