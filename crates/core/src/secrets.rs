//! Upload tokens and keys live ONLY in the Windows Credential Manager (protected by the user's DPAPI key).
//! They are never written to the settings file, the log or crash reports: every secret that is read or written is
//! also registered with the log redactor.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::diag;

/// Where secrets come from. The real implementation is the Credential Manager; tests use memory.
pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str);
}

/// In-memory store for tests and the demo (nothing real is touched).
#[derive(Default)]
pub struct MemoryStore(Mutex<HashMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Option<String> {
        self.0.lock().ok()?.get(name).cloned()
    }

    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        diag::register_secret(value);
        self.0
            .lock()
            .map_err(|e| e.to_string())?
            .insert(name.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, name: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(name);
        }
    }
}

#[cfg(windows)]
pub use win::CredentialManager;

#[cfg(windows)]
mod win {
    use windows::Win32::Security::Credentials::{
        CRED_FLAGS, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredDeleteW, CredFree, CredReadW,
        CredWriteW,
    };
    use windows::core::{PCWSTR, PWSTR};

    use super::{SecretStore, diag};

    pub struct CredentialManager;

    fn target(name: &str) -> Vec<u16> {
        format!("SnapForge/{name}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect()
    }

    impl SecretStore for CredentialManager {
        fn get(&self, name: &str) -> Option<String> {
            let t = target(name);
            let mut cred: *mut CREDENTIALW = std::ptr::null_mut();
            // SAFETY: `cred` receives a buffer owned by the API, copied out and freed with CredFree.
            unsafe {
                CredReadW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None, &mut cred).ok()?;
                let c = &*cred;
                let blob = std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec();
                CredFree(cred as *const _);
                let value = String::from_utf8(blob).ok()?;
                diag::register_secret(&value);
                Some(value)
            }
        }

        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            diag::register_secret(value);
            let mut t = target(name);
            let mut user: Vec<u16> = "SnapForge".encode_utf16().chain(std::iter::once(0)).collect();
            let mut blob = value.as_bytes().to_vec();
            let cred = CREDENTIALW {
                Flags: CRED_FLAGS(0),
                Type: CRED_TYPE_GENERIC,
                TargetName: PWSTR(t.as_mut_ptr()),
                CredentialBlobSize: blob.len() as u32,
                CredentialBlob: blob.as_mut_ptr(),
                Persist: CRED_PERSIST_LOCAL_MACHINE,
                UserName: PWSTR(user.as_mut_ptr()),
                ..Default::default()
            };
            // SAFETY: all pointers refer to buffers that outlive the call; the API copies the data.
            unsafe { CredWriteW(&cred, 0) }.map_err(|e| e.message())
        }

        fn delete(&self, name: &str) {
            let t = target(name);
            // SAFETY: valid NUL-terminated target name.
            unsafe {
                let _ = CredDeleteW(PCWSTR(t.as_ptr()), CRED_TYPE_GENERIC, None);
            }
        }
    }
}

/// Fills `{secret:NAME}` placeholders in `text` from the store. Unknown names are an error, so a typo never sends
/// the literal placeholder to a server.
pub fn expand(text: &str, store: &dyn SecretStore) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("{secret:") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "{secret:".len()..];
        let Some(end) = after.find('}') else {
            return Err("unterminated {secret:…} placeholder".into());
        };
        let name = &after[..end];
        let v = store.get(name).ok_or_else(|| format!("secret '{name}' is not set"))?;
        out.push_str(&v);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::TEST_SERIAL;

    #[test]
    fn placeholders_are_expanded_from_the_store() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let s = MemoryStore::default();
        s.set("api", "abc123").unwrap();
        assert_eq!(expand("Bearer {secret:api}", &s).unwrap(), "Bearer abc123");
        assert_eq!(expand("no placeholders", &s).unwrap(), "no placeholders");
        assert_eq!(expand("{secret:api}/{secret:api}", &s).unwrap(), "abc123/abc123");
        diag::clear_secrets();
    }

    #[test]
    fn unknown_or_broken_placeholders_fail_instead_of_leaking_the_template() {
        let s = MemoryStore::default();
        assert!(expand("x {secret:nope}", &s).unwrap_err().contains("nope"));
        assert!(expand("x {secret:broken", &s).is_err());
    }

    #[test]
    fn stored_secrets_are_masked_in_logs() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let s = MemoryStore::default();
        s.set("t", "VERYSECRETVALUE99").unwrap();
        assert!(!diag::redact("sent VERYSECRETVALUE99 to server").contains("VERYSECRETVALUE99"));
        diag::clear_secrets();
    }

    #[cfg(windows)]
    #[test]
    fn credential_manager_roundtrip() {
        let _g = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let store = CredentialManager;
        let name = format!("test-{}", std::process::id());
        assert_eq!(store.get(&name), None);
        store.set(&name, "тест-секрет-123").unwrap();
        assert_eq!(store.get(&name).as_deref(), Some("тест-секрет-123"));
        store.set(&name, "second").unwrap();
        assert_eq!(store.get(&name).as_deref(), Some("second"), "overwrite");
        store.delete(&name);
        assert_eq!(store.get(&name), None);
        diag::clear_secrets();
    }
}
