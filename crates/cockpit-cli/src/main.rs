use clap::{Parser, Subcommand};
use cockpit_core::modules::capacity_snapshot::{self, RouteCapacity};
use cockpit_core::modules::{cursor_account, github_copilot_account};
use colored::*;
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(
    name = "cockpit",
    bin_name = "cockpit",
    author,
    version,
    about = "Cockpit Tools CLI",
    long_about = None
)]
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
    /// Show current quota as a sanitized capacity snapshot
    Quota {
        /// Optional platform filter (antigravity, codex). Omit for all.
        platform: Option<String>,
        /// Output machine-readable sanitized capacity snapshot JSON
        #[arg(long)]
        json: bool,
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
        Some(Commands::Quota { platform, json }) => {
            let mut snapshot = capacity_snapshot::build_capacity_snapshot();
            match platform.as_deref().map(str::to_lowercase).as_deref() {
                None | Some("all") => {}
                Some(filter) => {
                    let supported = ["antigravity", "codex"];
                    if !supported.contains(&filter) {
                        if json {
                            snapshot.routes.clear();
                            snapshot.availability =
                                capacity_snapshot::SnapshotAvailability::Unavailable;
                            snapshot.sources.clear();
                            println!("{}", serde_json::to_string_pretty(&snapshot)?);
                        } else {
                            println!(
                                "{} Unknown or unsupported platform: {} (supported: {})",
                                "Error:".red(),
                                filter,
                                supported.join(", ")
                            );
                        }
                        return Ok(());
                    }
                    snapshot.routes.retain(|r| r.provider == filter);
                }
            };

            if json {
                println!("{}", serde_json::to_string_pretty(&snapshot)?);
            } else {
                display_routes(&snapshot.routes);
            }
        }
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

#[derive(Tabled)]
struct RouteDisplay {
    #[tabled(rename = "Route")]
    route_id: String,
    #[tabled(rename = "Provider")]
    provider: String,
    #[tabled(rename = "Plan")]
    plan: String,
    #[tabled(rename = "Health")]
    health: String,
    #[tabled(rename = "Min remaining")]
    min_remaining: String,
    #[tabled(rename = "Updated")]
    updated_at: String,
}

fn display_routes(routes: &[RouteCapacity]) {
    if routes.is_empty() {
        println!("No capacity routes found.");
        return;
    }
    let rows: Vec<RouteDisplay> = routes
        .iter()
        .map(|r| RouteDisplay {
            route_id: r.route_id.clone(),
            provider: r.provider.clone(),
            plan: r.plan.clone().unwrap_or_else(|| "-".to_string()),
            health: format!("{:?}", r.health.status).to_lowercase(),
            min_remaining: r
                .quota_windows
                .iter()
                .map(|w| w.remaining_ratio)
                .fold(None::<f64>, |acc, v| match acc {
                    Some(cur) => Some(cur.min(v)),
                    None => Some(v),
                })
                .map(|v| format!("{}%", (v * 100.0).round() as i64))
                .unwrap_or_else(|| "-".to_string()),
            updated_at: r
                .updated_at
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .unwrap_or_else(|| "-".to_string()),
        })
        .collect();
    println!("{}", Table::new(rows).to_string());
}
