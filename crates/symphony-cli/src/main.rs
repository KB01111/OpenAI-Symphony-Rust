use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use symphony_core::{
    domain::{Project, ProviderKind},
    workflow::{ServiceConfig, WorkflowDefinition},
};
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::{default_app_data_dir, RuntimeDiscovery, SecretStore, SingleInstanceGuard};
use symphony_server::{
    default_local_origins, generate_access_token, start_http_server, RemoteAccessManager,
    ServerConfig, API_VERSION,
};
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

    /// Data directory for the database, credential vault, runtime lock and `runtime.json`
    /// discovery file (defaults to the per-user local data directory)
    #[arg(long, env = "SYMPHONY_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// API access token. A random per-launch token is generated when omitted; clients discover it
    /// through `<data_dir>/runtime.json`. The token is never printed.
    #[arg(long, env = "SYMPHONY_API_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// Additional browser origins allowed by CORS (the desktop webview and Vite dev server are
    /// always allowed)
    #[arg(long = "allow-origin")]
    allow_origins: Vec<String>,

    /// Move an unreadable credential vault aside (`credentials.vault.corrupt-<timestamp>`) and
    /// start with an empty one. Without this flag an unreadable vault stops the runtime, so no
    /// credential is ever discarded silently.
    #[arg(long)]
    quarantine_credential_vault: bool,

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
        println!(
            "{}",
            serde_json::to_string_pretty(&cfg.redacted_for_display())?
        );
        return Ok(());
    }

    let data_dir = cli.data_dir.clone().unwrap_or_else(default_app_data_dir);
    std::fs::create_dir_all(&data_dir)?;

    // One runtime (desktop or CLI) owns a data directory at a time: single authoritative scheduler.
    let _instance_guard = SingleInstanceGuard::acquire(&data_dir)?;

    let db_path = cli
        .db_path
        .unwrap_or_else(|| data_dir.join("symphony.sqlite"));
    let storage = Storage::connect(&db_path).await?;
    // `--quarantine-credential-vault` only acts on a vault that cannot be opened; a readable
    // vault is used as is, so leaving the flag in a service definition is harmless.
    let secret_store = match SecretStore::new(&data_dir) {
        Err(symphony_core::SymphonyError::CredentialVault(message))
            if cli.quarantine_credential_vault =>
        {
            tracing::error!(error = %message, "Credential vault unreadable; moving it aside as requested");
            if let Some(moved) = SecretStore::quarantine_vault(&data_dir)? {
                eprintln!("Moved the credential vault aside to {}", moved.display());
            }
            SecretStore::new(&data_dir)?
        }
        other => other?,
    };

    // Precedence: --workflow, ./WORKFLOW.md, then a workflow previously saved to the data dir.
    let data_dir_workflow = data_dir.join("WORKFLOW.md");
    let wf_path = cli.workflow_path.or_else(|| {
        [PathBuf::from("WORKFLOW.md"), data_dir_workflow.clone()]
            .into_iter()
            .find(|candidate| candidate.exists())
    });

    let orchestrator = Arc::new(SymphonyOrchestrator::new(storage, wf_path, secret_store).await?);
    orchestrator
        .set_workflow_fallback_path(data_dir_workflow)
        .await;
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
                && (ev.event_type == "attempt/succeeded"
                    || ev.event_type == "attempt/failed"
                    || ev.event_type == "task/cancelled")
            {
                println!("{}", serde_json::to_string_pretty(&ev)?);
                break;
            }
        }
        return Ok(());
    }

    let configured_port = orchestrator
        .get_workflow_document()
        .await
        .config
        .and_then(|c| c.server_port);
    let port = cli.port.or(configured_port).unwrap_or(4780);
    // A supplied token is used as-is; a too-short one is a configuration error rather than being
    // silently replaced (clients configured with it could never authenticate).
    let token = match cli.token.clone() {
        Some(t) if t.len() >= 16 => t,
        Some(_) => anyhow::bail!("--token / SYMPHONY_API_TOKEN must be at least 16 characters"),
        None => generate_access_token(),
    };
    let mut allowed_origins = default_local_origins();
    allowed_origins.extend(cli.allow_origins.iter().cloned());
    let is_loopback = cli.host == "127.0.0.1" || cli.host == "localhost" || cli.host == "::1";
    if !is_loopback {
        tracing::warn!(
            host = %cli.host,
            "Binding the API to a non-loopback address. Requests are token-authenticated but not \
             encrypted; terminate TLS in front of this server for remote access."
        );
    }
    let remote_access =
        RemoteAccessManager::new(orchestrator.clone(), Some(data_dir.join("workspaces")));
    let server_config = ServerConfig {
        bind_host: cli.host.clone(),
        allowed_origins,
        default_workspace_root: Some(data_dir.join("workspaces")),
        remote_access: Some(remote_access.clone()),
        ..ServerConfig::local(port, Some(token.clone()))
    };
    let (addr, _handle) = match start_http_server(orchestrator.clone(), server_config.clone()).await
    {
        Ok(res) => res,
        // Requested port busy (e.g. desktop already bound it): fall back to an ephemeral port.
        Err(_) if cli.port.is_none() => {
            start_http_server(
                orchestrator.clone(),
                ServerConfig {
                    port: 0,
                    ..server_config
                },
            )
            .await?
        }
        Err(e) => return Err(e.into()),
    };
    let display_host = if is_loopback {
        "127.0.0.1".to_string()
    } else {
        addr.ip().to_string()
    };
    let discovery = RuntimeDiscovery {
        base_url: format!("http://{}:{}", display_host, addr.port()),
        token,
        pid: std::process::id(),
        api_version: API_VERSION,
        owner: "symphony-cli".into(),
    };
    let discovery_path = discovery.write(&data_dir)?;
    tracing::info!(
        "Symphony CLI & Background Engine running at {} (discovery: {})",
        discovery.base_url,
        discovery_path.display()
    );

    // Optional remote companion listener (disabled by default; reconfigured live by the API).
    remote_access.set_local_port(addr.port());
    let mut settings = match orchestrator.storage().get_settings().await {
        Ok(settings) => settings,
        Err(err) => {
            tracing::warn!(error = %err, "Could not read saved settings; remote access stays off until they are saved again");
            Default::default()
        }
    };
    // Settings saved before 2.0 carry the old default remote port, which is the local API's
    // default port. Migrate it only when it actually collides with the port this runtime bound,
    // so a deliberately chosen 4780 (with the local API elsewhere) is left alone.
    if settings.remote_port == 4780 && addr.port() == 4780 {
        settings.remote_port = symphony_core::domain::AppSettings::default().remote_port;
        if let Err(err) = orchestrator.storage().save_settings(&settings).await {
            tracing::warn!(error = %err, "Could not persist the migrated remote API port");
        }
    }
    let remote = remote_access.apply(&settings).await;
    if remote.enabled {
        tracing::info!(state = ?remote.state, error = ?remote.error, "Remote companion access");
    }

    wait_for_shutdown_signal().await;
    tracing::info!("Shutting down Symphony Engine gracefully...");
    remote_access.shutdown().await;
    RuntimeDiscovery::remove(&data_dir);
    Ok(())
}

/// Resolve on Ctrl+C, or SIGTERM on Unix, so the runtime lock and discovery file are released.
async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
