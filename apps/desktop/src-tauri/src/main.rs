#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use symphony_core::domain::{OrchestratorStateSnapshot, Task};
use symphony_orchestrator::SymphonyOrchestrator;
use symphony_platform::{configure_windows_autostart, SecretStore, SingleInstanceGuard};
use symphony_server::start_http_server;
use symphony_storage::Storage;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, State, WindowEvent,
};

struct DesktopAppState {
    orchestrator: Arc<SymphonyOrchestrator>,
    api_addr: SocketAddr,
    _instance_guard: Option<SingleInstanceGuard>,
}

#[tauri::command]
async fn get_api_base_url(state: State<'_, DesktopAppState>) -> Result<String, String> {
    Ok(format!("http://{}", state.api_addr))
}

#[tauri::command]
async fn get_orchestrator_state(
    state: State<'_, DesktopAppState>,
) -> Result<OrchestratorStateSnapshot, String> {
    state
        .orchestrator
        .state_snapshot()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn list_tasks_ipc(
    state: State<'_, DesktopAppState>,
    project_id: Option<String>,
) -> Result<Vec<Task>, String> {
    state
        .orchestrator
        .list_tasks(project_id.as_deref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_autostart_ipc(enabled: bool) -> Result<bool, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    configure_windows_autostart(enabled, &exe).map_err(|e| e.to_string())?;
    Ok(enabled)
}

#[tauri::command]
async fn exit_application_completely(app: AppHandle) -> Result<(), String> {
    app.exit(0);
    Ok(())
}

fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(app, "show", "Open Symphony Desktop", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(
        app,
        "pause_scheduler",
        "Pause Task Dispatching",
        true,
        None::<&str>,
    )?;
    let resume_item = MenuItem::with_id(
        app,
        "resume_scheduler",
        "Resume Task Dispatching",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(
        app,
        "quit",
        "Stop Orchestrator & Exit Completely",
        true,
        None::<&str>,
    )?;

    let menu = Menu::with_items(app, &[&show_item, &pause_item, &resume_item, &quit_item])?;

    let mut tray_builder = TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Symphony Desktop — Autonomous Coding Orchestrator")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "pause_scheduler" => {
                let state = app.state::<DesktopAppState>();
                let orch = state.orchestrator.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = orch.set_scheduler_paused(true).await;
                });
            }
            "resume_scheduler" => {
                let state = app.state::<DesktopAppState>();
                let orch = state.orchestrator.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = orch.set_scheduler_paused(false).await;
                });
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
        });

    if let Some(icon) = app.default_window_icon() {
        tray_builder = tray_builder.icon(icon.clone());
    }

    let _ = tray_builder.build(app)?;
    Ok(())
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,symphony=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();

            tauri::async_runtime::block_on(async move {
                let data_dir = dirs::data_local_dir()
                    .map(|d| d.join("symphony-desktop"))
                    .unwrap_or_else(|| std::env::temp_dir().join("symphony-desktop"));
                let _ = std::fs::create_dir_all(&data_dir);

                let lock_path = data_dir.join("symphony-desktop.lock");
                let instance_guard = SingleInstanceGuard::acquire(&lock_path).ok();

                let db_path = data_dir.join("symphony.sqlite");
                let storage = Storage::connect(&db_path)
                    .await
                    .expect("Failed to initialize SQLite storage");
                let secret_store =
                    SecretStore::new(&data_dir).expect("Failed to initialize secret store");

                let wf_path = {
                    let candidate = PathBuf::from("WORKFLOW.md");
                    if candidate.exists() {
                        Some(candidate)
                    } else {
                        None
                    }
                };

                let orchestrator = Arc::new(
                    SymphonyOrchestrator::new(storage, wf_path, secret_store)
                        .await
                        .expect("Failed to initialize Symphony Orchestrator"),
                );
                orchestrator.start_background_loops();

                // Bind local API server on 4780 (or ephemeral fallback if 4780 is occupied by standalone CLI)
                let (api_addr, _server_handle) =
                    match start_http_server(orchestrator.clone(), "127.0.0.1", 4780, None).await {
                        Ok(res) => res,
                        Err(_) => start_http_server(orchestrator.clone(), "127.0.0.1", 0, None)
                            .await
                            .expect("Failed to bind fallback HTTP port"),
                    };

                // Forward orchestrator broadcast events to Tauri webview
                let mut rx = orchestrator.subscribe_events();
                let emitter_handle = app_handle.clone();
                tokio::spawn(async move {
                    while let Ok(notif) = rx.recv().await {
                        let _ = emitter_handle.emit("symphony://event", &notif);
                    }
                });

                app_handle.manage(DesktopAppState {
                    orchestrator,
                    api_addr,
                    _instance_guard: instance_guard,
                });
            });

            let _ = setup_tray(app.handle());
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let app = window.app_handle().clone();
                let window_clone = window.clone();
                let minimize_to_tray = tauri::async_runtime::block_on(async move {
                    if let Some(state) = app.try_state::<DesktopAppState>() {
                        if let Ok(settings) = state.orchestrator.storage().get_settings().await {
                            return settings.minimize_to_tray_on_close;
                        }
                    }
                    true
                });

                if minimize_to_tray {
                    // Keep the Rust orchestration runtime running in the background when window closes
                    api.prevent_close();
                    let _ = window_clone.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_api_base_url,
            get_orchestrator_state,
            list_tasks_ipc,
            set_autostart_ipc,
            exit_application_completely
        ])
        .run(tauri::generate_context!())
        .expect("Error while running Symphony Desktop Tauri application");
}
