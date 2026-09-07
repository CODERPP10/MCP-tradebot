//! End-to-end test for `tradebot init`: drives the real binary with piped
//! stdin, then unseals the resulting store to check the records landed.

use std::io::Write;
use std::process::{Command, Stdio};

use secret_store::{SealedFileStore, SecretStore, KITE_API_KEY, KITE_API_SECRET};

fn run_init(home: &std::path::Path, stdin: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_tradebot"))
        .args(["init", "--home", home.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn init_creates_sealed_store_with_credentials() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");

    let out = run_init(
        &home,
        "hunter2hunter2\nhunter2hunter2\nmy_key\nmy_secret\n\n",
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let store = SealedFileStore::open(home.join("secrets.json"), "hunter2hunter2").unwrap();
    assert_eq!(store.get(KITE_API_KEY).unwrap(), "my_key");
    assert_eq!(store.get(KITE_API_SECRET).unwrap(), "my_secret");
}

#[test]
fn init_refuses_to_overwrite_existing_store() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");

    let first = run_init(&home, "hunter2hunter2\nhunter2hunter2\nk\ns\n\n");
    assert!(first.status.success());

    let second = run_init(&home, "hunter2hunter2\nhunter2hunter2\nk\ns\n\n");
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("already exists"));
}

#[test]
fn init_rejects_mismatched_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = run_init(&home, "hunter2hunter2\nDIFFERENT12345\nk\ns\n\n");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("did not match"));
}

#[test]
fn init_rejects_short_passphrase() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let out = run_init(&home, "short\nshort\nk\ns\n\n");
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("at least"));
}
