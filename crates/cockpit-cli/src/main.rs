mod cli;
mod output;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands, Platform};
use cockpit_core::modules::{cursor_account, github_copilot_account};
use output::{AccountDisplay, CommandOutput, OutputEnvelope};

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
        Some(Commands::Quota { platform }) => {
            show_quota_status(platform, cli.json)?;
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
    }

    Ok(())
}

fn show_quota_status(platform: Platform, json: bool) -> Result<()> {
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
