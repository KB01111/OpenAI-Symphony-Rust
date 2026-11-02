// Tauri checks every declared resource while compiling the Rust host, including plain
// `cargo check --workspace` and tests that never run Tauri CLI's beforeBuildCommand.
// Prepare missing Windows resources here as well; NSIS builds normally have them already
// because the Tauri pre-build hook does the same work first.
#[cfg(target_os = "windows")]
fn ensure_codesandbox_resources() {
    use std::path::PathBuf;
    use std::process::Command;

    let tauri_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let resource_dir = tauri_dir.join("resources").join("codesandbox-runner");
    let bridge = resource_dir.join("bridge.mjs");
    let node = resource_dir.join("node.exe");
    if bridge.is_file() && node.is_file() {
        return;
    }

    let root = tauri_dir.join("../../..");
    let runner_build = root.join("tools/codesandbox-runner/scripts/build.mjs");
    let prepare = root.join("scripts/prepare-codesandbox-windows.mjs");
    println!("cargo:rerun-if-changed={}", runner_build.display());
    println!("cargo:rerun-if-changed={}", prepare.display());
    for script in [runner_build, prepare] {
        let status = Command::new("node")
            .arg(&script)
            .current_dir(&root)
            .status()
            .unwrap_or_else(|err| {
                panic!(
                    "CodeSandbox packaging requires Node.js >=22.13.0 and installed pnpm workspace dependencies: {err}"
                )
            });
        assert!(
            status.success(),
            "Could not prepare CodeSandbox Windows resources using {}. Run `pnpm install --frozen-lockfile` and retry.",
            script.display()
        );
    }
    assert!(
        bridge.is_file() && node.is_file(),
        "CodeSandbox Windows resources are still missing after preparation"
    );
}

fn main() {
    #[cfg(target_os = "windows")]
    ensure_codesandbox_resources();
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
            "install_update",
            "open_releases_page",
        ]),
    ))
    .expect("failed to run tauri-build");
}
