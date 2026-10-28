use crate::ledger::model::{Journal, Transaction};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountKind {
  Asset,
  Liability,
  Equity,
  Income,
  Expense,
  Other,
}

impl AccountKind {
  pub fn of(account: &str) -> AccountKind {
    let top = account.split(':').next().unwrap_or("").to_lowercase();
    match top.as_str() {
      "assets" | "asset" => AccountKind::Asset,
      "liabilities" | "liability" => AccountKind::Liability,
      "equity" => AccountKind::Equity,
      "income" | "revenue" | "revenues" => AccountKind::Income,
      "expenses" | "expense" => AccountKind::Expense,
      _ => AccountKind::Other,
    }
  }

  pub fn is_balance_sheet(self) -> bool {
    matches!(self, AccountKind::Asset | AccountKind::Liability)
  }
}

/// Per-account balances, excluding subaccounts, over transactions accepted by `keep`.
pub fn balances(
  journal: &Journal,
  keep: impl Fn(&Transaction) -> bool,
) -> BTreeMap<String, Decimal> {
  let mut out: BTreeMap<String, Decimal> = BTreeMap::new();
  for (_, t) in journal.transactions().filter(|(_, t)| keep(t)) {
    for (p, amt) in t.resolved_postings() {
      if let Some(a) = amt {
        *out.entry(p.account.clone()).or_default() += a.quantity;
      }
    }
  }
  out
}

pub fn balances_as_of(journal: &Journal, date: Option<NaiveDate>) -> BTreeMap<String, Decimal> {
  balances(journal, |t| date.is_none_or(|d| t.date <= d))
}

pub fn balances_between(
  journal: &Journal,
  from: NaiveDate,
  to: NaiveDate,
) -> BTreeMap<String, Decimal> {
  balances(journal, |t| t.date >= from && t.date <= to)
}

/// Add every parent account with the sum of its subaccounts.
pub fn rollup(balances: &BTreeMap<String, Decimal>) -> BTreeMap<String, Decimal> {
  let mut out: BTreeMap<String, Decimal> = BTreeMap::new();
  for (acct, q) in balances {
    let mut prefix = String::new();
    for (k, part) in acct.split(':').enumerate() {
      if k > 0 {
        prefix.push(':');
      }
      prefix.push_str(part);
      *out.entry(prefix.clone()).or_default() += *q;
    }
  }
  out
}

/// Assets plus liabilities as of `date`.
pub fn net_worth(journal: &Journal, date: Option<NaiveDate>) -> Decimal {
  balances_as_of(journal, date)
    .iter()
    .filter(|(a, _)| AccountKind::of(a).is_balance_sheet())
    .map(|(_, q)| *q)
    .sum()
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ledger::parser::parse;

  #[test]
  fn rollup_sums_children() {
    let j = parse(
      "2026-01-01 x\n  assets:a:b  5 PLN\n  assets:c  2 PLN\n  equity\n",
      None,
    )
    .unwrap();
    let r = rollup(&balances_as_of(&j, None));
    assert_eq!(r["assets"], Decimal::from(7));
    assert_eq!(r["assets:a"], Decimal::from(5));
    assert_eq!(r["equity"], Decimal::from(-7));
    assert_eq!(net_worth(&j, None), Decimal::from(7));
  }
}
