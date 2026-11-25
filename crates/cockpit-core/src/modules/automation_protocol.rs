//! Versioned account automation commands used by the desktop host and CLI.

use serde::{Deserialize, Serialize};
use serde_json::Value;

macro_rules! platforms {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
        pub enum AccountPlatform {
            $(#[serde(rename = $name)] $variant),+
        }

        impl AccountPlatform {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $name),+ }
            }
        }
    };
}

platforms! {
    Antigravity => "antigravity",
    AntigravityIde => "antigravity_ide",
    Codex => "codex",
    Claude => "claude_manager",
    Zed => "zed",
    Copilot => "github-copilot",
    Windsurf => "windsurf",
    Kiro => "kiro",
    Cursor => "cursor",
    Grok => "grok",
    Codebuddy => "codebuddy",
    CodebuddyCn => "codebuddy_cn",
    Qoder => "qoder",
    Zcode => "zcode",
    Trae => "trae",
    TraeSolo => "trae_solo",
    TraeCn => "trae_cn",
    TraeSoloCn => "trae_solo_cn",
    Workbuddy => "workbuddy",
    WorkbuddyAi => "workbuddy_ai",
    Minimax => "minimaxcode",
}

pub const VERSION: u32 = 1;
pub const REQUEST_TYPE: &str = "request.automation";
pub const RESPONSE_TYPE: &str = "response.automation";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(
    tag = "operation",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum Operation {
    Platforms,
    List {
        platform: AccountPlatform,
    },
    Quota {
        platform: AccountPlatform,
        account_id: String,
        #[serde(default)]
        refresh: bool,
    },
    Switch {
        platform: AccountPlatform,
        account_id: String,
    },
    ApiMembers,
    ApiAdd {
        account_ids: Vec<String>,
    },
    ApiRemove {
        account_ids: Vec<String>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub version: u32,
    pub request_id: String,
    pub auth_token: String,
    pub command: Operation,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Request")
            .field("version", &self.version)
            .field("request_id", &self.request_id)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reply {
    pub version: u32,
    pub request_id: String,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Ok { data: Value },
    Error { code: String, message: String },
}
