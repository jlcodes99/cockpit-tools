use anyhow::{bail, Context};
use clap::{Parser, Subcommand, ValueEnum};
use cockpit_core::modules::{codex_account, cursor_account, github_copilot_account};
use serde::Serialize;
use tabled::{Table, Tabled};

#[derive(Parser)]
#[command(author, version, about = "Cockpit Tools CLI")]
struct Cli {
    /// Emit machine-readable JSON on stdout for successful commands
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Clone, Copy, ValueEnum)]
enum Platform {
    Codex,
    Cursor,
    #[value(alias = "github_copilot", alias = "github-copilot")]
    Copilot,
}

impl Platform {
    fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
        }
    }
}

#[derive(Subcommand)]
enum Commands {
    /// List saved account IDs, email addresses, plans and tags
    List {
        #[arg(value_enum, ignore_case = true)]
        platform: Platform,
    },
    /// Select a saved Codex or Cursor account for its default client profile
    Switch {
        #[arg(value_enum, ignore_case = true)]
        platform: Platform,
        /// Exact account ID or a unique email address
        account: String,
    },
    /// Show the availability of quota queries for a platform
    Quota {
        #[arg(value_enum, ignore_case = true)]
        platform: Platform,
    },
}

#[derive(Serialize, Tabled)]
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

impl AccountDisplay {
    fn new(id: String, email: String, plan: Option<String>, tags: Option<Vec<String>>) -> Self {
        Self {
            id,
            email,
            plan: plan.unwrap_or_default(),
            tags: tags.unwrap_or_default().join(", "),
        }
    }
}

fn list_accounts(platform: Platform) -> anyhow::Result<Vec<AccountDisplay>> {
    let (accounts, warning) = match platform {
        Platform::Codex => (
            codex_account::list_accounts_checked()
                .map_err(anyhow::Error::msg)?
                .into_iter()
                .map(|a| AccountDisplay::new(a.id, a.email, a.plan_type, a.tags))
                .collect(),
            None,
        ),
        Platform::Cursor => {
            let (accounts, warning) =
                cursor_account::list_accounts_for_cli().map_err(anyhow::Error::msg)?;
            let rows = accounts
                .into_iter()
                .map(|a| AccountDisplay::new(a.id, a.email, a.membership_type, a.tags))
                .collect();
            (rows, warning)
        }
        Platform::Copilot => {
            let (accounts, warning) =
                github_copilot_account::list_accounts_for_cli().map_err(anyhow::Error::msg)?;
            let rows = accounts
                .into_iter()
                .map(|a| {
                    AccountDisplay::new(
                        a.id,
                        a.github_email.unwrap_or_default(),
                        a.copilot_plan,
                        a.tags,
                    )
                })
                .collect();
            (rows, warning)
        }
    };
    if let Some(warning) = warning {
        eprintln!("Warning: {warning}");
    }
    Ok(accounts)
}

fn resolve_account_id<'a>(
    accounts: &'a [AccountDisplay],
    selector: &str,
) -> anyhow::Result<&'a str> {
    let selector = selector.trim();
    if let Some(account) = accounts.iter().find(|account| account.id == selector) {
        return Ok(&account.id);
    }
    let mut matches = accounts.iter().filter(|account| {
        !account.email.is_empty() && account.email.eq_ignore_ascii_case(selector)
    });
    let account = matches
        .next()
        .context("Account not found. List the saved accounts and use an account ID.")?;
    if matches.next().is_some() {
        bail!("More than one account has that email address. Use an account ID.");
    }
    Ok(&account.id)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::List { platform }) => {
            let accounts = list_accounts(platform)?;
            if cli.json {
                println!("{}", serde_json::to_string(&accounts)?);
            } else if accounts.is_empty() {
                println!("No accounts found.");
            } else {
                println!("{}", Table::new(accounts));
            }
        }
        Some(Commands::Switch { platform, account }) => {
            if matches!(platform, Platform::Copilot) {
                bail!("GitHub Copilot switching requires the Cockpit Tools desktop app.");
            }
            let accounts = list_accounts(platform)?;
            let account_id = resolve_account_id(&accounts, &account)?;
            match platform {
                Platform::Codex => {
                    codex_account::switch_account_managed(account_id)
                        .await
                        .map_err(anyhow::Error::msg)?;
                }
                Platform::Cursor => {
                    cursor_account::inject_to_cursor(account_id).map_err(anyhow::Error::msg)?;
                }
                Platform::Copilot => unreachable!("Copilot switching is rejected before lookup"),
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "platform": platform.name(),
                        "accountId": account_id,
                    })
                );
            } else {
                println!(
                    "Selected {account_id} for the default {} profile.",
                    platform.name()
                );
            }
        }
        Some(Commands::Quota { platform }) => {
            bail!(
                "Quota queries for {} are not available in the CLI.",
                platform.name()
            );
        }
        None if cli.json => bail!("Specify a command for JSON output."),
        None => println!("Welcome to Cockpit CLI! Use --help for commands."),
    }
    Ok(())
}
