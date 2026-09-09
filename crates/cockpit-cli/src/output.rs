use serde::Serialize;
use tabled::{Table, Tabled};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Serialize)]
pub struct OutputEnvelope<T> {
    pub schema_version: u32,
    pub data: T,
}

impl<T> OutputEnvelope<T> {
    pub const fn new(data: T) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            data,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CommandOutput {
    pub provider: String,
    pub operation: String,
    pub status: String,
    pub message: String,
}

impl CommandOutput {
    pub fn new(
        provider: impl Into<String>,
        operation: impl Into<String>,
        status: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            provider: provider.into(),
            operation: operation.into(),
            status: status.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Serialize, Tabled)]
pub struct AccountDisplay {
    #[tabled(rename = "ID")]
    pub id: String,
    #[tabled(rename = "Email")]
    pub email: String,
    #[tabled(rename = "Plan")]
    pub plan: String,
    #[tabled(rename = "Tags")]
    pub tags: String,
}

pub fn print_accounts(accounts: Vec<AccountDisplay>) {
    if accounts.is_empty() {
        println!("No accounts found.");
    } else {
        println!("{}", Table::new(accounts));
    }
}

#[cfg(test)]
mod tests {
    use super::{AccountDisplay, OutputEnvelope, SCHEMA_VERSION};

    #[test]
    fn output_envelope_has_stable_schema_version() {
        let output = OutputEnvelope::new(vec![AccountDisplay {
            id: "account-1".into(),
            email: "user@example.com".into(),
            plan: "Plus".into(),
            tags: String::new(),
        }]);

        let value = serde_json::to_value(output).expect("output should serialize");
        assert_eq!(value["schema_version"], SCHEMA_VERSION);
        assert_eq!(value["data"][0]["id"], "account-1");
        assert!(value["data"][0].get("access_token").is_none());
        assert!(value["data"][0].get("refresh_token").is_none());
        assert!(value["data"][0].get("agent_private_key").is_none());
    }
}
