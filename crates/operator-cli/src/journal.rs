//! `tradebot ledger [--today] [--pending] [--ref <id>]` — read the journal
//! (DESIGN.md §2.9). Pure local SQLite reads: no passphrase, no Kite call,
//! nothing mutating.

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use ledger::{Ledger, OrderRow};

use crate::home;

/// Default row count for the bare `tradebot ledger`.
const DEFAULT_LIMIT: u32 = 20;

pub fn run(
    explicit_home: Option<PathBuf>,
    today: bool,
    pending: bool,
    ref_id: Option<String>,
    limit: Option<u32>,
) -> Result<()> {
    let home = home::resolve(explicit_home);
    let db = home::ledger_db_path(&home);
    if !db.exists() {
        bail!(
            "no ledger at {} — nothing has run yet in this home",
            db.display()
        );
    }
    let ledger = Ledger::open(&db).context("opening the ledger")?;

    if let Some(ref_id) = ref_id {
        let row = ledger
            .order(&ref_id)
            .context("reading the order")?
            .with_context(|| format!("no such ref_id: {ref_id}"))?;
        print_detail(&row);
        return Ok(());
    }

    if pending {
        let rows = ledger
            .list_pending_confirms()
            .context("reading pending confirmations")?;
        if rows.is_empty() {
            println!("no pending confirmations");
        }
        for p in &rows {
            println!(
                "{}  {} {} {} @ {:.2}  requested {}",
                p.ref_id, p.side, p.quantity, p.tradingsymbol, p.limit_price, p.requested_at
            );
        }
        // `--pending` alongside `--today` prints both sections.
        if today {
            println!();
        } else {
            return Ok(());
        }
    }

    let rows = if today {
        ledger.orders_today().context("reading today's orders")?
    } else {
        ledger
            .recent_orders(limit.unwrap_or(DEFAULT_LIMIT))
            .context("reading recent orders")?
    };

    if rows.is_empty() {
        println!(
            "{}",
            if today {
                "no orders today"
            } else {
                "ledger is empty"
            }
        );
        return Ok(());
    }
    print_table(&rows);
    println!("\ndetail: tradebot ledger --ref <ref_id>");
    Ok(())
}

fn print_table(rows: &[OrderRow]) {
    println!(
        "{:<28} {:<20} {:<10} {:<5} {:>5} {:>10} {:<20} {:<12}",
        "REF_ID", "CREATED", "SYMBOL", "SIDE", "QTY", "LIMIT", "DECISION", "STATUS"
    );
    for r in rows {
        println!(
            "{:<28} {:<20} {:<10} {:<5} {:>5} {:>10.2} {:<20} {:<12}",
            r.ref_id,
            short_ts(&r.created_at),
            r.tradingsymbol,
            r.side,
            r.quantity,
            r.limit_price,
            r.decision.as_deref().unwrap_or("-"),
            effective_status(r),
        );
    }
}

fn print_detail(r: &OrderRow) {
    println!("ref_id            {}", r.ref_id);
    println!("created           {}", r.created_at);
    println!("source            {}", r.source);
    println!(
        "intent            {} {} {} ({} {}) limit {:.2}",
        r.side, r.quantity, r.tradingsymbol, r.exchange, r.product, r.limit_price
    );
    println!("order type        {}", r.order_type);
    println!(
        "decision          {}",
        r.decision.as_deref().unwrap_or("(none recorded)")
    );
    if let Some(reasons) = &r.reasons_json {
        println!("reasons           {reasons}");
    }
    match (&r.confirm_requested_at, &r.confirm_outcome) {
        (Some(at), Some(outcome)) => println!("confirmation      {outcome} (requested {at})"),
        (Some(at), None) => println!("confirmation      PENDING since {at}"),
        _ => println!("confirmation      not required"),
    }
    match &r.mode {
        Some(mode) => println!("submission        {mode}"),
        None => println!("submission        never submitted"),
    }
    if let Some(id) = &r.broker_order_id {
        println!("broker order id   {id}");
    }
    if let Some(err) = &r.submit_error {
        println!("submit error      {err}");
    }
    println!(
        "broker status     {}",
        r.status.as_deref().unwrap_or("(never observed)")
    );
    if let Some(filled) = r.filled_quantity {
        println!("filled            {filled} of {}", r.quantity);
    }
    if let Some(avg) = r.average_price {
        println!("average price     {avg:.2}");
    }
}

/// What the operator cares about in one column: the broker's last word if the
/// order reached the broker, otherwise where it stopped short.
fn effective_status(r: &OrderRow) -> String {
    if let Some(status) = &r.status {
        return status.clone();
    }
    if r.submit_error.is_some() {
        return "SUBMIT_FAILED".to_string();
    }
    if r.mode.is_some() {
        return "SUBMITTING".to_string();
    }
    if r.confirm_requested_at.is_some() && r.confirm_outcome.is_none() {
        return "AWAITING_CONFIRM".to_string();
    }
    "-".to_string()
}

/// `2026-09-12T14:03:11.123+05:30` → `2026-09-12 14:03:11`, so the table fits.
fn short_ts(ts: &str) -> String {
    let trimmed = ts.split('.').next().unwrap_or(ts);
    let trimmed = trimmed.split('+').next().unwrap_or(trimmed);
    trimmed.replacen('T', " ", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> OrderRow {
        OrderRow {
            ref_id: "01J".into(),
            created_at: "2026-09-12T14:03:11.123456+05:30".into(),
            source: "mcp".into(),
            tradingsymbol: "INFY".into(),
            exchange: "NSE".into(),
            side: "BUY".into(),
            quantity: 1,
            order_type: "LIMIT".into(),
            product: "CNC".into(),
            limit_price: 1500.0,
            decision: None,
            reasons_json: None,
            confirm_requested_at: None,
            confirm_outcome: None,
            mode: None,
            broker_order_id: None,
            submit_error: None,
            status: None,
            filled_quantity: None,
            average_price: None,
        }
    }

    #[test]
    fn short_ts_strips_subseconds_and_offset() {
        assert_eq!(
            short_ts("2026-09-12T14:03:11.123+05:30"),
            "2026-09-12 14:03:11"
        );
        assert_eq!(short_ts("2026-09-12T14:03:11+05:30"), "2026-09-12 14:03:11");
        assert_eq!(short_ts("whatever"), "whatever");
    }

    #[test]
    fn effective_status_prefers_the_brokers_last_word() {
        let mut r = row();
        r.mode = Some("paper".into());
        r.status = Some("COMPLETE".into());
        r.submit_error = Some("ignored".into());
        assert_eq!(effective_status(&r), "COMPLETE");
    }

    #[test]
    fn effective_status_falls_back_through_the_lifecycle() {
        assert_eq!(effective_status(&row()), "-");

        let mut awaiting = row();
        awaiting.confirm_requested_at = Some("2026-09-12T14:03:11+05:30".into());
        assert_eq!(effective_status(&awaiting), "AWAITING_CONFIRM");

        let mut submitting = awaiting.clone();
        submitting.confirm_outcome = Some("APPROVED".into());
        submitting.mode = Some("paper".into());
        assert_eq!(effective_status(&submitting), "SUBMITTING");

        let mut failed = submitting.clone();
        failed.submit_error = Some("network".into());
        assert_eq!(effective_status(&failed), "SUBMIT_FAILED");
    }
}
