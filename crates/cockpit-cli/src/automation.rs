use anyhow::{anyhow, Context};
use clap::Subcommand;
use cockpit_core::modules::{
    automation_protocol::{
        AccountPlatform, Operation, Outcome, Reply, Request, REQUEST_TYPE, RESPONSE_TYPE, VERSION,
    },
    config, i18n,
};
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

#[derive(Subcommand)]
pub enum AccountCommand {
    Platforms,
    List {
        platform: String,
    },
    Quota {
        platform: String,
        account_id: String,
        #[arg(long)]
        refresh: bool,
    },
    Switch {
        platform: String,
        account_id: String,
    },
}

#[derive(Subcommand)]
pub enum ApiServiceCommand {
    Members,
    Add {
        #[arg(required = true)]
        account_ids: Vec<String>,
    },
    Remove {
        #[arg(required = true)]
        account_ids: Vec<String>,
    },
}

#[derive(Deserialize)]
struct ServerStatus {
    ws_port: u16,
    #[serde(default)]
    auth_token: String,
    #[serde(default)]
    automation_version: u32,
}

fn message(key: &str) -> String {
    let locale = config::get_user_config().language;
    i18n::translate(&locale, key, &[])
}

fn parse_platform(value: &str) -> anyhow::Result<AccountPlatform> {
    AccountPlatform::ALL
        .iter()
        .copied()
        .find(|platform| platform.as_str() == value)
        .ok_or_else(|| {
            anyhow!(
                "{} ({})",
                message("common.automation.invalidPlatform"),
                value
            )
        })
}

pub async fn run_account(command: AccountCommand, json: bool) -> anyhow::Result<()> {
    let command = match command {
        AccountCommand::Platforms => {
            let platforms: Vec<_> = AccountPlatform::ALL
                .iter()
                .map(|platform| platform.as_str())
                .collect();
            print_data(serde_json::to_value(platforms)?, json)?;
            return Ok(());
        }
        AccountCommand::List { platform } => Operation::List {
            platform: parse_platform(&platform)?,
        },
        AccountCommand::Quota {
            platform,
            account_id,
            refresh,
        } => Operation::Quota {
            platform: parse_platform(&platform)?,
            account_id,
            refresh,
        },
        AccountCommand::Switch {
            platform,
            account_id,
        } => Operation::Switch {
            platform: parse_platform(&platform)?,
            account_id,
        },
    };
    run(command, json).await
}

pub async fn run_api_service(command: ApiServiceCommand, json: bool) -> anyhow::Result<()> {
    let command = match command {
        ApiServiceCommand::Members => Operation::ApiMembers,
        ApiServiceCommand::Add { account_ids } => Operation::ApiAdd { account_ids },
        ApiServiceCommand::Remove { account_ids } => Operation::ApiRemove { account_ids },
    };
    run(command, json).await
}

fn print_data(data: Value, json: bool) -> anyhow::Result<()> {
    let output = if json {
        serde_json::to_string(&data)?
    } else {
        serde_json::to_string_pretty(&data)?
    };
    println!("{output}");
    Ok(())
}

async fn run(command: Operation, json_output: bool) -> anyhow::Result<()> {
    let status_path = config::get_server_status_path().map_err(anyhow::Error::msg)?;
    let status =
        std::fs::read(status_path).with_context(|| message("common.automation.hostUnavailable"))?;
    let server: ServerStatus = serde_json::from_slice(&status)
        .with_context(|| message("common.automation.hostUnavailable"))?;

    match request(server, command).await? {
        Outcome::Ok { data } => print_data(data, json_output),
        Outcome::Error { code, message } => Err(anyhow!("{code}: {message}")),
    }
}

async fn request(server: ServerStatus, command: Operation) -> anyhow::Result<Outcome> {
    if server.automation_version != VERSION {
        return Err(anyhow!(
            "version_mismatch: {}",
            message("common.automation.versionMismatch")
        ));
    }
    if server.auth_token.is_empty() {
        return Err(anyhow!(
            "unauthorized: {}",
            message("common.automation.unauthorized")
        ));
    }
    let url = format!("ws://127.0.0.1:{}", server.ws_port);
    let (mut socket, _) = timeout(Duration::from_secs(5), connect_async(url))
        .await
        .context("AUTOMATION_CONNECT_TIMEOUT")?
        .with_context(|| message("common.automation.hostUnavailable"))?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let request = Request {
        version: VERSION,
        request_id: request_id.clone(),
        auth_token: server.auth_token,
        command,
    };
    let request = json!({ "type": REQUEST_TYPE, "payload": request }).to_string();

    timeout(Duration::from_secs(120), async {
        socket.send(Message::Text(request.into())).await?;
        while let Some(message) = socket.next().await {
            let Message::Text(text) = message? else {
                continue;
            };
            let event: Value = serde_json::from_str(&text)?;
            if event["type"] != RESPONSE_TYPE || event["payload"]["requestId"] != request_id {
                continue;
            }
            let reply: Reply = serde_json::from_value(event["payload"].clone())?;
            if reply.version != VERSION {
                return Err(anyhow!("AUTOMATION_VERSION_MISMATCH"));
            }
            return Ok(reply.outcome);
        }
        Err(anyhow!("AUTOMATION_DISCONNECTED"))
    })
    .await
    .with_context(|| message("common.automation.responseTimeout"))?
}
