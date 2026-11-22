//! Native Qoder CN client adapters. Credential schemas and CLI encryption are
//! adapted from workbuddy-switch-next (MIT); realm selection is always explicit.
use crate::models::qoder::QoderAccount;
use crate::modules::{
    logger,
    qoder_channel::{Channel, CredentialSource},
    qoder_cn_account::QoderCnAccountStore,
    qoder_native_profile,
};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, KeyIvInit};
use aes_gcm::{aead::Aead, Aes256Gcm, KeyInit, Nonce};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
};

fn io_error(error: std::io::Error) -> String {
    format!("QODER_NATIVE_STORAGE: {error}")
}

/// The public CN client, the only channel the native adapters are exposed for.
fn cn_channel() -> Result<Channel, String> {
    Channel::for_source(CredentialSource::PublicCn).ok_or_else(|| "QODER_REGION_INVALID".into())
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeClient {
    Desktop,
    Cli,
}

impl NativeClient {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "desktop" => Ok(Self::Desktop),
            "cli" => Ok(Self::Cli),
            _ => Err("QODER_CLIENT_INVALID".into()),
        }
    }

    fn root(self, channel: Channel) -> Result<PathBuf, String> {
        match self {
            Self::Desktop => Ok(dirs::config_dir()
                .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
                .join(channel.desktop_dir())),
            Self::Cli => Ok(dirs::home_dir()
                .ok_or("QODER_DATA_ROOT_UNAVAILABLE")?
                .join(channel.cli_dir())
                .join(".auth")),
        }
    }

    fn filename(self) -> &'static str {
        match self {
            Self::Desktop => "auth.v1.dat",
            Self::Cli => "user",
        }
    }
}

#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::{
        Foundation::{LocalFree, HLOCAL},
        Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
    };
    let source = CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| "QODER_NATIVE_KEY_INVALID")?,
        pbData: bytes.as_ptr() as *mut u8,
    };
    let mut result = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&source, None, None, None, None, 1, &mut result)
            .map_err(|_| "QODER_NATIVE_DPAPI_FAILED")?;
        let plain = std::slice::from_raw_parts(result.pbData, result.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(result.pbData as _));
        Ok(plain)
    }
}

fn desktop_key(root: &Path) -> Result<Vec<u8>, String> {
    #[cfg(windows)]
    {
        let state: Value =
            serde_json::from_slice(&fs::read(root.join("Local State")).map_err(io_error)?)
                .map_err(|_| "QODER_NATIVE_LOCAL_STATE_INVALID")?;
        let raw = STANDARD
            .decode(
                state
                    .pointer("/os_crypt/encrypted_key")
                    .and_then(Value::as_str)
                    .ok_or("QODER_NATIVE_KEY_MISSING")?,
            )
            .map_err(|_| "QODER_NATIVE_KEY_INVALID")?;
        if !raw.starts_with(b"DPAPI") {
            return Err("QODER_NATIVE_KEY_UNSUPPORTED".into());
        }
        let key = unprotect(&raw[5..])?;
        if key.len() != 32 {
            return Err("QODER_NATIVE_KEY_INVALID".into());
        }
        Ok(key)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Err("Native desktop adapter currently supports Windows; use the CLI adapter on this platform".into())
    }
}

fn decode(root: &Path, client: NativeClient, bytes: &[u8]) -> Result<Value, String> {
    let plain = match client {
        NativeClient::Desktop => {
            if !bytes.starts_with(b"v10") || bytes.len() < 31 {
                return Err("QODER_NATIVE_ENVELOPE_UNSUPPORTED".into());
            }
            Aes256Gcm::new_from_slice(&desktop_key(root)?)
                .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                .decrypt(Nonce::from_slice(&bytes[3..15]), &bytes[15..])
                .map_err(|_| "QODER_NATIVE_DECRYPT_FAILED")?
        }
        NativeClient::Cli => {
            if bytes.first() == Some(&b'{') {
                bytes.to_vec()
            } else {
                let machine = fs::read_to_string(root.join("machine_id")).map_err(io_error)?;
                let key = machine
                    .trim()
                    .as_bytes()
                    .get(..16)
                    .ok_or("QODER_NATIVE_MACHINE_ID_INVALID")?;
                let mut encrypted = STANDARD
                    .decode(bytes)
                    .map_err(|_| "QODER_NATIVE_ENVELOPE_INVALID")?;
                cbc::Decryptor::<aes::Aes128>::new_from_slices(key, key)
                    .map_err(|_| "QODER_NATIVE_KEY_INVALID")?
                    .decrypt_padded_mut::<Pkcs7>(&mut encrypted)
                    .map_err(|_| "QODER_NATIVE_DECRYPT_FAILED")?
                    .to_vec()
            }
        }
    };
    serde_json::from_slice(&plain).map_err(|_| "QODER_NATIVE_JSON_INVALID".into())
}

/// Reads the credentials the installed CN client currently holds and stores
/// them as an account. Nothing is written back to the client.
pub fn import(client: NativeClient) -> Result<QoderAccount, String> {
    let channel = cn_channel()?;
    let root = client.root(channel)?;
    let profile = decode(
        &root,
        client,
        &fs::read(root.join(client.filename())).map_err(io_error)?,
    )?;
    let info = qoder_native_profile::from_client(&profile, channel)?;
    QoderCnAccountStore::default_store()?.upsert_from_snapshot(info, None, None)
}

/// Imports whichever CN client profiles exist. Returns `Ok(false)` when neither
/// client has ever signed in on this machine.
pub fn import_from_local() -> Result<bool, String> {
    let channel = cn_channel()?;
    let mut found = false;
    let mut last_error = None;
    let mut imported = false;
    for client in [NativeClient::Desktop, NativeClient::Cli] {
        let path = client.root(channel)?.join(client.filename());
        if !path.exists() {
            continue;
        }
        found = true;
        match import(client) {
            Ok(_) => imported = true,
            Err(error) => {
                logger::log_warn(&format!(
                    "[Qoder CN Native] 导入客户端凭据失败: client={}, error={error}",
                    client.filename()
                ));
                last_error = Some(error);
            }
        }
    }
    if !found {
        return Ok(false);
    }
    if imported {
        Ok(true)
    } else {
        Err(last_error.unwrap_or_else(|| "QODER_NATIVE_IMPORT_FAILED".into()))
    }
}

/// Validates an import payload and stamps the CN marker before it reaches the
/// store, so a global account can never land in the CN index.
pub fn prepare_cn_import(value: &mut Value) -> Result<(), String> {
    qoder_native_profile::prepare_import(value, cn_channel()?)
}
