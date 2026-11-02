//! `tradebot` — human control, out-of-band from the agent (DESIGN.md §2.9).
//!
//! Every subcommand is wired: the `match` in `main` is exhaustive with no
//! "not implemented" fallback, so adding a `Command` variant without an arm
//! is a compile error rather than a runtime surprise.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use guardrails::Policy;

mod confirm;
mod home;
mod init;
mod journal;
mod kill;
mod login;
mod prompt;
mod reconcile;
mod secrets;
mod status;
mod whoami;

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
    /// Verify the sealed session with two read-only Kite calls.
    Whoami {
        /// Data directory (overrides $TRADEBOT_HOME; default ./.tradebot).
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Token validity, kill-switch state, today's ledger aggregates.
    Status {
        /// Data directory (overrides $TRADEBOT_HOME; default ./.tradebot).
        #[arg(long)]
        home: Option<PathBuf>,
        /// Skip the sealed-store token check (no passphrase prompt).
        #[arg(long)]
        no_session: bool,
    },
    /// Engage the kill switch (rejects everything mutating).
    Kill {
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Disengage the kill switch.
    Resume {
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Confirm a pending order by ref id.
    Confirm {
        ref_id: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Reject a pending order by ref id.
    Reject {
        ref_id: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Inspect the journal.
    Ledger {
        /// Only intents created today (IST), oldest first.
        #[arg(long)]
        today: bool,
        /// Confirmations still awaiting a human decision.
        #[arg(long)]
        pending: bool,
        /// Full lifecycle of one order.
        #[arg(long = "ref", value_name = "REF_ID")]
        ref_id: Option<String>,
        /// Row cap for the default (most-recent-first) listing.
        #[arg(long)]
        limit: Option<u32>,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Force a reconciliation pass and print divergences.
    Reconcile {
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// Guardrail policy commands.
    Policy {
        #[command(subcommand)]
        action: PolicyAction,
    },
    /// Seal an additional record into an existing store (e.g. Telegram
    /// credentials, without a destructive re-`init`).
    Secrets {
        #[command(subcommand)]
        action: SecretsAction,
    },
}

#[derive(Subcommand)]
enum SecretsAction {
    /// Seal a value under `key` (prompts, hidden input). Known keys:
    /// telegram_bot_token, telegram_chat_id, kite_api_key, kite_api_secret.
    Put {
        key: String,
        #[arg(long)]
        home: Option<PathBuf>,
    },
    /// List the record keys currently sealed (values are never shown).
    List {
        #[arg(long)]
        home: Option<PathBuf>,
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
        Command::Whoami { home } => match whoami::run(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("whoami failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Status { home, no_session } => match status::run(home, no_session) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("status failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Ledger {
            today,
            pending,
            ref_id,
            limit,
            home,
        } => match journal::run(home, today, pending, ref_id, limit) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ledger failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Reconcile { home } => match reconcile::run(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("reconcile failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Kill { home } => match kill::kill(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("kill failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Resume { home } => match kill::resume(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("resume failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Confirm { ref_id, home } => match confirm::confirm(home, &ref_id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("confirm failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Reject { ref_id, home } => match confirm::reject(home, &ref_id) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("reject failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Secrets {
            action: SecretsAction::Put { key, home },
        } => match secrets::put(home, &key) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("secrets put failed: {err:#}");
                ExitCode::FAILURE
            }
        },
        Command::Secrets {
            action: SecretsAction::List { home },
        } => match secrets::list(home) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("secrets list failed: {err:#}");
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
    }
}
