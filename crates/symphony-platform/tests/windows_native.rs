//! Windows-only checks of the native security boundaries: the owner-only DACL on the runtime
//! discovery file and the credential vault, and HKCU autostart registration. Runs on the
//! `windows-latest` CI runners (`cargo test --workspace`).
#![cfg(windows)]

use std::ffi::{c_void, OsStr};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr::null_mut;

use symphony_platform::secure_fs::{self, windows::CurrentUser};
use symphony_platform::{RuntimeDiscovery, SecretStore};
use tempfile::tempdir;
use windows_sys::Win32::Foundation::LocalFree;
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
    SetNamedSecurityInfoW, SDDL_REVISION_1, SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    GetSecurityDescriptorDacl, ACL, DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    UNPROTECTED_DACL_SECURITY_INFORMATION,
};

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(Some(0)).collect()
}

/// The DACL of `path` in SDDL form, e.g. `D:P(A;;FA;;;S-1-5-21-...)(A;;FA;;;SY)`.
fn dacl_sddl(path: &Path) -> String {
    let path_w = wide(path.as_os_str());
    unsafe {
        let mut dacl: *mut ACL = null_mut();
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        let status = GetNamedSecurityInfoW(
            path_w.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            &mut dacl,
            null_mut(),
            &mut sd,
        );
        assert_eq!(status, 0, "GetNamedSecurityInfoW failed");
        let mut out: *mut u16 = null_mut();
        let ok = ConvertSecurityDescriptorToStringSecurityDescriptorW(
            sd,
            SDDL_REVISION_1,
            DACL_SECURITY_INFORMATION,
            &mut out,
            null_mut(),
        );
        assert_ne!(ok, 0);
        let mut len = 0;
        while *out.add(len) != 0 {
            len += 1;
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, len));
        LocalFree(out as *mut c_void);
        LocalFree(sd);
        s
    }
}

/// Give `dir` an inheritable ACL that lets Everyone and Authenticated Users do anything — a
/// worst-case shared `--data-dir`.
fn make_directory_world_writable(dir: &Path) {
    let user = CurrentUser::query().unwrap().sid_string().unwrap();
    let sddl = wide(OsStr::new(&format!(
        "D:(A;OICI;FA;;;WD)(A;OICI;FA;;;AU)(A;OICI;FA;;;{user})"
    )));
    let path_w = wide(dir.as_os_str());
    unsafe {
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        assert_ne!(
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut()
            ),
            0
        );
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl: *mut ACL = null_mut();
        assert_ne!(
            GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted),
            0
        );
        let status = SetNamedSecurityInfoW(
            path_w.as_ptr() as *mut u16,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | UNPROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            dacl,
            null_mut(),
        );
        assert_eq!(status, 0, "SetNamedSecurityInfoW failed");
        LocalFree(sd);
    }
    assert!(
        dacl_sddl(dir).contains(";;;WD)"),
        "precondition: Everyone ACE on the directory"
    );
}

fn assert_owner_only(path: &Path) {
    let sddl = dacl_sddl(path);
    let user = CurrentUser::query().unwrap().sid_string().unwrap();
    // Wine (used for local cross-platform runs) does not persist the protected flag; real
    // Windows (CI) must.
    let under_wine = std::env::var_os("SYMPHONY_TEST_UNDER_WINE").is_some();
    assert!(
        under_wine || sddl.starts_with("D:P"),
        "DACL must be protected: {sddl}"
    );
    // SDDL renders well-known accounts as aliases: the built-in Administrator (RID 500, the
    // GitHub-hosted runner account) appears as `LA` rather than its full SID.
    let user_present = sddl.contains(&user) || (user.ends_with("-500") && sddl.contains(";;;LA)"));
    assert!(user_present, "current user must have access: {sddl}");
    for broad in [";;;WD)", ";;;AU)", ";;;BU)", ";;;IU)", ";;;AN)"] {
        assert!(
            !sddl.contains(broad),
            "{broad} must not have access: {sddl}"
        );
    }
    secure_fs::verify_private_file(path).unwrap();
}

fn record(token: &str) -> RuntimeDiscovery {
    RuntimeDiscovery {
        base_url: "http://127.0.0.1:4780".into(),
        token: token.into(),
        pid: std::process::id(),
        api_version: 2,
        owner: "windows-native-test".into(),
    }
}

#[test]
fn discovery_file_gets_owner_only_dacl() {
    let dir = tempdir().unwrap();
    let path = record("t".repeat(64).as_str()).write(dir.path()).unwrap();
    assert_owner_only(&path);
}

#[test]
fn discovery_file_is_protected_inside_world_writable_directory() {
    let dir = tempdir().unwrap();
    make_directory_world_writable(dir.path());
    let path = record("secret-token-in-shared-dir")
        .write(dir.path())
        .unwrap();
    assert_owner_only(&path);
}

#[test]
fn existing_permissive_discovery_file_is_replaced_and_ignored_until_then() {
    let dir = tempdir().unwrap();
    make_directory_world_writable(dir.path());
    let path = RuntimeDiscovery::file_path(dir.path());
    // A file that inherits the directory's permissive ACL (e.g. from an older version, or
    // planted by another account) is not trusted.
    std::fs::write(&path, serde_json::to_vec(&record("planted-token")).unwrap()).unwrap();
    assert!(secure_fs::verify_private_file(&path).is_err());
    assert!(RuntimeDiscovery::read_live(dir.path()).is_none());

    record("fresh-token").write(dir.path()).unwrap();
    assert_owner_only(&path);
    let found = RuntimeDiscovery::read_live(dir.path()).unwrap();
    assert_eq!(found.token, "fresh-token");
}

#[test]
fn discovery_survives_reopen_and_rewrite() {
    let dir = tempdir().unwrap();
    record("first").write(dir.path()).unwrap();
    // A restarted runtime publishes a new token over the old file.
    record("second").write(dir.path()).unwrap();
    assert_eq!(
        RuntimeDiscovery::read_live(dir.path()).unwrap().token,
        "second"
    );
    RuntimeDiscovery::remove(dir.path());
    assert!(RuntimeDiscovery::read_live(dir.path()).is_none());
}

#[test]
fn credential_vault_is_dpapi_protected_and_owner_only_in_shared_directory() {
    let dir = tempdir().unwrap();
    make_directory_world_writable(dir.path());
    {
        let store = SecretStore::new(dir.path()).unwrap();
        assert_eq!(store.protection(), "windows-dpapi-user");
        store
            .set_secret("GITHUB_TOKEN", "ghp_windows_native")
            .unwrap();
    }
    let vault = SecretStore::vault_path(dir.path());
    assert_owner_only(&vault);
    assert!(!std::fs::read_to_string(&vault)
        .unwrap()
        .contains("ghp_windows_native"));
    let reopened = SecretStore::new(dir.path()).unwrap();
    assert_eq!(
        reopened.get_vault_secret("GITHUB_TOKEN").as_deref(),
        Some("ghp_windows_native")
    );
}

#[test]
fn autostart_registration_roundtrip_reports_actual_state() {
    use symphony_platform::win_autostart;
    // A test-specific value name so the real `SymphonyDesktopRuntime` entry is never touched.
    let name = format!("SymphonyDesktopRuntimeTest{}", std::process::id());
    let exe = Path::new(r"C:\Program Files\Symphony Desktop\symphony-desktop.exe");

    win_autostart::configure(&name, true, exe).unwrap();
    assert_eq!(
        win_autostart::read(&name).unwrap().as_deref(),
        Some(symphony_platform::autostart_command(exe).as_str())
    );
    win_autostart::configure(&name, false, exe).unwrap();
    assert_eq!(win_autostart::read(&name).unwrap(), None);
    // Removing an entry that does not exist is not an error.
    win_autostart::configure(&name, false, exe).unwrap();
}
