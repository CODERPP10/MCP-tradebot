//! `tradebot` — human control, out-of-band from the agent (DESIGN.md §2.9).
//!
//! Only `policy check` is wired up in `feat/workspace-skeleton`; the rest are
//! stubbed and land with `feat/operator-cli`.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use guardrails::Policy;

mod home;
mod init;
mod login;
mod prompt;

#[derive(Parser)]
#[command(
    name = "tradebot",
    version,
    about = "Operator control for the MCP tradebot"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Create the sealed store and prompt for credentials.
    Init {
        /// Data directory (overrides $TRADEBOT_HOME; default ./.tradebot).
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Print the Kite login URL and accept a pasted request token.
    Login {
        /// Data directory (overrides $TRADEBOT_HOME; default ./.tradebot).
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Token validity, kill-switch state, today's ledger aggregates.
    Status,
    /// Engage the kill switch (rejects everything mutating).
    Kill,
    /// Disengage the kill switch.
    Resume,
    /// Confirm a pending order by ref id.
    Confirm { ref_id: String },
    /// Reject a pending order by ref id.
    Reject { ref_id: String },
    /// Inspect the journal.
    Ledger {
        #[arg(long)]
        today: bool,
        #[arg(long)]
        pending: bool,
    },
    /// Force a reconciliation pass and print divergences.
    Reconcile,
    /// Guardrail policy commands.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
}

#[derive(Subcommand)]
enum PolicyAction {
    /// Validate the policy file and print the effective values.
    Check {
        /// Path to the policy TOML.
        #[arg(long, default_value = "policy.toml")]
        file: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { home } => match init::run(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("init failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Login { home } => match login::run(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("login failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Policy {
            action: PolicyAction::Check { file },
        } => match Policy::load(&file) {
            Ok(policy) => {
                println!("{} is valid.\n", file.display());
                println!("{}", policy.effective_summary());
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("policy check failed: {err}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!(
                "`{}` is not implemented yet (feat/operator-cli)",
                other.name()
            );
            ExitCode::FAILURE
        }
    }
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Command::Init { .. } => "init",
            Command::Login { .. } => "login",
            Command::Status => "status",
            Command::Kill => "kill",
            Command::Resume => "resume",
            Command::Confirm { .. } => "confirm",
            Command::Reject { .. } => "reject",
            Command::Ledger { .. } => "ledger",
            Command::Reconcile => "reconcile",
            Command::Policy { .. } => "policy",
        }
    }
}
