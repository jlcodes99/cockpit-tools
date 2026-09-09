mod cli;
mod output;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, Platform};
use cockpit_core::modules::{
    codex_account, codex_instance, cursor_account, github_copilot_account,
};
use output::{
    AccountDisplay, CodexAccountDisplay, CodexCurrentDisplay, CodexInstanceDisplay, CommandOutput,
    OutputEnvelope,
};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::List { platform }) => {
            list_accounts(platform, cli.json)?;
        }
        Some(Commands::Switch { platform, account }) => {
            switch_account(platform, &account, cli.json)?;
        }
        Some(Commands::Quota { platform, account }) => {
            show_quota_status(platform, account.as_deref(), cli.json)?;
        }
        Some(Commands::Current { platform }) => {
            show_current_account(platform, cli.json)?;
        }
        Some(Commands::Instances { platform }) => {
            list_instances(platform, cli.json)?;
        }
        None => {
            if cli.json {
                print_json(&OutputEnvelope::new(CommandOutput::new(
                    "root",
                    "help",
                    "ok",
                    "Use --help for available commands.",
                )))?;
            } else {
                println!("Welcome to Cockpit CLI! Use --help for commands.");
            }
        }
    }

    Ok(())
}

fn list_accounts(platform: Platform, json: bool) -> Result<()> {
    if platform == Platform::Codex {
        return list_codex_accounts(json);
    }

    let accounts = match platform {
        Platform::Cursor => cursor_account::list_accounts()
            .iter()
            .map(|account| AccountDisplay {
                id: account.id.clone(),
                email: account.email.clone(),
                plan: account.membership_type.clone().unwrap_or_default(),
                tags: account
                    .tags
                    .as_ref()
                    .map(|tags| tags.join(", "))
                    .unwrap_or_default(),
            })
            .collect(),
        Platform::Copilot => github_copilot_account::list_accounts()
            .iter()
            .map(|account| AccountDisplay {
                id: account.id.clone(),
                email: account.github_email.clone().unwrap_or_default(),
                plan: account.copilot_plan.clone().unwrap_or_default(),
                tags: account
                    .tags
                    .as_ref()
                    .map(|tags| tags.join(", "))
                    .unwrap_or_default(),
            })
            .collect(),
        Platform::Codex => unreachable!("Codex accounts are handled above"),
    };

    if json {
        print_json(&OutputEnvelope::new(accounts))
    } else {
        output::print_accounts(accounts);
        Ok(())
    }
}

fn switch_account(platform: Platform, account: &str, json: bool) -> Result<()> {
    match platform {
        Platform::Cursor => {
            cursor_account::inject_to_cursor(account)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            print_operation(
                json,
                CommandOutput::new(
                    "cursor",
                    "switch",
                    "ok",
                    format!("Successfully switched Cursor account to {account}"),
                ),
            )?;
        }
        Platform::Copilot => {
            print_operation(
                json,
                CommandOutput::new(
                    "copilot",
                    "switch",
                    "unsupported",
                    "GitHub Copilot switch is partially implemented in CLI. Use GUI for full instance sync.",
                ),
            )?;
        }
        Platform::Codex => {
            print_operation(
                json,
                CommandOutput::new(
                    "codex",
                    "switch",
                    "unsupported",
                    "Codex account selection is instance-scoped; use an instance binding instead of switching global credentials.",
                ),
            )?;
        }
    }

    Ok(())
}

fn show_quota_status(platform: Platform, account: Option<&str>, json: bool) -> Result<()> {
    if platform == Platform::Codex {
        return show_codex_quota(account, json);
    }

    let provider = platform.as_str();
    print_operation(
        json,
        CommandOutput::new(
            provider,
            "quota",
            "not_implemented",
            format!("Quota command is not yet implemented for {provider}"),
        ),
    )
}

fn list_codex_accounts(json: bool) -> Result<()> {
    let accounts = codex_account::list_accounts_read_only().map_err(anyhow::Error::msg)?;
    let displays = accounts
        .iter()
        .map(CodexAccountDisplay::from)
        .collect::<Vec<_>>();

    if json {
        print_json(&OutputEnvelope::new(displays))
    } else {
        output::print_codex_accounts(&displays);
        Ok(())
    }
}

fn show_current_account(platform: Platform, json: bool) -> Result<()> {
    if platform != Platform::Codex {
        return print_operation(
            json,
            CommandOutput::new(
                platform.as_str(),
                "current",
                "unsupported",
                format!(
                    "Current account command is not yet implemented for {}",
                    platform.as_str()
                ),
            ),
        );
    }

    let account = codex_account::get_current_account_read_only().map_err(anyhow::Error::msg)?;
    let display = account.as_ref().map(CodexCurrentDisplay::from);

    if json {
        print_json(&OutputEnvelope::new(display))
    } else {
        output::print_codex_current(display.as_ref());
        Ok(())
    }
}

fn show_codex_quota(account: Option<&str>, json: bool) -> Result<()> {
    let accounts = match account {
        Some(selector) => vec![codex_account::resolve_account_read_only(selector)
            .map_err(anyhow::Error::msg)?
            .ok_or_else(|| anyhow::anyhow!("Codex account not found: {selector}"))?],
        None => codex_account::list_accounts_read_only().map_err(anyhow::Error::msg)?,
    };
    let displays = accounts
        .iter()
        .map(CodexAccountDisplay::from)
        .collect::<Vec<_>>();

    if json {
        print_json(&OutputEnvelope::new(displays))
    } else {
        output::print_codex_quota(&displays);
        Ok(())
    }
}

fn list_instances(platform: Platform, json: bool) -> Result<()> {
    if platform != Platform::Codex {
        return print_operation(
            json,
            CommandOutput::new(
                platform.as_str(),
                "instances",
                "unsupported",
                format!(
                    "Instance command is not yet implemented for {}",
                    platform.as_str()
                ),
            ),
        );
    }

    let store = codex_instance::load_instance_store().map_err(anyhow::Error::msg)?;
    let default_dir = codex_instance::get_default_codex_home().map_err(anyhow::Error::msg)?;
    let mut displays = store
        .instances
        .iter()
        .map(CodexInstanceDisplay::from)
        .collect::<Vec<_>>();
    displays.push(output::codex_default_instance_display(
        &default_dir,
        &store.default_settings,
    ));

    if json {
        print_json(&OutputEnvelope::new(displays))
    } else {
        output::print_codex_instances(&displays);
        Ok(())
    }
}

fn print_operation(json: bool, output: CommandOutput) -> Result<()> {
    if json {
        print_json(&OutputEnvelope::new(output))
    } else if output.status == "ok" {
        println!("Success: {}", output.message);
        Ok(())
    } else {
        println!("Info: {}", output.message);
        Ok(())
    }
}

fn print_json<T: serde::Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
