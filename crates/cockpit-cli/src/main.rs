use clap::{Parser, Subcommand};
use cockpit_core::modules::{cursor_account, github_copilot_account};
use antigravity_cockpit_tools_lib::modules::{
    account as antigravity_account, antigravity_cli, provider_current_state,
};
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
        /// The platform (cursor, copilot, antigravity-cli, agy)
        platform: String,
    },
    /// Switch accounts for a specific platform
    Switch {
        /// The platform (cursor, copilot, antigravity-cli, agy)
        platform: String,
        /// The account ID or email to switch to
        account: String,
    },
    /// Show current account and status for a platform
    Current {
        /// The platform (antigravity-cli, agy)
        platform: String,
    },
    /// Show current quota for a platform
    Quota {
        /// The platform (cursor, copilot)
        platform: String,
    },
    /// Run CLI tool for a platform
    Run {
        /// The platform (antigravity-cli, agy)
        platform: String,
        /// Account ID or email to switch to before running (optional)
        #[arg(short, long)]
        account: Option<String>,
        /// Arguments forwarded to the platform binary
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Tabled)]
struct AccountDisplay {
    #[tabled(rename = "ID")]
    id: String,
    #[tabled(rename = "Email")]
    email: String,
    #[tabled(rename = "Plan/Tier")]
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
                let accounts = cursor_account::list_accounts();
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
                let accounts = github_copilot_account::list_accounts();
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
            "antigravity-cli" | "antigravity_cli" | "agy" => {
                let accounts = antigravity_account::list_accounts().unwrap_or_default();
                display_accounts(
                    accounts
                        .iter()
                        .map(|a| AccountDisplay {
                            id: a.id.clone(),
                            email: a.email.clone(),
                            plan: a
                                .quota
                                .as_ref()
                                .and_then(|q| q.subscription_tier.clone())
                                .unwrap_or_else(|| "FREE".to_string()),
                            tags: a.tags.join(", "),
                        })
                        .collect(),
                );
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Current { platform }) => match platform.to_lowercase().as_str() {
            "antigravity-cli" | "antigravity_cli" | "agy" => {
                let status = match antigravity_cli::get_antigravity_cli_status() {
                    Ok(s) => s,
                    Err(e) => {
                        println!("{} Failed to get CLI status: {}", "Error:".red(), e);
                        return Ok(());
                    }
                };
                println!("{} Antigravity CLI Status:", "Info:".cyan());
                println!(
                    "  Installed:    {}",
                    if status.installed {
                        "Yes".green()
                    } else {
                        "No".red()
                    }
                );
                println!(
                    "  Binary Path:  {}",
                    status.executable_path.as_deref().unwrap_or("Not found")
                );
                println!(
                    "  Version:      {}",
                    status.version.as_deref().unwrap_or("Unknown")
                );
                println!("  Auth Backend: {}", status.auth_backend);
                if let Some(email) = &status.current_email {
                    println!(
                        "  Current Acct: {} ({})",
                        email.green(),
                        status.current_account_id.as_deref().unwrap_or("")
                    );
                } else {
                    println!(
                        "  Current Acct: {}",
                        "None (no account bound to CLI)".yellow()
                    );
                }
                if let Some(diag) = &status.diagnostic {
                    println!("  Diagnostic:   {}", diag.yellow());
                }
            }
            _ => println!("{} Current command not supported for platform: {}", "Error:".red(), platform),
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
                println!(
                    "{} GitHub Copilot switch is partially implemented in CLI. Use GUI for full instance sync.",
                    "Info:".yellow()
                );
            }
            "antigravity-cli" | "antigravity_cli" | "agy" => {
                let accounts = antigravity_account::list_accounts().unwrap_or_default();
                let target = accounts.iter().find(|a| {
                    a.id == account || a.email.eq_ignore_ascii_case(&account)
                });
                let target = match target {
                    Some(a) => a,
                    None => {
                        println!("{} Antigravity account not found: {}", "Error:".red(), account);
                        return Ok(());
                    }
                };

                match antigravity_cli::switch_account_transaction(&target.id).await {
                    Ok(switched) => {
                        println!(
                            "{} Successfully switched Antigravity CLI account to {} ({})",
                            "Success:".green(),
                            switched.email,
                            switched.id
                        );
                    }
                    Err(e) => {
                        println!("{} Failed to switch Antigravity CLI account: {}", "Error:".red(), e);
                    }
                }
            }
            _ => println!("{} Unknown platform: {}", "Error:".red(), platform),
        },
        Some(Commands::Run { platform, account, args }) => match platform.to_lowercase().as_str() {
            "antigravity-cli" | "antigravity_cli" | "agy" => {
                let target_id = if let Some(acc) = account {
                    let accounts = antigravity_account::list_accounts().unwrap_or_default();
                    let found = accounts.iter().find(|a| {
                        a.id == acc || a.email.eq_ignore_ascii_case(&acc)
                    });
                    match found {
                        Some(a) => a.id.clone(),
                        None => {
                            println!("{} Antigravity account not found: {}", "Error:".red(), acc);
                            return Ok(());
                        }
                    }
                } else {
                    match provider_current_state::get_current_account_id("antigravity_cli").ok().flatten() {
                        Some(id) => id,
                        None => {
                            println!(
                                "{} No current Antigravity CLI account selected. Please specify --account or switch first.",
                                "Error:".red()
                            );
                            return Ok(());
                        }
                    }
                };

                let options = antigravity_cli::AntigravityCliRunOptions {
                    cwd: None,
                    args: if args.is_empty() { None } else { Some(args) },
                    terminal: Some("direct".to_string()),
                };

                match antigravity_cli::run_antigravity_cli(&target_id, Some(options)).await {
                    Ok(res) => {
                        println!(
                            "{} Antigravity CLI launched with account {} ({})",
                            "Success:".green(),
                            res.email,
                            res.account_id
                        );
                    }
                    Err(e) => {
                        println!("{} Failed to run Antigravity CLI: {}", "Error:".red(), e);
                    }
                }
            }
            _ => println!("{} Run command not supported for platform: {}", "Error:".red(), platform),
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
