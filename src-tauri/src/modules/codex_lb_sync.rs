//! Push Cockpit OAuth accounts into the local codex-lb pool and pin routing to Cockpit.

use crate::models::codex::CodexAccount;
use crate::modules::codex_account::build_auth_file_value;
use crate::modules::logger;
use reqwest::multipart;
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use std::path::PathBuf;
use std::process::Command;

const CODEX_LB_ORIGIN: &str = "http://127.0.0.1:2455";
const CODEX_LB_IMPORT_URL: &str = "http://127.0.0.1:2455/api/accounts/import";
const CODEX_LB_IMPORT_TIMEOUT_SECS: u64 = 30;

#[derive(Debug, Clone, Deserialize)]
pub struct CodexLbImportResponse {
    #[serde(rename = "accountId")]
    pub account_id: String,
    pub email: Option<String>,
    #[serde(rename = "planType")]
    pub plan_type: Option<String>,
    pub status: Option<String>,
}

fn codex_lb_routing_script_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex").join("codex-lb-routing.ps1"))
}

fn powershell_executable() -> PathBuf {
    std::env::var("SystemRoot")
        .map(|root| {
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .unwrap_or_else(|_| PathBuf::from("powershell.exe"))
}

fn codex_lb_store_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".codex-lb").join("store.db"))
}

fn find_existing_codex_lb_account(
    account: &CodexAccount,
) -> Result<Option<CodexLbImportResponse>, String> {
    let Some(chatgpt_account_id) = account.account_id.as_deref().map(str::trim) else {
        return Ok(None);
    };
    if chatgpt_account_id.is_empty() {
        return Ok(None);
    }

    let Some(store_path) = codex_lb_store_path() else {
        return Ok(None);
    };
    if !store_path.is_file() {
        return Ok(None);
    }

    let connection = Connection::open(&store_path)
        .map_err(|err| format!("codex-lb store open failed ({}): {}", store_path.display(), err))?;
    let existing = connection
        .query_row(
            "select id, email, plan_type, status \
             from accounts \
             where chatgpt_account_id = ?1 \
             order by case status when 'active' then 0 else 1 end, created_at asc \
             limit 1",
            [chatgpt_account_id],
            |row| {
                Ok(CodexLbImportResponse {
                    account_id: row.get(0)?,
                    email: row.get(1)?,
                    plan_type: row.get(2)?,
                    status: row.get(3)?,
                })
            },
        )
        .optional()
        .map_err(|err| format!("codex-lb store query failed: {}", err))?;

    Ok(existing)
}

fn run_codex_lb_sync_cockpit_routing() {
    let Some(script) = codex_lb_routing_script_path() else {
        return;
    };
    if !script.is_file() {
        logger::log_warn("[codex-lb] sync-cockpit skipped: routing script missing");
        return;
    }

    let output = Command::new(powershell_executable())
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .args(["-Mode", "sync-cockpit"])
        .env("CODEX_LB_SYNC_CONTEXT", "1")
        .output();

    match output {
        Ok(result) => {
            let stdout = String::from_utf8_lossy(&result.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&result.stderr).trim().to_string();
            if result.status.success() {
                logger::log_info(&format!(
                    "[codex-lb] sync-cockpit ok: {}",
                    if stdout.is_empty() { "done" } else { &stdout }
                ));
            } else {
                logger::log_warn(&format!(
                    "[codex-lb] sync-cockpit failed: status={}, stdout={}, stderr={}",
                    result.status, stdout, stderr
                ));
            }
        }
        Err(err) => {
            logger::log_warn(&format!("[codex-lb] sync-cockpit spawn failed: {}", err));
        }
    }
}

/// Best-effort background import after OAuth account save (add / token refresh).
pub fn schedule_sync_oauth_account_to_codex_lb(account: CodexAccount) {
    if account.is_api_key_auth() {
        return;
    }

    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => {
                logger::log_warn(&format!(
                    "[codex-lb] background sync runtime failed: {}",
                    err
                ));
                return;
            }
        };

        if let Err(err) = runtime.block_on(sync_oauth_account_to_codex_lb(&account)) {
            logger::log_warn(&format!(
                "[codex-lb] background import failed: account_id={}, error={}",
                account.id, err
            ));
        }
    });
}

/// Import OAuth tokens into codex-lb and run `sync-cockpit` so LB pins the Cockpit account.
pub async fn sync_oauth_account_to_codex_lb(
    account: &CodexAccount,
) -> Result<Option<CodexLbImportResponse>, String> {
    if account.is_api_key_auth() {
        return Ok(None);
    }
    if account.tokens.access_token.trim().is_empty() {
        return Ok(None);
    }

    if let Some(existing) = find_existing_codex_lb_account(account)? {
        logger::log_info(&format!(
            "[codex-lb] import skipped; account already exists: cockpit_id={}, lb_id={}, email={}, plan={:?}, status={:?}",
            account.id,
            existing.account_id,
            existing.email.as_deref().unwrap_or("-"),
            existing.plan_type,
            existing.status
        ));
        run_codex_lb_sync_cockpit_routing();
        return Ok(Some(existing));
    }

    let auth_value = build_auth_file_value(account)?;
    let auth_bytes = serde_json::to_vec(&auth_value)
        .map_err(|err| format!("codex-lb import payload encode failed: {}", err))?;

    let client = crate::utils::http::create_client(CODEX_LB_IMPORT_TIMEOUT_SECS);
    let part = multipart::Part::bytes(auth_bytes)
        .file_name("auth.json")
        .mime_str("application/json")
        .map_err(|err| format!("codex-lb import multipart build failed: {}", err))?;
    let form = multipart::Form::new().part("auth_json", part);

    let response = client
        .post(CODEX_LB_IMPORT_URL)
        .multipart(form)
        .send()
        .await
        .map_err(|err| {
            format!(
                "codex-lb import request failed (is {} listening?): {}",
                CODEX_LB_ORIGIN, err
            )
        })?;

    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|err| format!("codex-lb import response read failed: {}", err))?;

    if !status.is_success() {
        return Err(format!(
            "codex-lb import HTTP {}: {}",
            status.as_u16(),
            body.trim()
        ));
    }

    let imported: CodexLbImportResponse = serde_json::from_str(&body).map_err(|err| {
        format!(
            "codex-lb import response parse failed: {} body={}",
            err,
            body.trim()
        )
    })?;

    logger::log_info(&format!(
        "[codex-lb] imported account: cockpit_id={}, lb_id={}, email={}, plan={:?}, status={:?}",
        account.id,
        imported.account_id,
        imported.email.as_deref().unwrap_or("-"),
        imported.plan_type,
        imported.status
    ));

    run_codex_lb_sync_cockpit_routing();

    Ok(Some(imported))
}
