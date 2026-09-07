//! Live read-only probe against your real Zerodha account — a throwaway dev
//! tool, NOT part of the product. It exists only so `feat/kite-client` can be
//! exercised end-to-end before `secret-store` / `session` land.
//!
//! It reads the api key/secret and the login `request_token` from stdin prompts
//! (never env, never a file, never logged) and holds them in memory only. It
//! places NO orders — it calls `generate_session` then `profile` / `margins` /
//! `ltp` and prints them.
//!
//! Run:
//!   cargo run -p kite-client --example probe
//!
//! You will be asked for:
//!   1. API key      (from your Kite Connect app)
//!   2. API secret   (from your Kite Connect app)
//!   3. It prints a login URL — open it, log in, and copy the `request_token`
//!      query param from the redirect URL back into the prompt.

use std::io::{self, Write};

use kite_client::{KiteClient, KiteConfig};

fn prompt(label: &str) -> String {
    print!("{label}: ");
    io::stdout().flush().unwrap();
    let mut s = String::new();
    io::stdin().read_line(&mut s).unwrap();
    s.trim().to_string()
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let api_key = prompt("API key");
    let api_secret = prompt("API secret");

    println!(
        "\nOpen this URL, log in, then paste the `request_token` from the redirect:\n  \
         https://kite.zerodha.com/connect/login?v=3&api_key={api_key}\n"
    );
    let request_token = prompt("request_token");

    let client = KiteClient::new(KiteConfig::new(api_key))?;

    let session = client.generate_session(&request_token, &api_secret).await?;
    println!(
        "\n[session] user_id={} name={}",
        session.user_id, session.user_name
    );

    let profile = client.profile().await?;
    println!(
        "[profile] {} <{}> broker={}",
        profile.user_name, profile.email, profile.broker
    );

    let margins = client.margins_equity().await?;
    println!(
        "[margins] equity: available(live)={:.2} utilised(debits)={:.2} net={:.2}",
        margins.available.live_balance, margins.utilised.debits, margins.net
    );

    let ltp = client.ltp(&["NSE:INFY", "NSE:TCS"]).await?;
    for (sym, q) in &ltp {
        println!("[ltp] {sym} = {:.2}", q.last_price);
    }

    println!("\nOK — read-only probe complete. No orders were placed.");
    Ok(())
}
