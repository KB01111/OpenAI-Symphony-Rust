//! Credential vault (`<data_dir>/credentials.vault`).
//!
//! Format version 2 is a small JSON envelope around one encrypted blob that holds every
//! credential:
//!
//! ```json
//! { "format": "symphony-credential-vault", "version": 2,
//!   "protection": "windows-dpapi-user", "nonce": null, "ciphertext": "<base64>" }
//! ```
//!
//! The blob is produced by a [`VaultProtector`]:
//! - **Windows**: DPAPI (`CryptProtectData`) bound to the current Windows user, with the envelope
//!   header as additional entropy. DPAPI authenticates the data, so tampering is detected.
//! - **Other platforms**: ChaCha20-Poly1305 (AEAD, `ring`) with a random 256-bit key kept in the
//!   owner-only `<data_dir>/credentials.key` (mode `0600`, verified on every load). The envelope
//!   header is the associated data.
//!
//! Guarantees:
//! - Writes are atomic (private temp file, `fsync`, rename). The in-memory cache changes only
//!   after the new vault is safely on disk, so a failed write never desynchronises them.
//! - A vault that cannot be read, authenticated or decrypted is never replaced: [`SecretStore::new`]
//!   fails with [`SymphonyError::CredentialVault`] and leaves the file in place. The operator can
//!   move it aside explicitly with [`SecretStore::quarantine_vault`].
//! - Version-1 vaults (XOR + base64, written by Symphony Desktop ≤ 2.0.0) are migrated on first
//!   load: decoded, re-written in the current format, read back and compared before use. A
//!   legacy file that cannot be decoded is reported, never discarded.
//! - Secret values are never logged; removed values and plaintext buffers are zeroized.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use base64::Engine;
use serde::{Deserialize, Serialize};
use symphony_core::{SymphonyError, SymphonyResult};
use tracing::{debug, info, warn};
use zeroize::{Zeroize, Zeroizing};

use crate::secure_fs;

const VAULT_FILE: &str = "credentials.vault";
const VAULT_FORMAT: &str = "symphony-credential-vault";
const VAULT_VERSION: u32 = 2;

fn vault_error(message: impl Into<String>) -> SymphonyError {
    SymphonyError::CredentialVault(message.into())
}

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Additional authenticated data / DPAPI entropy binding a blob to its envelope header.
fn associated_data(protection: &str) -> Vec<u8> {
    format!("{VAULT_FORMAT}:v{VAULT_VERSION}:{protection}").into_bytes()
}

#[derive(Debug, Serialize, Deserialize)]
struct VaultEnvelope {
    format: String,
    version: u32,
    protection: String,
    nonce: Option<String>,
    ciphertext: String,
}

#[derive(Serialize, Deserialize, Default)]
struct VaultPayload {
    entries: BTreeMap<String, String>,
}

/// Borrowing twin of [`VaultPayload`] for serialization, so no unzeroized copy of the secrets
/// is made.
#[derive(Serialize)]
struct VaultPayloadRef<'a> {
    entries: &'a BTreeMap<String, String>,
}

/// Encrypted blob produced by a [`VaultProtector`].
pub struct SealedBlob {
    pub nonce: Option<Vec<u8>>,
    pub ciphertext: Vec<u8>,
}

/// OS-specific authenticated encryption for the vault contents.
pub trait VaultProtector: Send + Sync {
    /// Identifier stored in the envelope (`protection`).
    fn protection(&self) -> &'static str;
    fn seal(&self, plaintext: &[u8], aad: &[u8]) -> SymphonyResult<SealedBlob>;
    fn open(&self, blob: &SealedBlob, aad: &[u8]) -> SymphonyResult<Zeroizing<Vec<u8>>>;
}

/// The protector for this platform. `vault_exists` prevents a missing key file from being
/// silently regenerated (which would make an existing vault undecryptable).
fn platform_protector(
    app_data_dir: &Path,
    vault_exists: bool,
) -> SymphonyResult<Box<dyn VaultProtector>> {
    #[cfg(windows)]
    {
        let _ = (app_data_dir, vault_exists);
        Ok(Box::new(dpapi::DpapiProtector))
    }
    #[cfg(not(windows))]
    {
        Ok(Box::new(keyfile::KeyFileProtector::load_or_create(
            app_data_dir,
            vault_exists,
        )?))
    }
}

/// Secure secret store that persists credentials outside SQLite in the encrypted vault, with an
/// environment-variable fallback for reads. Raw secrets are never stored in SQLite.
#[derive(Clone)]
pub struct SecretStore {
    vault_path: PathBuf,
    protector: Arc<dyn VaultProtector>,
    cache: Arc<Mutex<BTreeMap<String, String>>>,
}

impl SecretStore {
    /// Open (or create) the vault in `app_data_dir`, migrating a legacy vault if necessary.
    ///
    /// Fails with [`SymphonyError::CredentialVault`] when an existing vault cannot be read; the
    /// file is left untouched so no credential is lost.
    pub fn new(app_data_dir: &Path) -> SymphonyResult<Self> {
        secure_fs::ensure_dir(app_data_dir)?;
        let vault_path = Self::vault_path(app_data_dir);
        let vault_exists = vault_path.exists();
        let protector: Arc<dyn VaultProtector> =
            Arc::from(platform_protector(app_data_dir, vault_exists)?);

        let store = Self {
            vault_path: vault_path.clone(),
            protector,
            cache: Arc::new(Mutex::new(BTreeMap::new())),
        };
        if !vault_exists {
            return Ok(store);
        }

        let raw = std::fs::read(&vault_path).map_err(|e| {
            vault_error(format!(
                "cannot read {}: {e}. {}",
                vault_path.display(),
                Self::recovery_hint()
            ))
        })?;
        let entries = match serde_json::from_slice::<VaultEnvelope>(&raw) {
            Ok(envelope) => store.decode_envelope(&envelope)?,
            Err(_) => store.migrate_legacy(&raw)?,
        };
        for value in entries.values() {
            crate::logging::redact_from_logs(value);
        }
        *store.cache.lock().expect("secret cache mutex") = entries;
        Ok(store)
    }

    pub fn vault_path(app_data_dir: &Path) -> PathBuf {
        app_data_dir.join(VAULT_FILE)
    }

    fn recovery_hint() -> &'static str {
        "The file was left unchanged. Restore it from a backup, or move it aside (the desktop \
         app offers this; `symphony-cli --quarantine-credential-vault`) and re-enter your \
         credentials."
    }

    /// Move an unreadable vault aside (`credentials.vault.corrupt-<timestamp>`) so a fresh one can
    /// be created. Returns the new location of the old file, or `None` when there was no vault.
    pub fn quarantine_vault(app_data_dir: &Path) -> SymphonyResult<Option<PathBuf>> {
        let path = Self::vault_path(app_data_dir);
        if !path.exists() {
            return Ok(None);
        }
        // Never overwrite an earlier quarantined vault: pick a name that does not exist yet.
        let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let target = (0u32..)
            .map(|n| match n {
                0 => app_data_dir.join(format!("{VAULT_FILE}.corrupt-{stamp}")),
                n => app_data_dir.join(format!("{VAULT_FILE}.corrupt-{stamp}-{n}")),
            })
            .find(|candidate| !candidate.exists())
            .expect("an unused quarantine name exists");
        std::fs::rename(&path, &target)?;
        warn!(moved_to = %target.display(), "Moved unreadable credential vault aside");
        Ok(Some(target))
    }

    fn decode_envelope(
        &self,
        envelope: &VaultEnvelope,
    ) -> SymphonyResult<BTreeMap<String, String>> {
        let path = self.vault_path.display();
        if envelope.format != VAULT_FORMAT {
            return Err(vault_error(format!(
                "{path} is not a Symphony credential vault. {}",
                Self::recovery_hint()
            )));
        }
        if envelope.version > VAULT_VERSION {
            return Err(vault_error(format!(
                "{path} was written by a newer Symphony version (vault format {}); upgrade \
                 Symphony to read it",
                envelope.version
            )));
        }
        if envelope.version < VAULT_VERSION {
            return Err(vault_error(format!(
                "{path} has unsupported vault format {}. {}",
                envelope.version,
                Self::recovery_hint()
            )));
        }
        if envelope.protection != self.protector.protection() {
            return Err(vault_error(format!(
                "{path} is protected with '{}', which is not available on this platform \
                 (expected '{}')",
                envelope.protection,
                self.protector.protection()
            )));
        }
        let decode = |field: &str, value: &str| {
            b64().decode(value).map_err(|_| {
                vault_error(format!(
                    "{path} is corrupt (invalid {field}). {}",
                    Self::recovery_hint()
                ))
            })
        };
        let blob = SealedBlob {
            nonce: envelope
                .nonce
                .as_deref()
                .map(|n| decode("nonce", n))
                .transpose()?,
            ciphertext: decode("ciphertext", &envelope.ciphertext)?,
        };
        let plaintext = self
            .protector
            .open(&blob, &associated_data(&envelope.protection))
            .map_err(|e| {
                vault_error(format!(
                    "{path} could not be decrypted or failed authentication ({e}). {}",
                    Self::recovery_hint()
                ))
            })?;
        let payload: VaultPayload = serde_json::from_slice(&plaintext).map_err(|_| {
            vault_error(format!(
                "{path} decrypted to invalid data. {}",
                Self::recovery_hint()
            ))
        })?;
        Ok(payload.entries)
    }

    fn encode(&self, entries: &BTreeMap<String, String>) -> SymphonyResult<Vec<u8>> {
        let protection = self.protector.protection();
        let plaintext = Zeroizing::new(serde_json::to_vec(&VaultPayloadRef { entries })?);
        let blob = self
            .protector
            .seal(&plaintext, &associated_data(protection))?;
        let envelope = VaultEnvelope {
            format: VAULT_FORMAT.into(),
            version: VAULT_VERSION,
            protection: protection.into(),
            nonce: blob.nonce.as_ref().map(|n| b64().encode(n)),
            ciphertext: b64().encode(&blob.ciphertext),
        };
        Ok(serde_json::to_vec_pretty(&envelope)?)
    }

    fn persist(&self, entries: &BTreeMap<String, String>) -> SymphonyResult<()> {
        let bytes = self.encode(entries)?;
        secure_fs::write_private_atomic(&self.vault_path, &bytes).map_err(|e| {
            vault_error(format!(
                "failed to write {}: {e}",
                self.vault_path.display()
            ))
        })
    }

    /// Convert a version-1 (XOR-obfuscated) vault to the current format.
    fn migrate_legacy(&self, raw: &[u8]) -> SymphonyResult<BTreeMap<String, String>> {
        let path = self.vault_path.display();
        let text = std::str::from_utf8(raw).map_err(|_| {
            vault_error(format!(
                "{path} is corrupt (unrecognised format). {}",
                Self::recovery_hint()
            ))
        })?;
        let entries = legacy::decode(text).map_err(|_| {
            vault_error(format!(
                "{path} looks like a legacy (format 1) vault but could not be decoded. Legacy \
                 vaults are bound to the Windows user name and computer name that wrote them; \
                 run Symphony as that user on that computer to migrate it. {}",
                Self::recovery_hint()
            ))
        })?;
        self.persist(&entries)?;
        // Read the migrated vault back before trusting it: the legacy file is gone now, so the
        // new one must provably hold every credential.
        let reread = std::fs::read(&self.vault_path)?;
        let envelope: VaultEnvelope = serde_json::from_slice(&reread)?;
        let roundtrip = self.decode_envelope(&envelope)?;
        if roundtrip != entries {
            return Err(vault_error(format!(
                "{path}: migrated vault did not round-trip; refusing to continue"
            )));
        }
        info!(
            credentials = entries.len(),
            "Migrated legacy credential vault to format {VAULT_VERSION}"
        );
        Ok(entries)
    }

    /// Store `value` under `key`; an empty value deletes the key. The change is visible only
    /// after it has been persisted.
    pub fn set_secret(&self, key: &str, value: &str) -> SymphonyResult<()> {
        let mut guard = self.cache.lock().expect("secret cache mutex");
        let mut next = guard.clone();
        let removed = if value.trim().is_empty() {
            next.remove(key)
        } else {
            crate::logging::redact_from_logs(value);
            next.insert(key.to_string(), value.trim().to_string())
        };
        let result = self.persist(&next);
        // Zeroize the copy that is being discarded: `next` on failure, the old map on success.
        let mut discarded = match result {
            Ok(()) => std::mem::replace(&mut *guard, next),
            Err(_) => next,
        };
        for v in discarded.values_mut() {
            v.zeroize();
        }
        if let Some(mut old) = removed {
            old.zeroize();
        }
        result?;
        debug!(key = key, "Updated credential in secure secret store");
        Ok(())
    }

    /// Value stored in the vault only (no environment fallback).
    pub fn get_vault_secret(&self, key: &str) -> Option<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        guard.get(key).filter(|v| !v.is_empty()).cloned()
    }

    pub fn get_secret(&self, key: &str) -> Option<String> {
        self.get_vault_secret(key)
            .or_else(|| std::env::var(key).ok().filter(|s| !s.is_empty()))
    }

    pub fn delete_secret(&self, key: &str) -> SymphonyResult<()> {
        self.set_secret(key, "")
    }

    pub fn list_configured_keys(&self) -> Vec<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        guard.keys().cloned().collect()
    }

    /// Every stored value, for redaction of logs, diffs and diagnostics.
    pub fn all_secret_values(&self) -> Vec<String> {
        let guard = self.cache.lock().expect("secret cache mutex");
        guard.values().cloned().collect()
    }

    /// Name of the protection scheme in use (for diagnostics; contains no secret material).
    pub fn protection(&self) -> &'static str {
        self.protector.protection()
    }
}

impl std::fmt::Debug for SecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretStore")
            .field("vault_path", &self.vault_path)
            .field("protection", &self.protector.protection())
            .field("keys", &self.list_configured_keys())
            .finish()
    }
}

/// Reader for the format-1 vault (XOR with a key derived from user and host name, base64). Kept
/// only to migrate existing files; nothing is ever written in this format.
pub(crate) mod legacy {
    use super::*;

    fn machine_key() -> Vec<u8> {
        use sha2::{Digest, Sha256};
        let user = std::env::var("USERNAME")
            .or_else(|_| std::env::var("USER"))
            .unwrap_or_else(|_| "symphony-local-user".to_string());
        let host = std::env::var("COMPUTERNAME")
            .or_else(|_| std::env::var("HOSTNAME"))
            .unwrap_or_else(|_| "symphony-windows-host".to_string());
        let mut hasher = Sha256::new();
        hasher.update(format!("symphony-desktop-v1:{user}@{host}").as_bytes());
        hasher.finalize().to_vec()
    }

    fn xor(bytes: &[u8]) -> Zeroizing<Vec<u8>> {
        let key = machine_key();
        Zeroizing::new(
            bytes
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ key[i % key.len()])
                .collect(),
        )
    }

    pub fn decode(encoded: &str) -> Result<BTreeMap<String, String>, ()> {
        let bytes = b64().decode(encoded.trim()).map_err(|_| ())?;
        serde_json::from_slice(&xor(&bytes)).map_err(|_| ())
    }

    /// Test helper: produce a format-1 vault exactly as Symphony Desktop ≤ 2.0.0 did.
    #[cfg(test)]
    pub fn encode(map: &BTreeMap<String, String>) -> String {
        b64().encode(xor(&serde_json::to_vec(map).unwrap()).as_slice())
    }
}

#[cfg(windows)]
mod dpapi {
    use super::*;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    /// DPAPI with the current user's master key (no machine-wide scope).
    pub struct DpapiProtector;

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// Copy a DPAPI output blob and free it.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
        let data =
            Zeroizing::new(std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec());
        std::ptr::write_bytes(out.pbData, 0, out.cbData as usize);
        LocalFree(out.pbData as *mut _);
        data
    }

    impl VaultProtector for DpapiProtector {
        fn protection(&self) -> &'static str {
            "windows-dpapi-user"
        }

        fn seal(&self, plaintext: &[u8], aad: &[u8]) -> SymphonyResult<SealedBlob> {
            let input = blob(plaintext);
            let entropy = blob(aad);
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: null_mut(),
            };
            // SAFETY: input/entropy point at live slices; `out` is freed by `take`.
            unsafe {
                if CryptProtectData(
                    &input,
                    null_mut(),
                    &entropy,
                    null_mut(),
                    null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                ) == 0
                {
                    return Err(vault_error(format!(
                        "CryptProtectData failed: {}",
                        std::io::Error::last_os_error()
                    )));
                }
                Ok(SealedBlob {
                    nonce: None,
                    ciphertext: take(out).to_vec(),
                })
            }
        }

        fn open(&self, sealed: &SealedBlob, aad: &[u8]) -> SymphonyResult<Zeroizing<Vec<u8>>> {
            let input = blob(&sealed.ciphertext);
            let entropy = blob(aad);
            let mut out = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: null_mut(),
            };
            // SAFETY: as in `seal`.
            unsafe {
                if CryptUnprotectData(
                    &input,
                    null_mut(),
                    &entropy,
                    null_mut(),
                    null_mut(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut out,
                ) == 0
                {
                    return Err(vault_error(format!(
                        "CryptUnprotectData failed: {}",
                        std::io::Error::last_os_error()
                    )));
                }
                Ok(take(out))
            }
        }
    }
}

#[cfg(not(windows))]
mod keyfile {
    use super::*;
    use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, CHACHA20_POLY1305, NONCE_LEN};
    use ring::rand::{SecureRandom, SystemRandom};
    use std::io::Write;

    const KEY_FILE: &str = "credentials.key";
    const KEY_LEN: usize = 32;

    /// ChaCha20-Poly1305 with a random key stored in an owner-only key file.
    pub struct KeyFileProtector {
        key: LessSafeKey,
        rng: SystemRandom,
    }

    impl KeyFileProtector {
        pub fn load_or_create(app_data_dir: &Path, vault_exists: bool) -> SymphonyResult<Self> {
            let path = app_data_dir.join(KEY_FILE);
            let rng = SystemRandom::new();
            let mut key_bytes = Zeroizing::new([0u8; KEY_LEN]);
            if path.exists() {
                let buf = Zeroizing::new(secure_fs::read_private_file(&path).map_err(|e| {
                    vault_error(format!(
                        "refusing to use the vault key: {e}. Restrict it with `chmod 600 {}`",
                        path.display()
                    ))
                })?);
                if buf.len() != KEY_LEN {
                    return Err(vault_error(format!(
                        "{} is corrupt (expected a {KEY_LEN}-byte key)",
                        path.display()
                    )));
                }
                key_bytes.copy_from_slice(&buf);
            } else if vault_exists {
                // A vault without its key must never be paired with a fresh key: that would make
                // every stored credential silently unrecoverable.
                let legacy = std::fs::read(SecretStore::vault_path(app_data_dir))
                    .ok()
                    .is_some_and(|raw| serde_json::from_slice::<VaultEnvelope>(&raw).is_err());
                if !legacy {
                    return Err(vault_error(format!(
                        "the vault key {} is missing, so the existing credential vault cannot \
                         be decrypted. {}",
                        path.display(),
                        SecretStore::recovery_hint()
                    )));
                }
                Self::create_key(&path, &rng, &mut key_bytes)?;
            } else {
                Self::create_key(&path, &rng, &mut key_bytes)?;
            }
            let key = UnboundKey::new(&CHACHA20_POLY1305, key_bytes.as_ref())
                .map_err(|_| vault_error("invalid vault key"))?;
            Ok(Self {
                key: LessSafeKey::new(key),
                rng,
            })
        }

        fn create_key(
            path: &Path,
            rng: &SystemRandom,
            key_bytes: &mut [u8; KEY_LEN],
        ) -> SymphonyResult<()> {
            rng.fill(key_bytes)
                .map_err(|_| vault_error("system random number generator unavailable"))?;
            let mut file = secure_fs::create_private_file(path)?;
            file.write_all(key_bytes)?;
            file.sync_all()?;
            Ok(())
        }
    }

    impl VaultProtector for KeyFileProtector {
        fn protection(&self) -> &'static str {
            "aead-chacha20poly1305-keyfile"
        }

        fn seal(&self, plaintext: &[u8], aad: &[u8]) -> SymphonyResult<SealedBlob> {
            let mut nonce = [0u8; NONCE_LEN];
            self.rng
                .fill(&mut nonce)
                .map_err(|_| vault_error("system random number generator unavailable"))?;
            let mut in_out = plaintext.to_vec();
            self.key
                .seal_in_place_append_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(aad),
                    &mut in_out,
                )
                .map_err(|_| vault_error("encryption failed"))?;
            Ok(SealedBlob {
                nonce: Some(nonce.to_vec()),
                ciphertext: in_out,
            })
        }

        fn open(&self, blob: &SealedBlob, aad: &[u8]) -> SymphonyResult<Zeroizing<Vec<u8>>> {
            let nonce: [u8; NONCE_LEN] = blob
                .nonce
                .as_deref()
                .and_then(|n| n.try_into().ok())
                .ok_or_else(|| vault_error("missing or invalid nonce"))?;
            let mut in_out = Zeroizing::new(blob.ciphertext.clone());
            let len = self
                .key
                .open_in_place(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(aad),
                    &mut in_out,
                )
                .map_err(|_| vault_error("authentication failed"))?
                .len();
            in_out.truncate(len);
            Ok(in_out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn entries(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn credentials_persist_across_restarts() {
        let dir = tempdir().unwrap();
        {
            let store = SecretStore::new(dir.path()).unwrap();
            store
                .set_secret("GITHUB_TOKEN", "ghp_secret1234567890")
                .unwrap();
            store.set_secret("OPENAI_API_KEY", "sk-test-abc").unwrap();
        }
        // A new process opens the same data directory.
        let reopened = SecretStore::new(dir.path()).unwrap();
        assert_eq!(
            reopened.get_vault_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_secret1234567890")
        );
        assert_eq!(
            reopened.get_vault_secret("OPENAI_API_KEY").as_deref(),
            Some("sk-test-abc")
        );
        secure_fs::verify_private_file(&SecretStore::vault_path(dir.path())).unwrap();
    }

    #[test]
    fn vault_file_never_contains_plaintext() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store
            .set_secret("LINEAR_API_KEY", "lin_api_plaintextcheck")
            .unwrap();
        let raw = std::fs::read_to_string(SecretStore::vault_path(dir.path())).unwrap();
        assert!(!raw.contains("lin_api_plaintextcheck"));
        assert!(
            !raw.contains("LINEAR_API_KEY"),
            "key names are encrypted too"
        );
        let envelope: VaultEnvelope = serde_json::from_str(&raw).unwrap();
        assert_eq!(envelope.version, VAULT_VERSION);
        assert_eq!(envelope.protection, store.protection());
    }

    #[test]
    fn delete_removes_secret_from_disk() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store.set_secret("GH_TOKEN", "ghp_deleteme").unwrap();
        store.delete_secret("GH_TOKEN").unwrap();
        assert!(store.get_vault_secret("GH_TOKEN").is_none());
        let reopened = SecretStore::new(dir.path()).unwrap();
        assert!(reopened.get_vault_secret("GH_TOKEN").is_none());
        assert!(reopened.list_configured_keys().is_empty());
    }

    #[test]
    fn legacy_vault_is_migrated_without_loss() {
        let dir = tempdir().unwrap();
        let legacy_entries = entries(&[
            ("GITHUB_TOKEN", "ghp_legacy_value"),
            ("ANTHROPIC_API_KEY", "sk-ant-legacy"),
        ]);
        let path = SecretStore::vault_path(dir.path());
        std::fs::write(&path, legacy::encode(&legacy_entries)).unwrap();

        let store = SecretStore::new(dir.path()).unwrap();
        assert_eq!(
            store.get_vault_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_legacy_value")
        );
        // The file on disk is now in the current format and private.
        let raw = std::fs::read_to_string(&path).unwrap();
        let envelope: VaultEnvelope = serde_json::from_str(&raw).unwrap();
        assert_eq!(envelope.version, VAULT_VERSION);
        assert!(!raw.contains("ghp_legacy_value"));
        secure_fs::verify_private_file(&path).unwrap();

        // And survives another restart.
        let reopened = SecretStore::new(dir.path()).unwrap();
        assert_eq!(
            reopened.get_vault_secret("ANTHROPIC_API_KEY").as_deref(),
            Some("sk-ant-legacy")
        );
    }

    #[test]
    fn corrupt_vault_is_reported_and_left_untouched() {
        let dir = tempdir().unwrap();
        let path = SecretStore::vault_path(dir.path());
        std::fs::write(&path, b"!!! definitely not a vault !!!").unwrap();
        let err = SecretStore::new(dir.path()).unwrap_err();
        assert!(matches!(err, SymphonyError::CredentialVault(_)), "{err}");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"!!! definitely not a vault !!!"
        );

        // Explicit, recoverable path: move it aside and start fresh.
        let moved = SecretStore::quarantine_vault(dir.path()).unwrap().unwrap();
        assert!(moved.exists());
        assert!(!path.exists());
        let store = SecretStore::new(dir.path()).unwrap();
        assert!(store.list_configured_keys().is_empty());
    }

    #[test]
    fn repeated_quarantine_keeps_every_old_vault() {
        let dir = tempdir().unwrap();
        let path = SecretStore::vault_path(dir.path());
        std::fs::write(&path, b"first").unwrap();
        let first = SecretStore::quarantine_vault(dir.path()).unwrap().unwrap();
        std::fs::write(&path, b"second").unwrap();
        let second = SecretStore::quarantine_vault(dir.path()).unwrap().unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(first).unwrap(), b"first");
        assert_eq!(std::fs::read(second).unwrap(), b"second");
    }

    #[test]
    fn tampered_ciphertext_fails_authentication() {
        let dir = tempdir().unwrap();
        SecretStore::new(dir.path())
            .unwrap()
            .set_secret("GITHUB_TOKEN", "ghp_tamper")
            .unwrap();
        let path = SecretStore::vault_path(dir.path());
        let mut envelope: VaultEnvelope =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut ct = b64().decode(&envelope.ciphertext).unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 0x01;
        envelope.ciphertext = b64().encode(&ct);
        let tampered = serde_json::to_vec(&envelope).unwrap();
        std::fs::write(&path, &tampered).unwrap();

        let err = SecretStore::new(dir.path()).unwrap_err();
        assert!(matches!(err, SymphonyError::CredentialVault(_)));
        assert_eq!(std::fs::read(&path).unwrap(), tampered, "never overwritten");
    }

    #[test]
    fn newer_vault_format_is_rejected() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store.set_secret("GITHUB_TOKEN", "ghp_x").unwrap();
        let path = SecretStore::vault_path(dir.path());
        let mut envelope: VaultEnvelope =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        envelope.version = VAULT_VERSION + 1;
        std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        let err = SecretStore::new(dir.path()).unwrap_err().to_string();
        assert!(err.contains("newer Symphony version"), "{err}");
    }

    #[test]
    fn failed_write_keeps_memory_and_disk_consistent() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store.set_secret("GITHUB_TOKEN", "ghp_before").unwrap();

        // Replace the vault file with a non-empty directory so the atomic rename fails.
        let path = SecretStore::vault_path(dir.path());
        let saved = std::fs::read(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("block"), b"x").unwrap();

        assert!(store.set_secret("GITHUB_TOKEN", "ghp_after").is_err());
        assert!(store.set_secret("OPENAI_API_KEY", "sk-new").is_err());
        assert_eq!(
            store.get_vault_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_before")
        );
        assert!(store.get_vault_secret("OPENAI_API_KEY").is_none());

        // Restore the file: the persisted state matches what the store reported.
        std::fs::remove_dir_all(&path).unwrap();
        std::fs::write(&path, saved).unwrap();
        let reopened = SecretStore::new(dir.path()).unwrap();
        assert_eq!(
            reopened.get_vault_secret("GITHUB_TOKEN").as_deref(),
            Some("ghp_before")
        );
    }

    #[test]
    fn debug_output_never_contains_values() {
        let dir = tempdir().unwrap();
        let store = SecretStore::new(dir.path()).unwrap();
        store.set_secret("GITHUB_TOKEN", "ghp_debugleak").unwrap();
        let printed = format!("{store:?}");
        assert!(printed.contains("GITHUB_TOKEN"));
        assert!(!printed.contains("ghp_debugleak"));
    }

    #[cfg(not(windows))]
    #[test]
    fn missing_key_file_is_an_explicit_error() {
        let dir = tempdir().unwrap();
        SecretStore::new(dir.path())
            .unwrap()
            .set_secret("GITHUB_TOKEN", "ghp_keyfile")
            .unwrap();
        std::fs::remove_file(dir.path().join("credentials.key")).unwrap();
        let err = SecretStore::new(dir.path()).unwrap_err().to_string();
        assert!(err.contains("missing"), "{err}");
        // No replacement key was generated behind the operator's back.
        assert!(!dir.path().join("credentials.key").exists());
    }

    #[cfg(unix)]
    #[test]
    fn world_readable_key_file_is_rejected() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempdir().unwrap();
        SecretStore::new(dir.path()).unwrap();
        let key = dir.path().join("credentials.key");
        std::fs::set_permissions(&key, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = SecretStore::new(dir.path()).unwrap_err().to_string();
        assert!(err.contains("chmod 600"), "{err}");
    }
}
