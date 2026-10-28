use super::balance::{AccountKind, balances_between, net_worth};
use crate::ledger::model::{Journal, Transaction};
use chrono::{Datelike, NaiveDate};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Month {
  pub year: i32,
  pub month: u32,
}

impl Month {
  pub fn of(date: NaiveDate) -> Month {
    Month {
      year: date.year(),
      month: date.month(),
    }
  }

  pub fn parse(s: &str) -> Option<Month> {
    let (y, m) = s.trim().split_once(['-', '/', '.'])?;
    let month = Month {
      year: y.parse().ok()?,
      month: m.parse().ok()?,
    };
    (1..=12).contains(&month.month).then_some(month)
  }

  pub fn first_day(self) -> NaiveDate {
    NaiveDate::from_ymd_opt(self.year, self.month, 1).expect("valid month")
  }

  pub fn last_day(self) -> NaiveDate {
    self.next().first_day().pred_opt().expect("valid date")
  }

  pub fn next(self) -> Month {
    if self.month == 12 {
      Month {
        year: self.year + 1,
        month: 1,
      }
    } else {
      Month {
        month: self.month + 1,
        ..self
      }
    }
  }

  pub fn prev(self) -> Month {
    if self.month == 1 {
      Month {
        year: self.year - 1,
        month: 12,
      }
    } else {
      Month {
        month: self.month - 1,
        ..self
      }
    }
  }

  pub fn contains(self, date: NaiveDate) -> bool {
    Month::of(date) == self
  }

  pub fn short_name(self) -> String {
    self.first_day().format("%b").to_string()
  }

  pub fn long_name(self) -> String {
    self.first_day().format("%b %Y").to_string()
  }

  /// The `count` months ending with `self`, oldest first.
  pub fn trailing(self, count: usize) -> Vec<Month> {
    let mut out = vec![self];
    for _ in 1..count {
      let prev = out.last().unwrap().prev();
      out.push(prev);
    }
    out.reverse();
    out
  }

  pub fn year_months(self) -> Vec<Month> {
    (1..=12)
      .map(|month| Month {
        year: self.year,
        month,
      })
      .collect()
  }
}

impl fmt::Display for Month {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "{:04}-{:02}", self.year, self.month)
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Flow {
  pub income: Decimal,
  pub spending: Decimal,
}

impl Flow {
  pub fn saved(self) -> Decimal {
    self.income - self.spending
  }
}

/// Income (sign flipped to positive) and spending between two dates, inclusive.
pub fn flow_between(journal: &Journal, from: NaiveDate, to: NaiveDate) -> Flow {
  let mut flow = Flow {
    income: Decimal::ZERO,
    spending: Decimal::ZERO,
  };
  for (acct, q) in balances_between(journal, from, to) {
    match AccountKind::of(&acct) {
      AccountKind::Income => flow.income -= q,
      AccountKind::Expense => flow.spending += q,
      _ => {}
    }
  }
  flow
}

pub fn monthly_flows(journal: &Journal, months: &[Month]) -> Vec<(Month, Flow)> {
  months
    .iter()
    .map(|m| (*m, flow_between(journal, m.first_day(), m.last_day())))
    .collect()
}

/// Spending per expense category (account truncated to `depth` levels), largest first.
pub fn categories(
  journal: &Journal,
  from: NaiveDate,
  to: NaiveDate,
  depth: usize,
) -> Vec<(String, Decimal)> {
  let mut cats: BTreeMap<String, Decimal> = BTreeMap::new();
  for (acct, q) in balances_between(journal, from, to) {
    if AccountKind::of(&acct) != AccountKind::Expense {
      continue;
    }
    let name: Vec<&str> = acct.split(':').take(depth).collect();
    let name = if name.len() > 1 {
      name[1..].join(":")
    } else {
      name.join(":")
    };
    *cats.entry(name).or_default() += q;
  }
  let mut out: Vec<_> = cats.into_iter().filter(|(_, q)| !q.is_zero()).collect();
  out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
  out
}

/// Net worth at the end of each month.
pub fn net_worth_series(journal: &Journal, months: &[Month]) -> Vec<Decimal> {
  months
    .iter()
    .map(|m| net_worth(journal, Some(m.last_day())))
    .collect()
}

/// Net effect of a transaction on assets and liabilities, falling back to its first amount.
pub fn transaction_flow(txn: &Transaction) -> Decimal {
  let resolved = txn.resolved_postings();
  let mut sheet = resolved
    .iter()
    .filter(|(p, _)| AccountKind::of(&p.account).is_balance_sheet());
  if sheet.clone().next().is_none() {
    return resolved
      .first()
      .and_then(|(_, a)| a.as_ref())
      .map(|a| a.quantity)
      .unwrap_or_default();
  }
  let sum: Decimal = sheet
    .by_ref()
    .filter_map(|(_, a)| a.as_ref())
    .map(|a| a.quantity)
    .sum();
  if sum.is_zero() {
    // a transfer between own accounts: show the first amount moved
    return resolved
      .first()
      .and_then(|(_, a)| a.as_ref())
      .map(|a| a.quantity.abs())
      .unwrap_or_default();
  }
  sum
}

/// True when money only moves between own asset and liability accounts.
pub fn is_transfer(txn: &Transaction) -> bool {
  txn
    .postings
    .iter()
    .all(|p| AccountKind::of(&p.account).is_balance_sheet())
}

/// The account that best describes a transaction: the first non-balance-sheet posting.
pub fn transaction_category(txn: &Transaction) -> &str {
  txn
    .postings
    .iter()
    .find(|p| !AccountKind::of(&p.account).is_balance_sheet())
    .or(txn.postings.first())
    .map(|p| p.account.as_str())
    .unwrap_or("")
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ledger::parser::parse;

  #[test]
  fn months() {
    let m = Month::parse("2026-01").unwrap();
    assert_eq!(m.prev().to_string(), "2025-12");
    assert_eq!(m.prev().next(), m);
    assert_eq!(Month::parse("2024-02").unwrap().last_day().day(), 29);
    assert_eq!(
      m.trailing(3),
      vec![Month::parse("2025-11").unwrap(), m.prev(), m]
    );
    assert!(Month::parse("2026-13").is_none());
  }

  #[test]
  fn flows_and_categories() {
    let text = "2026-01-05 Pay\n  assets:bank  100 PLN\n  income:salary\n\n\
                2026-01-06 Food\n  expenses:food:out  30 PLN\n  assets:bank\n\n\
                2026-01-07 Bus\n  expenses:transport  5 PLN\n  liabilities:card\n";
    let j = parse(text, None).unwrap();
    let m = Month::parse("2026-01").unwrap();
    let f = flow_between(&j, m.first_day(), m.last_day());
    assert_eq!(
      (f.income, f.spending, f.saved()),
      (100.into(), 35.into(), 65.into())
    );
    let c = categories(&j, m.first_day(), m.last_day(), 2);
    assert_eq!(
      c,
      vec![
        ("food".to_string(), 30.into()),
        ("transport".to_string(), 5.into())
      ]
    );
    let t: Vec<_> = j.transactions().map(|(_, t)| transaction_flow(t)).collect();
    assert_eq!(
      t,
      vec![Decimal::from(100), Decimal::from(-30), Decimal::from(-5)]
    );
  }
}
