// Included by codex_account. Mirrors the official keyring / encrypted local auth format.
// No independent cache: each operation reads the selected authoritative store.
#[cfg(target_os = "windows")]
mod windows_auth_store {
    use super::*;
    use age::secrecy::SecretString;
    use std::io::Read;

    const AUTH_SECRET: &str = "global/CODEX_AUTH";

    fn entry(base_dir: &Path, service: &str, prefix: &str) -> Result<keyring::Entry, String> {
        let canonical = fs::canonicalize(base_dir).unwrap_or_else(|_| base_dir.to_path_buf());
        let digest = format!(
            "{:x}",
            Sha256::digest(canonical.to_string_lossy().as_bytes())
        );
        keyring::Entry::new(service, &format!("{prefix}|{}", &digest[..16]))
            .map_err(|_| "CODEX_AUTH_KEYRING_UNAVAILABLE".to_string())
    }

    fn load_secret(entry: &keyring::Entry) -> Result<Option<String>, String> {
        match entry.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err("CODEX_AUTH_KEYRING_UNAVAILABLE".to_string()),
        }
    }

    fn direct(base_dir: &Path) -> bool {
        fs::read_to_string(get_config_toml_path(base_dir))
            .ok()
            .and_then(|s| {
                crate::modules::codex_config_format::read_codex_config_doc_from_str(&s).ok()
            })
            .is_some_and(|doc| {
                doc.get("features")
                    .and_then(|features| features.get("secret_auth_storage"))
                    .and_then(|value| value.as_bool())
                    == Some(false)
            })
    }

    fn path(base_dir: &Path) -> PathBuf {
        base_dir.join("secrets/codex_auth.age")
    }

    pub(super) fn credentials_present(base_dir: &Path) -> bool {
        if path(base_dir).is_file() {
            return true;
        }
        // No scrypt decryption is needed for recovery preservation. On lookup
        // failure, retain the profile rather than destroy potentially valid auth.
        entry(base_dir, "Codex Auth", "cli")
            .and_then(|key| load_secret(&key))
            .map(|value| value.is_some_and(|secret| !secret.trim().is_empty()))
            .unwrap_or(true)
    }

    fn decode(ciphertext: &[u8], passphrase: String) -> Result<serde_json::Value, String> {
        let identity = age::scrypt::Identity::new(SecretString::from(passphrase));
        let decryptor = age::Decryptor::new(ciphertext)
            .map_err(|_| "CODEX_AUTH_SECRETS_INVALID".to_string())?;
        let mut reader = decryptor
            .decrypt(std::iter::once(&identity as &dyn age::Identity))
            .map_err(|_| "CODEX_AUTH_SECRETS_DECRYPT_FAILED".to_string())?;
        let mut bytes = Vec::new();
        reader
            .by_ref()
            .take(8 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "CODEX_AUTH_SECRETS_INVALID".to_string())?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err("CODEX_AUTH_SECRETS_INVALID".to_string());
        }
        let doc: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "CODEX_AUTH_SECRETS_INVALID".to_string())?;
        if doc["version"] != 1 || !doc["secrets"].is_object() {
            return Err("CODEX_AUTH_SECRETS_VERSION_UNSUPPORTED".to_string());
        }
        Ok(doc)
    }

    fn encode(doc: &serde_json::Value, passphrase: String) -> Result<Vec<u8>, String> {
        let recipient = age::scrypt::Recipient::new(SecretString::from(passphrase));
        let encryptor =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .map_err(|_| "CODEX_AUTH_SECRETS_ENCRYPT_FAILED".to_string())?;
        let mut bytes = Vec::new();
        let mut writer = encryptor
            .wrap_output(&mut bytes)
            .map_err(|_| "CODEX_AUTH_SECRETS_ENCRYPT_FAILED".to_string())?;
        std::io::Write::write_all(
            &mut writer,
            &serde_json::to_vec(doc).map_err(|e| e.to_string())?,
        )
        .map_err(|_| "CODEX_AUTH_SECRETS_ENCRYPT_FAILED".to_string())?;
        writer
            .finish()
            .map_err(|_| "CODEX_AUTH_SECRETS_ENCRYPT_FAILED".to_string())?;
        Ok(bytes)
    }

    pub(super) fn read(base_dir: &Path) -> Result<Option<serde_json::Value>, String> {
        let payload = if direct(base_dir) {
            load_secret(&entry(base_dir, "Codex Auth", "cli")?)?
        } else {
            if !path(base_dir).is_file() {
                return Ok(None);
            }
            let passphrase = load_secret(&entry(base_dir, "codex", "secrets")?)?
                .ok_or("CODEX_AUTH_SECRETS_KEY_MISSING")?;
            let doc = decode(
                &fs::read(path(base_dir)).map_err(|e| e.to_string())?,
                passphrase,
            )?;
            doc["secrets"][AUTH_SECRET].as_str().map(str::to_string)
        };
        payload
            .map(|s| serde_json::from_str(&s).map_err(|_| "CODEX_AUTH_SECRETS_INVALID".to_string()))
            .transpose()
    }

    pub(super) fn write(base_dir: &Path, payload: &serde_json::Value) -> Result<(), String> {
        let serialized = serde_json::to_string(payload).map_err(|e| e.to_string())?;
        if direct(base_dir) {
            return entry(base_dir, "Codex Auth", "cli")?
                .set_password(&serialized)
                .map_err(|_| "CODEX_AUTH_KEYRING_WRITE_FAILED".to_string());
        }
        let file = path(base_dir);
        let key = entry(base_dir, "codex", "secrets")?;
        let passphrase = match load_secret(&key)? {
            Some(passphrase) => passphrase,
            None if file.exists() => return Err("CODEX_AUTH_SECRETS_KEY_MISSING".to_string()),
            None => {
                use base64::Engine;
                let value =
                    base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>());
                key.set_password(&value)
                    .map_err(|_| "CODEX_AUTH_KEYRING_WRITE_FAILED".to_string())?;
                value
            }
        };
        let mut doc = if file.exists() {
            decode(
                &fs::read(&file).map_err(|e| e.to_string())?,
                passphrase.clone(),
            )?
        } else {
            serde_json::json!({"version": 1, "secrets": {}})
        };
        doc["secrets"][AUTH_SECRET] = serde_json::Value::String(serialized);
        let ciphertext = encode(&doc, passphrase)?;
        fs::create_dir_all(file.parent().ok_or("CODEX_AUTH_SECRETS_INVALID")?)
            .map_err(|e| e.to_string())?;
        // tempfile::persist uses replacement semantics on Windows; no delete-before-write gap.
        let mut temporary =
            tempfile::NamedTempFile::new_in(file.parent().unwrap()).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut temporary, &ciphertext).map_err(|e| e.to_string())?;
        temporary.as_file().sync_all().map_err(|e| e.to_string())?;
        temporary.persist(&file).map_err(|e| e.to_string())?;
        Ok(())
    }

    pub(super) fn delete(base_dir: &Path) -> Result<bool, String> {
        // Keep shared MCP keys unless the entire owned temporary login profile is
        // being removed. The caller retains the directory if key cleanup fails.
        let removed = match entry(base_dir, "Codex Auth", "cli")?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err("CODEX_AUTH_KEYRING_DELETE_FAILED".to_string()),
        }?;
        if base_dir.join(".cockpit-temp-login.json").is_file()
            || (!base_dir.join("secrets/local.age").exists()
                && !base_dir.join("secrets/mcp_oauth.age").exists())
        {
            match entry(base_dir, "codex", "secrets")?.delete_credential() {
                Ok(()) => return Ok(true),
                Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err("CODEX_AUTH_KEYRING_DELETE_FAILED".to_string()),
            }
        }
        Ok(removed)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn official_age_auth_format_roundtrips_and_keeps_other_secrets() {
            let doc = serde_json::json!({"version":1,"secrets":{"global/CODEX_AUTH":"{\"auth_mode\":\"chatgpt\"}","global/OTHER":"keep"}});
            let encoded = encode(&doc, "isolated-test-passphrase".to_string()).unwrap();
            assert!(encoded.starts_with(b"age-encryption.org/v1"));
            assert_eq!(
                decode(&encoded, "isolated-test-passphrase".to_string()).unwrap(),
                doc
            );
            assert!(decode(&encoded, "wrong-passphrase".to_string()).is_err());
        }

        #[test]
        #[ignore = "requires explicit installed CLI path; uses only disposable synthetic credentials"]
        fn auth_store_matches_installed_official_cli() {
            use std::os::windows::process::CommandExt;
            let cli = std::env::var("COCKPIT_TEST_OFFICIAL_CODEX_EXE").expect("explicit CLI path");
            for backend in ["secrets", "direct"] {
                let dir = tempfile::tempdir().unwrap();
                struct Cleanup<'a>(&'a Path);
                impl Drop for Cleanup<'_> {
                    fn drop(&mut self) {
                        let _ = delete(self.0);
                    }
                }
                let _cleanup = Cleanup(dir.path());
                let encrypted = backend == "secrets";
                fs::write(dir.path().join("config.toml"), format!("cli_auth_credentials_store = \"keyring\"\n[features]\nsecret_auth_storage = {encrypted}\n")).unwrap();
                use base64::Engine;
                let claims = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::json!({
                    "email":"synthetic@example.test", "https://api.openai.com/auth":{"chatgpt_account_id":"synthetic-account", "chatgpt_plan_type":"free"}
                }).to_string());
                let payload = serde_json::json!({"auth_mode":"chatgpt", "tokens":{"id_token":format!("e30.{claims}.signature"),"access_token":"synthetic-access","refresh_token":"synthetic-refresh","account_id":"synthetic-account"}});
                write(dir.path(), &payload).unwrap();
                assert!(credentials_present(dir.path()));
                assert_eq!(read(dir.path()).unwrap().unwrap(), payload);
                let output = std::process::Command::new(&cli)
                    .args(["login", "status"])
                    .env("CODEX_HOME", dir.path())
                    .env_remove("CODEX_ACCESS_TOKEN")
                    .env_remove("CODEX_API_KEY")
                    .env_remove("OPENAI_API_KEY")
                    .creation_flags(0x08000000)
                    .output()
                    .unwrap();
                assert!(
                    output.status.success(),
                    "official CLI did not accept {backend} storage"
                );
                let text = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                assert!(
                    text.contains("Logged in using ChatGPT"),
                    "official CLI did not load synthetic ChatGPT credentials"
                );
            }
        }
    }
}
