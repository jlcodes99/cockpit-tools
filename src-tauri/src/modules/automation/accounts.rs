use super::{blocking, Failure, Platform};
use crate::{commands, models, modules};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AccountSummary {
    id: String,
    email: Option<String>,
    name: Option<String>,
    plan: Option<String>,
    tags: Vec<String>,
}

pub(super) trait AccountProjection {
    fn summary(&self) -> AccountSummary;
}

macro_rules! project {
    ($model:ty, |$account:ident| $email:expr, $name:expr, $plan:expr) => {
        impl AccountProjection for $model {
            fn summary(&$account) -> AccountSummary {
                AccountSummary {
                    id: $account.id.clone(),
                    email: $email,
                    name: $name,
                    plan: $plan,
                    tags: Option::<Vec<String>>::from($account.tags.clone()).unwrap_or_default(),
                }
            }
        }
    };
}

project!(
    models::Account,
    |self| Some(self.email.clone()),
    self.name.clone(),
    self.quota
        .as_ref()
        .and_then(|quota| quota.subscription_tier.clone())
);
project!(
    models::codex::CodexAccount,
    |self| Some(self.email.clone()),
    self.account_name.clone(),
    self.plan_type.clone()
);
project!(
    models::claude::ClaudeAccount,
    |self| Some(self.email.clone()),
    self.organization_name.clone(),
    self.plan_type.clone()
);
project!(
    models::zed::ZedAccount,
    |self| None,
    self.display_name
        .clone()
        .or_else(|| Some(self.github_login.clone())),
    self.plan_raw.clone()
);
project!(
    models::github_copilot::GitHubCopilotAccount,
    |self| self.github_email.clone(),
    Some(self.github_login.clone()),
    self.copilot_plan.clone()
);
project!(
    models::windsurf::WindsurfAccount,
    |self| self.github_email.clone(),
    Some(self.github_login.clone()),
    self.copilot_plan.clone()
);
project!(
    models::kiro::KiroAccount,
    |self| Some(self.email.clone()),
    None,
    self.plan_name.clone().or_else(|| self.plan_tier.clone())
);
project!(
    models::cursor::CursorAccount,
    |self| Some(self.email.clone()),
    self.name.clone(),
    self.membership_type.clone()
);
project!(
    models::grok::GrokAccountView,
    |self| Some(self.email.clone()),
    None,
    self.plan_type.clone()
);
project!(
    models::codebuddy::CodebuddyAccount,
    |self| Some(self.email.clone()),
    self.nickname.clone(),
    self.plan_type.clone()
);
project!(
    models::qoder::QoderAccount,
    |self| Some(self.email.clone()),
    self.display_name.clone(),
    self.plan_type.clone()
);
project!(
    models::zcode::ZcodeAccount,
    |self| Some(self.email.clone()),
    self.display_name.clone(),
    self.plan_type.clone()
);
project!(
    models::trae::TraeAccount,
    |self| Some(self.email.clone()),
    self.nickname.clone(),
    self.plan_type.clone()
);
project!(
    models::workbuddy::WorkbuddyAccount,
    |self| Some(self.email.clone()),
    self.nickname.clone(),
    self.plan_type.clone()
);
project!(
    models::minimax_code::MinimaxCodeAccount,
    |self| Some(self.email.clone()),
    self.nickname.clone(),
    self.plan_type.clone()
);

fn summaries<T: AccountProjection>(accounts: Vec<T>) -> Vec<AccountSummary> {
    accounts.iter().map(AccountProjection::summary).collect()
}

pub(super) async fn list(platform: Platform) -> Result<Vec<AccountSummary>, Failure> {
    use Platform::*;
    let list = match platform {
        Antigravity | AntigravityIde => commands::account::list_accounts().await.map(summaries),
        Codex => commands::codex::list_codex_accounts().await.map(summaries),
        Claude => commands::claude::list_claude_accounts()
            .await
            .map(summaries),
        Zed => return blocking(|| commands::zed::list_zed_accounts().map(summaries)).await,
        Copilot => {
            return blocking(|| {
                commands::github_copilot::list_github_copilot_accounts().map(summaries)
            })
            .await
        }
        Windsurf => commands::windsurf::list_windsurf_accounts()
            .await
            .map(summaries),
        Kiro => commands::kiro::list_kiro_accounts().await.map(summaries),
        Cursor => commands::cursor::list_cursor_accounts()
            .await
            .map(summaries),
        Grok => commands::grok::list_grok_accounts().await.map(summaries),
        Codebuddy => {
            return blocking(|| commands::codebuddy::list_codebuddy_accounts().map(summaries)).await
        }
        CodebuddyCn => {
            return blocking(|| commands::codebuddy_cn::list_codebuddy_cn_accounts().map(summaries))
                .await
        }
        Qoder => return blocking(|| commands::qoder::list_qoder_accounts().map(summaries)).await,
        QoderCn => {
            return blocking(|| commands::qoder_cn::list_qoder_cn_accounts().map(summaries)).await
        }
        Zcode => return blocking(|| commands::zcode::list_zcode_accounts().map(summaries)).await,
        Trae | TraeSolo | TraeCn | TraeSoloCn => {
            let kind = trae_kind(platform)?;
            commands::trae::list_trae_accounts().await.map(|accounts| {
                accounts
                    .iter()
                    .filter(|account| {
                        modules::trae_account::resolve_account_platform_kind(account) == kind
                    })
                    .map(AccountProjection::summary)
                    .collect()
            })
        }
        Workbuddy => {
            return blocking(|| commands::workbuddy::list_workbuddy_accounts().map(summaries)).await
        }
        WorkbuddyAi => commands::workbuddy_ai::list_workbuddy_ai_accounts()
            .await
            .map(summaries),
        Minimax => {
            return blocking(|| commands::minimax_code::list_minimax_code_accounts().map(summaries))
                .await
        }
    };
    list.map_err(Failure::host)
}

fn trae_kind(platform: Platform) -> Result<modules::trae_account::TraePlatformKind, Failure> {
    modules::trae_account::TraePlatformKind::parse(Some(platform.as_str())).map_err(Failure::host)
}

fn project_loaded<T, R>(result: Option<T>, projection: impl FnOnce(T) -> R) -> Result<R, String> {
    result
        .map(projection)
        .ok_or_else(|| "account_not_found".into())
}

pub(super) async fn lookup<R: Send + 'static>(
    platform: Platform,
    id: String,
    project: impl FnOnce(StoredAccount) -> R + Send + 'static,
) -> Result<R, Failure> {
    use Platform::*;
    blocking(move || {
        let account = match platform {
            Antigravity | AntigravityIde => {
                StoredAccount::Antigravity(modules::account::load_account(&id)?)
            }
            Codex => project_loaded(
                modules::codex_account::load_account_checked(&id)?,
                StoredAccount::Codex,
            )?,
            Claude => project_loaded(
                modules::claude_account::load_account(&id),
                StoredAccount::Claude,
            )?,
            Zed => project_loaded(modules::zed_account::load_stored_account(&id), |account| {
                StoredAccount::Zed(account.to_public())
            })?,
            Copilot => project_loaded(
                modules::github_copilot_account::load_account(&id),
                StoredAccount::Copilot,
            )?,
            Windsurf => project_loaded(
                modules::windsurf_account::load_account(&id),
                StoredAccount::Windsurf,
            )?,
            Kiro => project_loaded(
                modules::kiro_account::load_account(&id),
                StoredAccount::Kiro,
            )?,
            Cursor => project_loaded(
                modules::cursor_account::load_account(&id),
                StoredAccount::Cursor,
            )?,
            Grok => project_loaded(modules::grok_account::load_account(&id), |account| {
                StoredAccount::Grok(models::grok::GrokAccountView::from(&account))
            })?,
            Codebuddy => project_loaded(
                modules::codebuddy_account::load_account(&id),
                StoredAccount::Codebuddy,
            )?,
            CodebuddyCn => project_loaded(
                modules::codebuddy_cn_account::load_account(&id),
                StoredAccount::Codebuddy,
            )?,
            Qoder => project_loaded(
                modules::qoder_account::load_account(&id),
                StoredAccount::Qoder,
            )?,
            QoderCn => {
                // The CN store reports real IO failures; only a missing file is
                // "not found", so the automation reply keeps the distinction.
                let store = modules::qoder_cn_account::QoderCnAccountStore::default_store()?;
                let account = store
                    .load_account(&id)?
                    .ok_or_else(|| "account_not_found".to_string())?;
                StoredAccount::Qoder(account)
            }
            Zcode => project_loaded(
                modules::zcode_account::load_account(&id),
                StoredAccount::Zcode,
            )?,
            Trae | TraeSolo | TraeCn | TraeSoloCn => {
                let kind = modules::trae_account::TraePlatformKind::parse(Some(platform.as_str()))?;
                let account = modules::trae_account::load_account(&id).filter(|account| {
                    modules::trae_account::resolve_account_platform_kind(account) == kind
                });
                project_loaded(account, StoredAccount::Trae)?
            }
            Workbuddy => project_loaded(
                modules::workbuddy_account::load_account(&id),
                StoredAccount::Workbuddy,
            )?,
            WorkbuddyAi => StoredAccount::Workbuddy(modules::workbuddy_ai_account::load(&id)?),
            Minimax => project_loaded(
                modules::minimax_code_account::load_account(&id),
                StoredAccount::Minimax,
            )?,
        };
        Ok(project(account))
    })
    .await
}

pub(super) enum StoredAccount {
    Antigravity(models::Account),
    Codex(models::codex::CodexAccount),
    Claude(models::claude::ClaudeAccount),
    Zed(models::zed::ZedAccount),
    Copilot(models::github_copilot::GitHubCopilotAccount),
    Windsurf(models::windsurf::WindsurfAccount),
    Kiro(models::kiro::KiroAccount),
    Cursor(models::cursor::CursorAccount),
    Grok(models::grok::GrokAccountView),
    Codebuddy(models::codebuddy::CodebuddyAccount),
    Qoder(models::qoder::QoderAccount),
    Zcode(models::zcode::ZcodeAccount),
    Trae(models::trae::TraeAccount),
    Workbuddy(models::workbuddy::WorkbuddyAccount),
    Minimax(models::minimax_code::MinimaxCodeAccount),
}
