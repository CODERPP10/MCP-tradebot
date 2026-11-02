//! Small stdin helpers shared by the interactive subcommands. Hidden input is
//! not echoed on a real terminal; when stdin is a pipe (scripts, CI) a plain
//! line is read so the commands stay automatable.

use std::io::{self, IsTerminal, Write};

use anyhow::{bail, Context, Result};

/// Read one line. Returns an error on empty input or EOF.
pub fn read_line(label: &str) -> Result<String> {
    let s = read_line_raw(label)?;
    if s.is_empty() {
        bail!("{label} must not be empty");
    }
    Ok(s)
}

/// Read one line, trimmed. Empty is allowed; EOF is an error.
pub fn read_line_raw(label: &str) -> Result<String> {
    if io::stdin().is_terminal() {
        print!("{label}: ");
        io::stdout().flush()?;
    }
    let mut s = String::new();
    if io::stdin().read_line(&mut s).context("reading stdin")? == 0 {
        bail!("unexpected end of input while reading {label}");
    }
    Ok(s.trim().to_string())
}

/// Read one hidden value (no echo on a tty; plain line otherwise).
pub fn read_hidden(label: &str) -> Result<String> {
    if io::stdin().is_terminal() {
        Ok(rpassword::prompt_password(format!("{label}: "))?)
    } else {
        read_line_raw(label)
    }
}
