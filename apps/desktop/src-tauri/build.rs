fn main() {
    // Declare the app's IPC commands so each one needs an explicit `allow-*` permission in
    // `capabilities/default.json` (least privilege: no command is reachable by default).
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_local_api_config",
            "open_task_workspace",
            "pick_repository_folder",
            "set_autostart",
            "exit_application",
            "set_native_locale",
            "open_data_directory",
            "open_log_directory",
            "check_for_updates",
            "open_releases_page",
        ]),
    ))
    .expect("failed to run tauri-build");
}
