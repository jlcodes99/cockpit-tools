use clap::{Parser, Subcommand};
use cockpit_core::modules::{cursor_account, github_copilot_account};
use colored::*;
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(author, version, about = "Cockpit Tools CLI", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List accounts for a platform
    List {
        /// The platform (cursor, copilot)
        platform: String,
    },
    /// Switch accounts for a specific platform
    Switch {
        /// The platform (cursor, copilot)
        platform: String,
        /// The account ID or email to switch to
        account: String,
    },
    /// Show current quota for a platform
    Quota {
        /// The platform (cursor, copilot)
        platform: String,
    },
}

#[derive(Tabled)]
struct AccountDisplay {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Email")]
    email: String,
    #[tabled(rename = "Plan")]
    plan: String,
    #[tabled(rename = "Tags")]
    tags: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::List { platform }) => match platform.to_lowercase().as_str() {
            "cursor" => {
                let (accounts, warning) = cursor_account::list_accounts_for_cli().map_err(anyhow::Error::msg)?;
                if let Some(warning) = warning {
                    eprintln!("{} {warning}", "Warning:".yellow());
                }
                display_accounts(
                    accounts
                        .iter()
                        .map(|a| AccountDisplay {
                            id: a.id.clone(),
                            email: a.email.clone(),
                            plan: a.membership_type.clone().unwrap_or_default(),
                            tags: a.tags.as_ref().map(|t| t.join(", ")).unwrap_or_default(),
                        })
                        .collect(),
                );
            }
            "copilot" | "github_copilot" => {
                let (accounts, warning) = github_copilot_account::list_accounts_for_cli().map_err(anyhow::Error::msg)?;
                if let Some(warning) = warning {
                    eprintln!("{} {warning}", "Warning:".yellow());
                }
                display_accounts(
                    accounts
                        .iter()
                        .map(|a| AccountDisplay {
                            id: a.id.clone(),
                            email: a.github_email.clone().unwrap_or_default(),
                            plan: a.copilot_plan.clone().unwrap_or_default(),
                            tags: a.tags.as_ref().map(|t| t.join(", ")).unwrap_or_default(),
                        })
                        .collect(),
                );
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Switch { platform, account }) => match platform.to_lowercase().as_str() {
            "cursor" => {
                if let Err(e) = cursor_account::inject_to_cursor(&account) {
                    println!("{} {}", "Error:".red(), e);
                } else {
                    println!(
                        "{} Successfully switched Cursor account to {}",
                        "Success:".green(),
                        account
                    );
                }
            }
            "copilot" | "github_copilot" => {
                println!("{} GitHub Copilot switch is partially implemented in CLI. Use GUI for full instance sync.", "Info:".yellow());
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Quota { platform }) => match platform.to_lowercase().as_str() {
            _ => println!(
                "{} Quota command not yet implemented for {}",
                "Info:".yellow(),
                platform
            ),
        },
        None => {
            println!("Welcome to Cockpit CLI! Use --help for commands.");
        }
    }

    Ok(())
}

fn display_accounts(accounts: Vec<AccountDisplay>) {
    if accounts.is_empty() {
        println!("No accounts found.");
    } else {
        println!("{}", Table::new(accounts).to_string());
    }
}
