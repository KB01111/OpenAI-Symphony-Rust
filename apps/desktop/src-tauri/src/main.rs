#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Symphony Desktop native host.
//!
//! The host owns the native lifecycle (window, tray, notifications, folder picker, autostart) and
//! either embeds the Rust orchestration runtime or attaches to one already running for the same
//! data directory (e.g. `symphony-cli serve`). The webview talks to the runtime over the
//! authenticated local HTTP/SSE API; the endpoint and per-launch token are handed to it through
//! the `get_local_api_config` command. IPC commands are narrowly scoped: none of them executes
//! arbitrary commands or touches arbitrary paths.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use symphony_core::SymphonyError;
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::{
    autostart_command, configure_windows_autostart, default_app_data_dir,
    windows_autostart_command, RuntimeDiscovery, SecretStore, SingleInstanceGuard,
};
use symphony_server::{
    generate_access_token, start_http_server, RemoteAccessManager, ServerConfig, API_VERSION,
};
use symphony_storage::Storage;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_notification::NotificationExt;

const DEFAULT_LOCAL_PORT: u16 = 4780;
const MAIN_WINDOW: &str = "main";

enum RuntimeMode {
    /// This process owns the orchestrator and its scheduler.
    Embedded {
        orchestrator: Arc<SymphonyOrchestrator>,
        data_dir: PathBuf,
        _instance_guard: SingleInstanceGuard,
    },
    /// Another process (e.g. `symphony-cli serve`) owns the data directory; the UI attaches to it.
    Attached,
}

struct DesktopAppState {
    mode: RuntimeMode,
    base_url: String,
    token: String,
    /// Why `runtime.json` could not be published, if it could not. The desktop keeps working
    /// over IPC, but CLI tools cannot attach until the data directory is fixed.
    discovery_error: Option<String>,
}

#[derive(Serialize)]
struct LocalApiConfig {
    base_url: String,
    token: String,
    api_version: u32,
    /// `embedded` or `attached`.
    mode: &'static str,
    discovery_error: Option<String>,
}

#[tauri::command]
fn get_local_api_config(state: State<'_, DesktopAppState>) -> LocalApiConfig {
    LocalApiConfig {
        base_url: state.base_url.clone(),
        token: state.token.clone(),
        api_version: API_VERSION,
        mode: match state.mode {
            RuntimeMode::Embedded { .. } => "embedded",
            RuntimeMode::Attached => "attached",
        },
        discovery_error: state.discovery_error.clone(),
    }
}

/// Open a task's worktree in the platform file manager. Only paths that the orchestrator has
/// allocated for the task and that validate inside the project's workspace root are opened.
#[tauri::command]
async fn open_task_workspace(
    state: State<'_, DesktopAppState>,
    task_id: String,
) -> Result<String, String> {
    let RuntimeMode::Embedded { orchestrator, .. } = &state.mode else {
        return Err("Opening workspaces is only available when the runtime is embedded".into());
    };
    let path = orchestrator
        .task_workspace_path(&task_id)
        .await
        .map_err(|e| e.to_string())?;
    reveal_in_file_manager(&path)?;
    Ok(path.to_string_lossy().to_string())
}

fn reveal_in_file_manager(path: &Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("Workspace directory no longer exists".into());
    }
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    std::process::Command::new(program)
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("Failed to open file manager: {e}"))
}

/// Native folder picker for repository registration. Returns `None` when cancelled.
#[tauri::command]
async fn pick_repository_folder(app: AppHandle) -> Option<String> {
    app.dialog()
        .file()
        .set_title("Select Git repository")
        .blocking_pick_folder()
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().to_string())
}

/// Register or remove the per-user login autostart entry (Windows `HKCU\...\Run`).
/// Returns whether autostart is supported on this platform. A registry write that Windows
/// rejects (or that does not persist) is returned as an error, so the UI never reports success
/// for an entry that does not exist.
#[tauri::command]
///
/// On failure the error starts with `autostart_state=<enabled|disabled|unknown>;`: the state the
/// registry was observed in afterwards, so the UI records reality instead of guessing.
fn set_autostart(enabled: bool) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    match configure_windows_autostart(enabled, &exe) {
        Ok(()) => Ok(cfg!(windows)),
        Err(err) => {
            // Only an entry that launches this executable counts as enabled; a stale entry for
            // another path is reported as unknown so the UI does not claim working autostart.
            let observed = match windows_autostart_command() {
                Ok(Some(command)) if command == autostart_command(&exe) => "enabled",
                Ok(Some(_)) => "unknown",
                Ok(None) => "disabled",
                Err(_) => "unknown",
            };
            Err(format!("autostart_state={observed}; {err}"))
        }
    }
}

/// Stop the runtime and exit, bypassing close-to-tray.
#[tauri::command]
fn exit_application(app: AppHandle) {
    app.exit(0);
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn main_window_focused(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN_WINDOW)
        .and_then(|w| w.is_focused().ok().zip(w.is_visible().ok()))
        .map(|(focused, visible)| focused && visible)
        .unwrap_or(false)
}

fn setup_tray(app: &AppHandle, embedded: bool) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Open Symphony Desktop", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(
        app,
        "pause_scheduler",
        "Pause task dispatching",
        embedded,
        None::<&str>,
    )?;
    let resume_item = MenuItem::with_id(
        app,
        "resume_scheduler",
        "Resume task dispatching",
        embedded,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, "quit", "Stop runtime and exit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_item, &pause_item, &resume_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::with_id("symphony-tray")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Symphony Desktop")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_main_window(app),
            id @ ("pause_scheduler" | "resume_scheduler") => {
                let state = app.state::<DesktopAppState>();
                if let RuntimeMode::Embedded { orchestrator, .. } = &state.mode {
                    let orch = orchestrator.clone();
                    let paused = id == "pause_scheduler";
                    tauri::async_runtime::spawn(async move {
                        let _ = orch.set_scheduler_paused(paused).await;
                    });
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }
    tray_builder.build(app)?;
    Ok(())
}

/// Ask the user whether to move an unreadable credential vault aside. Returns `true` to move it
/// aside and continue with an empty vault, `false` to quit. Never decides silently.
#[cfg(windows)]
fn confirm_vault_quarantine(message: &str, locale: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONERROR, MB_YESNO,
    };
    let wide = |s: &str| -> Vec<u16> { OsStr::new(s).encode_wide().chain(Some(0)).collect() };
    let (text, title) = vault_quarantine_prompt(message, locale);
    let (text, title) = (wide(&text), wide(title));
    // SAFETY: both strings are NUL-terminated UTF-16 and outlive the call.
    let choice = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            text.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONERROR | MB_DEFBUTTON2,
        )
    };
    choice == IDYES
}

/// Text and title of the vault quarantine prompt in the user's language (`en` | `sv`).
#[cfg_attr(not(windows), allow(dead_code))]
fn vault_quarantine_prompt(message: &str, locale: &str) -> (String, &'static str) {
    if locale == "sv" {
        (
            format!(
                "Symphony kunde inte öppna sitt lösenordsvalv:\n\n{message}\n\nFlytta undan det \
                 oläsbara valvet och fortsätt med ett tomt valv? Den gamla filen sparas bredvid som \
                 credentials.vault.corrupt-<tid>, och du behöver ange dina API-nycklar igen.\n\n\
                 Välj Nej för att avsluta utan att ändra något."
            ),
            "Symphony Desktop: lösenordsvalv",
        )
    } else {
        (
            format!(
                "Symphony could not open its credential vault:\n\n{message}\n\nMove the unreadable \
                 vault aside and continue with an empty vault? The old file is kept next to it as \
                 credentials.vault.corrupt-<time>, and you will need to re-enter your API keys.\n\n\
                 Choose No to quit without changing anything."
            ),
            "Symphony Desktop: credential vault",
        )
    }
}

#[cfg(not(windows))]
fn confirm_vault_quarantine(_message: &str, _locale: &str) -> bool {
    // Development hosts: no native prompt; use `symphony-cli --quarantine-credential-vault`.
    false
}

/// Open the credential vault. An unreadable vault is never replaced silently: the user either
/// moves it aside explicitly or the app stops with the error.
fn open_secret_store(data_dir: &Path, locale: &str) -> Result<SecretStore, String> {
    match SecretStore::new(data_dir) {
        Ok(store) => Ok(store),
        Err(SymphonyError::CredentialVault(message)) => {
            tracing::error!(error = %message, "Credential vault could not be opened");
            if !confirm_vault_quarantine(&message, locale) {
                return Err(format!("Credential vault could not be opened: {message}"));
            }
            SecretStore::quarantine_vault(data_dir).map_err(|e| e.to_string())?;
            SecretStore::new(data_dir).map_err(|e| e.to_string())
        }
        Err(other) => Err(other.to_string()),
    }
}

/// Native notification for events an operator should act on while the window is not focused.
fn notify_for_event(app: &AppHandle, event: &symphony_core::domain::AgentEvent) {
    let title = match event.event_type.as_str() {
        "approval/requested" => "Approval required",
        "attempt/succeeded" => "Ready for review",
        "attempt/failed" => "Agent run failed",
        _ => return,
    };
    if main_window_focused(app) {
        return;
    }
    let body = match &event.issue_identifier {
        Some(id) => format!("{id}: {}", event.message),
        None => event.message.clone(),
    };
    let _ = app
        .notification()
        .builder()
        .title(title)
        .body(body.chars().take(240).collect::<String>())
        .show();
}

async fn start_embedded_runtime(
    app: &AppHandle,
    data_dir: PathBuf,
    guard: SingleInstanceGuard,
) -> Result<DesktopAppState, String> {
    let storage = Storage::connect(&data_dir.join("symphony.sqlite"))
        .await
        .map_err(|e| format!("Failed to initialize SQLite storage: {e}"))?;
    let mut settings = storage.get_settings().await.unwrap_or_default();
    let secret_store = open_secret_store(&data_dir, &settings.locale)?;
    // Settings saved before 2.0 carry the old default remote port, which is the local API's port.
    // Migrate that legacy value to the new default and persist it, so the listener, `GET /settings`
    // and the settings form all agree on the port actually used.
    if settings.remote_port == DEFAULT_LOCAL_PORT {
        settings.remote_port = symphony_core::AppSettings::default().remote_port;
        if let Err(err) = storage.save_settings(&settings).await {
            tracing::warn!(error = %err, "Could not persist the migrated remote API port");
        }
    }

    let wf_path = [PathBuf::from("WORKFLOW.md"), data_dir.join("WORKFLOW.md")]
        .into_iter()
        .find(|p| p.is_file());
    let orchestrator = Arc::new(
        SymphonyOrchestrator::new(storage, wf_path, secret_store.clone())
            .await
            .map_err(|e| format!("Failed to initialize orchestrator: {e}"))?,
    );
    orchestrator
        .set_workflow_fallback_path(data_dir.join("WORKFLOW.md"))
        .await;
    orchestrator.start_background_loops();

    let token = generate_access_token();
    let remote_access =
        RemoteAccessManager::new(orchestrator.clone(), Some(data_dir.join("workspaces")));
    let local_config = ServerConfig {
        default_workspace_root: Some(data_dir.join("workspaces")),
        remote_access: Some(remote_access.clone()),
        ..ServerConfig::local(DEFAULT_LOCAL_PORT, Some(token.clone()))
    };
    // Preferred port first; an ephemeral fallback is discovered by clients via IPC/runtime.json.
    let (addr, _server) = match start_http_server(orchestrator.clone(), local_config.clone()).await
    {
        Ok(res) => res,
        Err(_) => start_http_server(
            orchestrator.clone(),
            ServerConfig {
                port: 0,
                ..local_config
            },
        )
        .await
        .map_err(|e| format!("Failed to bind local API: {e}"))?,
    };
    let base_url = format!("http://127.0.0.1:{}", addr.port());
    // Publish the endpoint for CLI tools and a second desktop instance. Failure is surfaced (log,
    // notification, `discovery_error` in the IPC config) but never worked around insecurely:
    // without an owner-only file, nothing is published.
    let discovery_error = match (RuntimeDiscovery {
        base_url: base_url.clone(),
        token: token.clone(),
        pid: std::process::id(),
        api_version: API_VERSION,
        owner: "symphony-desktop".into(),
    })
    .write(&data_dir)
    {
        Ok(_) => None,
        Err(err) => {
            RuntimeDiscovery::remove(&data_dir);
            let message = err.to_string();
            tracing::error!(error = %message, "Runtime discovery file was not published");
            let _ = app
                .notification()
                .builder()
                .title("Symphony runtime discovery unavailable")
                .body("CLI tools cannot attach to this runtime. Open Settings > Application for details.")
                .show();
            Some(message)
        }
    };

    // Optional remote companion listener: disabled by default, reconfigured live whenever the
    // settings or the remote access token change. Its effective state is `GET /remote-access`.
    remote_access.set_local_port(addr.port());
    let remote = remote_access.apply(&settings).await;
    if remote.enabled {
        tracing::info!(state = ?remote.state, error = ?remote.error, "Remote companion access");
    }

    // Forward orchestrator events to the webview and raise native notifications.
    let mut rx = orchestrator.subscribe_events();
    let emitter = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let _ = emitter.emit("symphony://event", &event);
                    notify_for_event(&emitter, &event);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    let _ = emitter.emit("symphony://resync", "lagged");
                }
                Err(_) => break,
            }
        }
    });

    Ok(DesktopAppState {
        mode: RuntimeMode::Embedded {
            orchestrator,
            data_dir,
            _instance_guard: guard,
        },
        base_url,
        token,
        discovery_error,
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,symphony=debug".into()),
        )
        .init();

    let start_hidden = std::env::args().any(|a| a == "--background");

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }))
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .setup(move |app| {
            let handle = app.handle().clone();
            // Same override as `symphony-cli --data-dir` / `SYMPHONY_DATA_DIR`, so the desktop and
            // the CLI agree on the data directory (and isolated native smoke tests are possible).
            let data_dir = std::env::var_os("SYMPHONY_DATA_DIR")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(default_app_data_dir);
            std::fs::create_dir_all(&data_dir)?;

            let state = tauri::async_runtime::block_on(async {
                match SingleInstanceGuard::acquire(&data_dir) {
                    Ok(guard) => start_embedded_runtime(&handle, data_dir.clone(), guard).await,
                    Err(_) => match RuntimeDiscovery::read_live(&data_dir) {
                        Some(found) => {
                            tracing::info!(base_url = %found.base_url, owner = %found.owner, "Attaching to existing Symphony runtime");
                            Ok(DesktopAppState {
                                mode: RuntimeMode::Attached,
                                base_url: found.base_url,
                                token: found.token,
                                discovery_error: None,
                            })
                        }
                        None => Err("Another Symphony runtime holds the data directory lock but did not publish its endpoint".to_string()),
                    },
                }
            })
            .map_err(Box::<dyn std::error::Error>::from)?;

            let embedded = matches!(state.mode, RuntimeMode::Embedded { .. });
            app.manage(state);
            setup_tray(app.handle(), embedded)?;

            if start_hidden {
                if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
                    let _ = window.hide();
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let minimize_to_tray = tauri::async_runtime::block_on(async move {
                    match app.try_state::<DesktopAppState>().map(|s| s.inner().mode_settings()) {
                        Some(Some(orch)) => orch
                            .storage()
                            .get_settings()
                            .await
                            .map(|s| s.minimize_to_tray_on_close)
                            .unwrap_or(true),
                        // Attached mode: closing the window never stops the external runtime.
                        _ => false,
                    }
                });
                if minimize_to_tray {
                    // Keep the orchestration runtime running in the background.
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_local_api_config,
            open_task_workspace,
            pick_repository_folder,
            set_autostart,
            exit_application
        ])
        .build(tauri::generate_context!())
        .expect("Error while building Symphony Desktop")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<DesktopAppState>() {
                    if let RuntimeMode::Embedded { data_dir, .. } = &state.mode {
                        RuntimeDiscovery::remove(data_dir);
                    }
                }
            }
        });
}

impl DesktopAppState {
    fn mode_settings(&self) -> Option<Arc<SymphonyOrchestrator>> {
        match &self.mode {
            RuntimeMode::Embedded { orchestrator, .. } => Some(orchestrator.clone()),
            RuntimeMode::Attached => None,
        }
    }
}
