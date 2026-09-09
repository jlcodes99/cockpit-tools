use clap::{Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(author, version, about = "Cockpit Tools CLI", long_about = None)]
pub struct Cli {
    /// Emit machine-readable JSON instead of human-readable output.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// List accounts for a platform.
    List {
        /// The platform.
        #[arg(ignore_case = true)]
        platform: Platform,
    },
    /// Switch accounts for a specific platform.
    Switch {
        /// The platform.
        #[arg(ignore_case = true)]
        platform: Platform,
        /// The account ID or email to switch to.
        account: String,
    },
    /// Show current quota for a platform.
    Quota {
        /// The platform.
        #[arg(ignore_case = true)]
        platform: Platform,
        /// Optional account ID or email for a single-account snapshot.
        account: Option<String>,
    },
    /// Show the current account for a platform.
    Current {
        /// The platform.
        #[arg(ignore_case = true)]
        platform: Platform,
    },
    /// List persisted instances for a platform.
    Instances {
        /// The platform.
        #[arg(ignore_case = true)]
        platform: Platform,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum Platform {
    Cursor,
    #[value(alias = "github-copilot", alias = "github_copilot")]
    Copilot,
    Codex,
}

impl Platform {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cursor => "cursor",
            Self::Copilot => "copilot",
            Self::Codex => "codex",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Commands, Platform};
    use clap::Parser;

    #[test]
    fn parses_global_json_before_command() {
        let cli = Cli::try_parse_from(["cockpit-cli", "--json", "list", "cursor"])
            .expect("CLI arguments should parse");

        assert!(cli.json);
        assert!(matches!(
            cli.command,
            Some(Commands::List {
                platform: Platform::Cursor
            })
        ));
    }

    #[test]
    fn preserves_copilot_platform_aliases() {
        for alias in ["copilot", "github-copilot", "github_copilot"] {
            let cli = Cli::try_parse_from(["cockpit-cli", "list", alias])
                .expect("Copilot alias should parse");
            assert!(matches!(
                cli.command,
                Some(Commands::List {
                    platform: Platform::Copilot
                })
            ));
        }
    }
}
