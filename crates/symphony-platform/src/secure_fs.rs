//! Owner-only files for secrets (credential vault, runtime discovery token).
//!
//! - Unix: files are created with mode `0600` (`O_EXCL`), and verified to be owned by the
//!   effective user with no group/other permission bits.
//! - Windows: files are created with an explicit, *protected* DACL (inheritance from the parent
//!   directory is blocked) that grants access only to the current user and `SYSTEM`. The
//!   descriptor is applied by `CreateFileW` itself, so the file is never readable by anyone else,
//!   not even for the moment between creation and a later ACL change. Verification rejects a
//!   missing (NULL) DACL and any access-allowed entry, inherited or explicit, for another
//!   principal.
//!
//! [`write_private_atomic`] writes a fresh private temp file next to the target, flushes it to
//! disk, verifies its permissions and atomically renames it over the target. A failure at any
//! step leaves the previous file untouched.

use std::fs::File;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Create `path` exclusively (it must not exist) with owner-only permissions.
pub fn create_private_file(path: &Path) -> io::Result<File> {
    imp::create_private_file(path)
}

/// Fail unless `path` is a regular file owned by the current user that no other principal
/// (other than `SYSTEM`/Administrators on Windows) can access.
pub fn verify_private_file(path: &Path) -> io::Result<()> {
    imp::verify_private_file(path)
}

/// Read a file only if it is private to the current user. The checks and the read use the same
/// open handle (symlinks / reparse points are refused), so the file cannot be swapped between
/// the check and the read.
pub fn read_private_file(path: &Path) -> io::Result<Vec<u8>> {
    imp::read_private_file(path)
}

/// Open `path` for appending, creating it owner-only if it does not exist. An existing file is
/// checked through the opened handle itself (symlinks / reparse points are refused), so it cannot
/// be swapped between the check and the writes; one that others can access is rejected.
pub fn open_private_append(path: &Path) -> io::Result<File> {
    imp::open_private_append(path)
}

/// Create `dir` (and parents) if missing. A directory created here is owner-only on Unix; an
/// existing directory is left as is, since the files inside it are protected individually.
pub fn ensure_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Atomically replace `path` with `contents`, readable only by the current user.
pub fn write_private_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let tmp = temp_path_for(path)?;
    let result = (|| {
        {
            let mut file = create_private_file(&tmp)?;
            file.write_all(contents)?;
            file.sync_all()?;
        }
        verify_private_file(&tmp)?;
        rename_replace(&tmp, path)?;
        sync_parent_dir(path);
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn temp_path_for(path: &Path) -> io::Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?
        .to_string_lossy()
        .to_string();
    let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp = path.with_file_name(format!(".{name}.{}.{unique}.tmp", std::process::id()));
    // Process-unique name; a leftover from a crashed process with a recycled PID is removed so
    // `create_new` never reuses a file with someone else's permissions.
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    Ok(tmp)
}

fn rename_replace(from: &Path, to: &Path) -> io::Result<()> {
    // `std::fs::rename` replaces an existing target on every platform (MoveFileExW with
    // MOVEFILE_REPLACE_EXISTING on Windows). Antivirus scanners can hold a Windows file open
    // briefly, so retry a sharing violation a few times before giving up.
    let mut attempts = 0;
    loop {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) if cfg!(windows) && attempts < 5 && e.raw_os_error() == Some(32) => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            Err(e) => return Err(e),
        }
    }
}

fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if let Ok(dir) = File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn insecure(path: &Path, why: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{} is not private to the current user: {why}",
            path.display()
        ),
    )
}

#[cfg(unix)]
mod imp {
    use super::insecure;
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    use std::path::Path;

    pub fn create_private_file(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
    }

    pub fn verify_private_file(path: &Path) -> io::Result<()> {
        check_metadata(path, &std::fs::symlink_metadata(path)?)
    }

    pub fn open_private_append(path: &Path) -> io::Result<File> {
        match OpenOptions::new()
            .append(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
        {
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let file = OpenOptions::new()
                    .append(true)
                    // O_NONBLOCK: a FIFO planted at `path` must not block the open.
                    .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                    .open(path)?;
                check_metadata(path, &file.metadata()?)?;
                Ok(file)
            }
            other => other,
        }
    }

    pub fn read_private_file(path: &Path) -> io::Result<Vec<u8>> {
        use std::io::Read;
        let mut file = OpenOptions::new()
            .read(true)
            // O_NONBLOCK: opening a FIFO planted at `path` must not block; it is then rejected
            // as "not a regular file". The flag has no effect on reads of a regular file.
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)?;
        check_metadata(path, &file.metadata()?)?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        Ok(contents)
    }

    fn check_metadata(path: &Path, meta: &std::fs::Metadata) -> io::Result<()> {
        if !meta.file_type().is_file() {
            return Err(insecure(path, "not a regular file"));
        }
        // SAFETY: geteuid has no preconditions and cannot fail.
        let euid = unsafe { libc::geteuid() };
        if meta.uid() != euid {
            return Err(insecure(path, format!("owned by uid {}", meta.uid())));
        }
        if meta.mode() & 0o077 != 0 {
            return Err(insecure(
                path,
                format!("mode {:o} grants group/other access", meta.mode() & 0o777),
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
mod imp {
    use super::insecure;
    use std::ffi::{c_void, OsStr};
    use std::fs::File;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::path::Path;
    use std::ptr::null_mut;

    use windows_sys::Win32::Foundation::{
        CloseHandle, LocalFree, ERROR_INSUFFICIENT_BUFFER, GENERIC_READ, GENERIC_WRITE, HANDLE,
        INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        GetNamedSecurityInfoW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AclSizeInformation, EqualSid, GetAce, GetAclInformation, GetTokenInformation,
        IsWellKnownSid, TokenUser, WinBuiltinAdministratorsSid, WinLocalSystemSid,
        ACCESS_ALLOWED_ACE, ACE_HEADER, ACL, ACL_SIZE_INFORMATION, DACL_SECURITY_INFORMATION,
        OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, CREATE_NEW, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OPEN_REPARSE_POINT, READ_CONTROL,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    const ACCESS_ALLOWED_ACE_TYPE: u8 = 0;
    const ACCESS_DENIED_ACE_TYPE: u8 = 1;

    fn wide(s: &OsStr) -> Vec<u16> {
        s.encode_wide().chain(Some(0)).collect()
    }

    /// Frees memory returned by Win32 APIs that allocate with `LocalAlloc`.
    struct LocalBox(*mut c_void);
    impl Drop for LocalBox {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: the pointer was allocated by the API that produced it with LocalAlloc.
                unsafe { LocalFree(self.0) };
            }
        }
    }

    /// The SID of the user the process runs as (`TOKEN_USER`).
    pub struct CurrentUser {
        // u64 storage keeps the TOKEN_USER structure correctly aligned.
        buf: Vec<u64>,
    }

    impl CurrentUser {
        pub fn query() -> io::Result<Self> {
            // SAFETY: standard token query; every handle opened here is closed before returning.
            unsafe {
                let mut token: HANDLE = null_mut();
                if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let mut len = 0u32;
                GetTokenInformation(token, TokenUser, null_mut(), 0, &mut len);
                let err = io::Error::last_os_error();
                if err.raw_os_error() != Some(ERROR_INSUFFICIENT_BUFFER as i32) {
                    CloseHandle(token);
                    return Err(err);
                }
                let mut buf = vec![0u64; (len as usize).div_ceil(8)];
                let ok = GetTokenInformation(
                    token,
                    TokenUser,
                    buf.as_mut_ptr() as *mut c_void,
                    len,
                    &mut len,
                );
                CloseHandle(token);
                if ok == 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(Self { buf })
            }
        }

        pub fn sid(&self) -> PSID {
            // SAFETY: `buf` holds a TOKEN_USER filled in by GetTokenInformation.
            unsafe { (*(self.buf.as_ptr() as *const TOKEN_USER)).User.Sid }
        }

        pub fn sid_string(&self) -> io::Result<String> {
            let mut out: *mut u16 = null_mut();
            // SAFETY: `sid()` is a valid SID; the returned string is freed by LocalBox.
            unsafe {
                if ConvertSidToStringSidW(self.sid(), &mut out) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let _free = LocalBox(out as *mut c_void);
                let mut len = 0;
                while *out.add(len) != 0 {
                    len += 1;
                }
                Ok(String::from_utf16_lossy(std::slice::from_raw_parts(
                    out, len,
                )))
            }
        }
    }

    /// SDDL for a protected DACL granting full access to the current user and SYSTEM only.
    pub fn owner_only_sddl(user: &CurrentUser) -> io::Result<String> {
        Ok(format!("D:P(A;;FA;;;{})(A;;FA;;;SY)", user.sid_string()?))
    }

    pub fn create_private_file(path: &Path) -> io::Result<File> {
        create_with_owner_only_dacl(path, GENERIC_READ | GENERIC_WRITE, 0)
    }

    pub fn open_private_append(path: &Path) -> io::Result<File> {
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_APPEND_DATA, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
            FILE_SHARE_WRITE, SYNCHRONIZE,
        };
        // Appending only; other processes may append (a second instance) or prune (delete) too.
        let access = FILE_APPEND_DATA | FILE_READ_ATTRIBUTES | READ_CONTROL | SYNCHRONIZE;
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
        match create_with_owner_only_dacl(path, access, share) {
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                let file = std::fs::OpenOptions::new()
                    .access_mode(access)
                    .share_mode(share)
                    .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(path)?;
                let meta = file.metadata()?;
                if meta.file_type().is_symlink() || !meta.is_file() {
                    return Err(insecure(path, "not a regular file"));
                }
                let handle = file.as_raw_handle() as HANDLE;
                check_descriptor(path, |owner, dacl, sd| {
                    // SAFETY: `handle` is open with READ_CONTROL for the duration of the call.
                    unsafe {
                        GetSecurityInfo(
                            handle,
                            SE_FILE_OBJECT,
                            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                            owner,
                            null_mut(),
                            dacl,
                            null_mut(),
                            sd,
                        )
                    }
                })?;
                Ok(file)
            }
            other => other,
        }
    }

    /// Create `path` (it must not exist) with a protected owner-only DACL applied atomically by
    /// `CreateFileW`.
    fn create_with_owner_only_dacl(path: &Path, access: u32, share: u32) -> io::Result<File> {
        let user = CurrentUser::query()?;
        let sddl = wide(OsStr::new(&owner_only_sddl(&user)?));
        let path_w = wide(path.as_os_str());
        // SAFETY: the security descriptor outlives CreateFileW and is freed by LocalBox; the
        // returned handle is owned by the File.
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let _free = LocalBox(sd);
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            };
            let handle = CreateFileW(
                path_w.as_ptr(),
                access,
                share,
                &attributes,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL,
                null_mut(),
            );
            if handle == INVALID_HANDLE_VALUE {
                return Err(io::Error::last_os_error());
            }
            Ok(File::from_raw_handle(handle as _))
        }
    }

    pub fn verify_private_file(path: &Path) -> io::Result<()> {
        let meta = std::fs::symlink_metadata(path)?;
        if !meta.file_type().is_file() {
            return Err(insecure(path, "not a regular file"));
        }
        let path_w = wide(path.as_os_str());
        check_descriptor(path, |owner, dacl, sd| {
            // SAFETY: valid NUL-terminated path and out pointers.
            unsafe {
                GetNamedSecurityInfoW(
                    path_w.as_ptr(),
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    owner,
                    null_mut(),
                    dacl,
                    null_mut(),
                    sd,
                )
            }
        })
    }

    pub fn read_private_file(path: &Path) -> io::Result<Vec<u8>> {
        use std::io::Read;
        use std::os::windows::fs::OpenOptionsExt;
        use std::os::windows::io::AsRawHandle;
        // FILE_FLAG_OPEN_REPARSE_POINT opens a symlink/junction itself instead of its target, so
        // the metadata check below rejects it.
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .access_mode(GENERIC_READ | READ_CONTROL)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let meta = file.metadata()?;
        if meta.file_type().is_symlink() || !meta.is_file() {
            return Err(insecure(path, "not a regular file"));
        }
        let handle = file.as_raw_handle() as HANDLE;
        check_descriptor(path, |owner, dacl, sd| {
            // SAFETY: `handle` is open with READ_CONTROL for the duration of the call.
            unsafe {
                GetSecurityInfo(
                    handle,
                    SE_FILE_OBJECT,
                    OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                    owner,
                    null_mut(),
                    dacl,
                    null_mut(),
                    sd,
                )
            }
        })?;
        let mut contents = Vec::new();
        file.read_to_end(&mut contents)?;
        Ok(contents)
    }

    /// Validate the owner and DACL returned by `fetch` (GetNamedSecurityInfoW or
    /// GetSecurityInfo).
    fn check_descriptor(
        path: &Path,
        fetch: impl FnOnce(*mut PSID, *mut *mut ACL, *mut PSECURITY_DESCRIPTOR) -> u32,
    ) -> io::Result<()> {
        let user = CurrentUser::query()?;
        // SAFETY: every pointer below points into the security descriptor returned by `fetch`,
        // which stays alive (LocalBox) until the end of this block.
        unsafe {
            let mut owner: PSID = null_mut();
            let mut dacl: *mut ACL = null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = null_mut();
            let status = fetch(&mut owner, &mut dacl, &mut sd);
            if status != 0 {
                return Err(io::Error::from_raw_os_error(status as i32));
            }
            let _free = LocalBox(sd);

            if owner.is_null()
                || (EqualSid(owner, user.sid()) == 0
                    && IsWellKnownSid(owner, WinBuiltinAdministratorsSid) == 0)
            {
                return Err(insecure(path, "owned by another account"));
            }
            if dacl.is_null() {
                // A NULL DACL grants everyone full access (e.g. FAT volumes).
                return Err(insecure(
                    path,
                    "no access control list (everyone has access)",
                ));
            }
            // Every ACE is checked, inherited ones included, so a permissive parent directory
            // is caught whether or not the DACL is marked protected.
            let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            if GetAclInformation(
                dacl,
                &mut info as *mut _ as *mut c_void,
                std::mem::size_of::<ACL_SIZE_INFORMATION>() as u32,
                AclSizeInformation,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            for index in 0..info.AceCount {
                let mut ace: *mut c_void = null_mut();
                if GetAce(dacl, index, &mut ace) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let header = &*(ace as *const ACE_HEADER);
                match header.AceType {
                    ACCESS_DENIED_ACE_TYPE => continue,
                    ACCESS_ALLOWED_ACE_TYPE => {
                        let allowed = &*(ace as *const ACCESS_ALLOWED_ACE);
                        let sid = &allowed.SidStart as *const u32 as PSID;
                        let permitted = EqualSid(sid, user.sid()) != 0
                            || IsWellKnownSid(sid, WinLocalSystemSid) != 0
                            || IsWellKnownSid(sid, WinBuiltinAdministratorsSid) != 0;
                        if !permitted {
                            return Err(insecure(path, "grants access to another account"));
                        }
                    }
                    other => {
                        return Err(insecure(
                            path,
                            format!("unsupported access control entry type {other}"),
                        ))
                    }
                }
            }
        }
        Ok(())
    }
}

/// Windows-only helpers exposed for integration tests and diagnostics.
#[cfg(windows)]
pub mod windows {
    pub use super::imp::{owner_only_sddl, CurrentUser};
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_creates_private_file_and_replaces_existing() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("secret.json");
        write_private_atomic(&path, b"first").unwrap();
        verify_private_file(&path).unwrap();
        write_private_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        verify_private_file(&path).unwrap();
        // No temp files are left behind.
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty());
    }

    #[test]
    fn open_private_append_creates_private_and_appends() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("log");
        {
            use std::io::Write;
            open_private_append(&path)
                .unwrap()
                .write_all(b"one\n")
                .unwrap();
            open_private_append(&path)
                .unwrap()
                .write_all(b"two\n")
                .unwrap();
        }
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "one\ntwo\n");
        verify_private_file(&path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn open_private_append_refuses_shared_files_and_symlinks() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let shared = dir.path().join("shared");
        std::fs::write(&shared, b"x").unwrap();
        std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(open_private_append(&shared).is_err());
        let private = dir.path().join("private");
        std::fs::write(&private, b"x").unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&private, &link).unwrap();
        assert!(
            open_private_append(&link).is_err(),
            "symlinks are never followed"
        );
    }

    #[test]
    fn create_private_file_refuses_existing_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("exists");
        std::fs::write(&path, b"x").unwrap();
        assert!(create_private_file(&path).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn verify_rejects_group_or_world_readable_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        let path = dir.path().join("shared");
        std::fs::write(&path, b"x").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = verify_private_file(&path).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn read_private_file_reads_only_private_files() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("private");
        write_private_atomic(&path, b"secret").unwrap();
        assert_eq!(read_private_file(&path).unwrap(), b"secret");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let shared = dir.path().join("shared");
            std::fs::write(&shared, b"x").unwrap();
            std::fs::set_permissions(&shared, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(read_private_file(&shared).is_err());
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&path, &link).unwrap();
            assert!(
                read_private_file(&link).is_err(),
                "symlinks are never followed"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn verify_rejects_symlink() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("target");
        write_private_atomic(&target, b"x").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(verify_private_file(&link).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_a_fifo_without_blocking() {
        let dir = tempdir().unwrap();
        let fifo = dir.path().join("runtime.json");
        let c_path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
        // SAFETY: valid NUL-terminated path.
        assert_eq!(unsafe { libc::mkfifo(c_path.as_ptr(), 0o600) }, 0);
        // Without O_NONBLOCK the open would wait forever for a writer.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read_private_file(&fifo).map(|_| ()));
        });
        let result = rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("reading a FIFO must not block");
        assert!(result.is_err(), "a FIFO is not a regular file");
    }

    #[test]
    fn failed_replace_keeps_previous_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("target");
        // A non-empty directory at the target path makes the final rename fail.
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inner"), b"keep").unwrap();
        assert!(write_private_atomic(&path, b"new").is_err());
        assert_eq!(std::fs::read(path.join("inner")).unwrap(), b"keep");
        let leftovers = std::fs::read_dir(dir.path()).unwrap().count();
        assert_eq!(leftovers, 1, "temp file must be cleaned up");
    }
}
