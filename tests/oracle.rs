//! Compare kakeibo against the real `hledger` binary. Skipped when hledger is not installed.

use kakeibo::analysis::balance::balances_as_of;
use kakeibo::ledger::model::Item;
use kakeibo::ledger::{ops, parser, writer};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;

fn hledger_available() -> bool {
  let ok = Command::new("hledger")
    .arg("--version")
    .output()
    .is_ok_and(|o| o.status.success());
  if !ok {
    eprintln!("hledger not found, skipping oracle test");
  }
  ok
}

fn hledger_balances(path: &Path) -> BTreeMap<String, Decimal> {
  let out = Command::new("hledger")
    .args(["bal", "--flat", "-N", "-O", "csv", "-f"])
    .arg(path)
    .output()
    .unwrap();
  assert!(
    out.status.success(),
    "{}",
    String::from_utf8_lossy(&out.stderr)
  );
  let csv = String::from_utf8(out.stdout).unwrap();
  csv
    .lines()
    .skip(1)
    .map(|line| {
      let (acct, amt) = line.split_once("\",\"").unwrap();
      let mut amt = amt.trim_end_matches('"');
      let a = parser::amount(&mut amt).unwrap();
      (acct.trim_start_matches('"').to_string(), a.quantity)
    })
    .collect()
}

fn ours(text: &str) -> BTreeMap<String, Decimal> {
  let j = parser::parse(text, None).unwrap();
  balances_as_of(&j, None)
    .into_iter()
    .filter(|(_, q)| !q.is_zero())
    .collect()
}

fn hledger_check(path: &Path) -> bool {
  Command::new("hledger")
    .arg("check")
    .arg("-f")
    .arg(path)
    .output()
    .unwrap()
    .status
    .success()
}

fn fixtures() -> Vec<std::path::PathBuf> {
  let mut v: Vec<_> = fs::read_dir("tests/fixtures")
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "journal"))
    .collect();
  v.sort();
  v
}

#[test]
fn balances_match_hledger() {
  if !hledger_available() {
    return;
  }
  for path in fixtures() {
    if !hledger_check(&path) {
      continue;
    }
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(ours(&text), hledger_balances(&path), "{}", path.display());
  }
}

#[test]
fn check_matches_hledger() {
  if !hledger_available() {
    return;
  }
  for path in fixtures() {
    let text = fs::read_to_string(&path).unwrap();
    let ours_ok = parser::parse(&text, None).is_ok_and(|j| ops::check(&j).is_empty());
    assert_eq!(ours_ok, hledger_check(&path), "{}", path.display());
  }
}

#[test]
fn regenerated_journal_is_valid_hledger() {
  if !hledger_available() {
    return;
  }
  let dir = tempfile::tempdir().unwrap();
  for path in fixtures() {
    let text = fs::read_to_string(&path).unwrap();
    let mut journal = parser::parse(&text, None).unwrap();
    for item in &mut journal.items {
      if let Item::Txn { src, .. } = item {
        *src = None;
      }
    }
    let out = dir.path().join(path.file_name().unwrap());
    fs::write(&out, writer::write(&journal)).unwrap();
    assert_eq!(
      hledger_check(&out),
      hledger_check(&path),
      "{}",
      path.display()
    );
    if hledger_check(&path) {
      assert_eq!(
        hledger_balances(&out),
        hledger_balances(&path),
        "{}",
        path.display()
      );
    }
  }
}
