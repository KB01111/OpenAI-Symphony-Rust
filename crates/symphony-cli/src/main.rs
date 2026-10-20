use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use symphony_core::{
    domain::{Project, ProviderKind},
    workflow::{ServiceConfig, WorkflowDefinition},
};
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::{SecretStore, SingleInstanceGuard};
use symphony_server::start_http_server;
use symphony_storage::Storage;

#[derive(Parser, Debug)]
#[command(
    name = "symphony-cli",
    version,
    about = "Symphony Desktop — Native Rust AI Coding Orchestration Runtime & CLI"
)]
struct Cli {
    /// Optional path to WORKFLOW.md (defaults to ./WORKFLOW.md if present)
    #[arg(value_name = "WORKFLOW_PATH")]
    workflow_path: Option<PathBuf>,

    /// Port for the HTTP orchestration API (defaults to 4780 or server.port in WORKFLOW.md)
    #[arg(long, short = 'p')]
    port: Option<u16>,

    /// Bind host (defaults to 127.0.0.1 for local security)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Optional SQLite database file path
    #[arg(long)]
    db_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Validate a WORKFLOW.md file and print the parsed configuration
    Validate {
        #[arg(default_value = "WORKFLOW.md")]
        path: PathBuf,
    },
    /// Run a single task immediately in headless CLI mode
    RunTask {
        #[arg(long)]
        identifier: String,
        #[arg(long)]
        title: String,
        #[arg(long)]
        description: Option<String>,
        #[arg(long, default_value = "codex")]
        provider: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,symphony=debug".into()),
        )
        .init();

    let cli = Cli::parse();

    if let Some(Commands::Validate { path }) = cli.command {
        let def = WorkflowDefinition::load_from_path(&path)?;
        let cfg = ServiceConfig::from_workflow(&def)?;
        cfg.validate_for_dispatch()?;
        println!("{}", serde_json::to_string_pretty(&cfg)?);
        return Ok(());
    }

    let data_dir = dirs::data_local_dir()
        .map(|d| d.join("symphony-desktop"))
        .unwrap_or_else(|| std::env::temp_dir().join("symphony-desktop"));
    std::fs::create_dir_all(&data_dir)?;

    let lock_path = data_dir.join("symphony-cli.lock");
    let _instance_guard = SingleInstanceGuard::acquire(&lock_path)?;

    let db_path = cli
        .db_path
        .unwrap_or_else(|| data_dir.join("symphony.sqlite"));
    let storage = Storage::connect(&db_path).await?;
    let secret_store = SecretStore::new(&data_dir)?;

    let wf_path = cli.workflow_path.or_else(|| {
        let candidate = PathBuf::from("WORKFLOW.md");
        if candidate.exists() {
            Some(candidate)
        } else {
            None
        }
    });

    let orchestrator = Arc::new(SymphonyOrchestrator::new(storage, wf_path, secret_store).await?);
    orchestrator.start_background_loops();

    if let Some(Commands::RunTask {
        identifier,
        title,
        description,
        provider,
    }) = cli.command
    {
        use chrono::Utc;

        let now = Utc::now();
        let pref_provider = match provider.to_lowercase().as_str() {
            "claude" | "claude_code" => ProviderKind::ClaudeCode,
            "opencode" => ProviderKind::OpenCode,
            "simulated" => ProviderKind::Simulated,
            _ => ProviderKind::Codex,
        };

        let projects = orchestrator.list_projects().await?;
        let project = if let Some(first) = projects.first() {
            first.clone()
        } else {
            let repo_dir = std::env::current_dir()?.to_string_lossy().to_string();
            let ws_root = std::env::temp_dir()
                .join("symphony_workspaces")
                .to_string_lossy()
                .to_string();
            orchestrator
                .create_project(Project {
                    id: uuid::Uuid::new_v4().to_string(),
                    name: "CLI Default Project".into(),
                    repo_path: repo_dir,
                    github_repo: None,
                    linear_project_slug: None,
                    default_branch: "main".into(),
                    workspace_root: ws_root,
                    preferred_provider: pref_provider,
                    preferred_model: None,
                    workflow_path: None,
                    tracker_kind: "manual".into(),
                    required_labels: vec!["symphony".into()],
                    auto_dispatch: false,
                    max_concurrent_agents: 2,
                    validation_commands: Vec::new(),
                    created_at: now,
                    updated_at: now,
                })
                .await?
        };

        let mut rx = orchestrator.subscribe_events();
        let task = orchestrator
            .create_manual_task(
                project.id,
                Some(identifier),
                title,
                description,
                Some(1),
                Some(pref_provider),
                None,
                vec!["cli".into()],
                true,
            )
            .await?;

        while let Ok(ev) = rx.recv().await {
            if ev.task_id.as_deref() == Some(&task.id)
                && (ev.event_type == "task/completed"
                    || ev.event_type == "task/human_review_ready"
                    || ev.event_type == "task/failed")
            {
                println!("{}", serde_json::to_string_pretty(&ev)?);
                break;
            }
        }
        return Ok(());
    }

    let (_, cfg, _) = orchestrator.get_workflow_document(None).await?;
    let port = cli.port.or(cfg.server_port).unwrap_or(4780);
    let (addr, _handle) = start_http_server(orchestrator.clone(), &cli.host, port, None).await?;
    tracing::info!(
        "Symphony CLI & Background Engine running at http://{}",
        addr
    );

    tokio::signal::ctrl_c().await?;
    tracing::info!("Shutting down Symphony Engine gracefully...");
    Ok(())
}
