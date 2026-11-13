use super::{
    antigravity_account_index_store::{IndexDocument, IndexStore},
    provider_account_index::{self, provider_index},
};
use crate::models::{
    claude, codebuddy, github_copilot, grok, minimax_code, qoder, workbuddy, zcode, zed,
};
use std::{collections::VecDeque, path::Path};

provider_index!(
    crate::models::minimax_code::MinimaxCodeAccountIndex,
    crate::models::minimax_code::MinimaxCodeAccountSummary,
    "minimax_code",
    false,
    None
);

provider_index!(
    crate::models::grok::GrokAccountIndex,
    crate::models::grok::GrokAccountSummary,
    "grok",
    false,
    Some("1.0")
);

provider_index!(
    crate::models::zcode::ZcodeAccountIndex,
    crate::models::zcode::ZcodeAccountSummary,
    "zcode",
    true,
    None,
    current_account_id
);

provider_index!(
    crate::models::claude::ClaudeAccountIndex,
    crate::models::claude::ClaudeAccountSummary,
    "claude",
    true,
    None
);

pub(crate) fn schedule_all() -> Result<(), String> {
    schedule_root(&super::account::get_data_dir()?);
    Ok(())
}

pub(crate) fn schedule<T: IndexDocument>(store: IndexStore<T>) {
    if let Some(root) = store.legacy_path().parent() {
        schedule_root(root);
    }
}

type Batch = Box<dyn Fn() -> Result<bool, String> + Send>;

fn batch<T: IndexDocument + Send + 'static>(root: &Path, file: &str) -> Batch {
    let path = root.join(file);
    Box::new(move || {
        let store = provider_account_index::store::<T>(&path)?;
        if !store.has_legacy_source()? {
            return Ok(true);
        }
        store.migrate_batch()
    })
}

fn schedule_root(root: &Path) {
    let mut work: VecDeque<(Batch, u8)> = [
        batch::<codebuddy::CodebuddyAccountIndex>(root, "codebuddy_accounts.json"),
        batch::<github_copilot::GitHubCopilotAccountIndex>(root, "github_copilot_accounts.json"),
        batch::<qoder::QoderAccountIndex>(root, "qoder_accounts.json"),
        batch::<workbuddy::WorkbuddyAccountIndex>(root, "workbuddy_accounts.json"),
        batch::<zed::ZedAccountIndex>(root, "zed_accounts.json"),
        batch::<minimax_code::MinimaxCodeAccountIndex>(root, "minimax_code_accounts.json"),
        batch::<grok::GrokAccountIndex>(root, "grok_accounts.json"),
        batch::<zcode::ZcodeAccountIndex>(root, "zcode_accounts.json"),
        batch::<claude::ClaudeAccountIndex>(root, "claude_accounts.json"),
        batch::<crate::models::kiro::KiroAccountIndex>(root, "kiro_accounts.json"),
        batch::<crate::models::windsurf::WindsurfAccountIndex>(root, "windsurf_accounts.json"),
        batch::<crate::models::trae::TraeAccountIndex>(root, "trae_accounts.json"),
        batch::<codebuddy::CodebuddyAccountIndex>(root, "codebuddy_cn_accounts.json"),
    ]
    .into_iter()
    .map(|batch| (batch, 0))
    .collect();
    super::background_maintenance::schedule(
        format!("provider-indexes:{}", root.display()),
        move || {
            if let Some((batch, failures)) = work.pop_front() {
                match batch() {
                    Ok(true) => {}
                    Ok(false) => work.push_back((batch, 0)),
                    Err(error) => {
                        super::logger::log_warn(&format!("[ProviderIndex] {error}"));
                        if failures < 4 {
                            work.push_back((batch, failures + 1));
                        }
                    }
                }
            }
            Ok(work.is_empty())
        },
    );
}
