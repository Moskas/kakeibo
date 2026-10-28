use super::model::{Item, Journal, Posting, Transaction};
use super::parser;
use super::writer::line_ending;
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::collections::HashMap;
use std::fmt;

/// Replace `removed.len()` items at `index` with `inserted`. Inverting swaps the two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
  pub index: usize,
  pub removed: Vec<Item>,
  pub inserted: Vec<Item>,
}

impl Splice {
  pub fn inverse(&self) -> Splice {
    Splice {
      index: self.index,
      removed: self.inserted.clone(),
      inserted: self.removed.clone(),
    }
  }
}

pub fn apply(journal: &mut Journal, splice: &Splice) {
  let end = splice.index + splice.removed.len();
  journal
    .items
    .splice(splice.index..end, splice.inserted.iter().cloned());
  if journal.commodity.is_none() {
    journal.commodity = splice.inserted.iter().find_map(|it| match it {
      Item::Txn { txn, .. } => txn
        .postings
        .iter()
        .find_map(|p| p.amount.as_ref())
        .map(|a| a.commodity.clone()),
      Item::Raw(_) => None,
    });
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
  pub line: usize,
  pub message: String,
}

impl fmt::Display for Problem {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if self.line > 0 {
      write!(f, "line {}: {}", self.line, self.message)
    } else {
      write!(f, "{}", self.message)
    }
  }
}

fn check_account_name(name: &str) -> Result<(), String> {
  if name.contains("  ") || name.contains(['\t', ';', '\n', '\r']) || name != name.trim() {
    return Err(format!("Invalid account name '{name}'"));
  }
  if name.starts_with(['(', '[']) {
    return Err("Virtual postings are not supported".into());
  }
  Ok(())
}

/// Validate a single transaction against the journal commodity.
pub fn validate(txn: &Transaction, commodity: Option<&str>) -> Result<(), String> {
  if txn.description.contains([';', '\n', '\r']) {
    return Err("Description cannot contain ';' or newlines".into());
  }
  if txn.postings.len() < 2 {
    return Err("A transaction needs at least two postings".into());
  }
  for p in &txn.postings {
    if p.account.trim().is_empty() {
      return Err("Every posting needs an account".into());
    }
    check_account_name(&p.account)?;
    for a in [
      p.amount.as_ref(),
      p.cost.as_ref().map(|c| c.amount()),
      p.assertion.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
      if let Some(c) = commodity
        && a.commodity != c
        && !(a.commodity.is_empty() && a.quantity.is_zero())
      {
        return Err(format!(
          "Commodity '{}' differs from journal commodity '{c}'",
          a.commodity
        ));
      }
    }
  }
  let elided = txn.postings.iter().filter(|p| p.amount.is_none()).count();
  if elided > 1 {
    return Err("Only one posting may omit its amount".into());
  }
  if elided == 0 {
    let amounts: Vec<_> = txn
      .postings
      .iter()
      .filter_map(|p| p.balancing_amount())
      .collect();
    let scale = amounts
      .iter()
      .map(|a| a.quantity.scale())
      .max()
      .unwrap_or(0);
    let sum: Decimal = amounts.iter().map(|a| a.quantity).sum();
    if !sum.round_dp(scale).is_zero() {
      return Err(format!(
        "Amounts must balance (off by {})",
        sum.round_dp(scale)
      ));
    }
  }
  Ok(())
}

/// Validate every transaction and check balance assertions in date order.
pub fn check(journal: &Journal) -> Vec<Problem> {
  let mut problems = Vec::new();
  let commodity = journal.commodity.as_deref();
  let mut txns: Vec<&Transaction> = journal.transactions().map(|(_, t)| t).collect();
  for t in &txns {
    if let Err(m) = validate(t, commodity) {
      problems.push(Problem {
        line: t.line,
        message: m,
      });
    }
  }

  txns.sort_by_key(|t| t.date);
  let mut balances: HashMap<&str, Decimal> = HashMap::new();
  for t in txns {
    for (p, amt) in t.resolved_postings() {
      let bal = balances.entry(&p.account).or_default();
      if let Some(a) = amt {
        *bal += a.quantity;
      }
      if let Some(expected) = &p.assertion
        && *bal != expected.quantity
      {
        problems.push(Problem {
          line: p.line,
          message: format!(
            "balance assertion failed for {}: expected {}, actual {}",
            p.account,
            expected,
            expected.with_quantity(*bal)
          ),
        });
      }
    }
  }
  problems.sort_by_key(|p| p.line);
  problems
}

/// Append a transaction at the end of the journal.
pub fn add_transaction(journal: &Journal, txn: Transaction) -> Result<Splice, String> {
  validate(&txn, journal.commodity.as_deref())?;
  Ok(append(journal, Item::Txn { txn, src: None }, true))
}

/// Declare an account with an `account` directive, next to existing declarations,
/// otherwise before the first transaction.
pub fn add_account(journal: &Journal, name: &str) -> Result<Splice, String> {
  let name = name.trim();
  if name.is_empty() {
    return Err("Account name is required".into());
  }
  check_account_name(name)?;
  if name.split(':').any(str::is_empty) {
    return Err(format!("Account '{name}' has an empty part"));
  }
  if journal.accounts().iter().any(|a| a == name) {
    return Err(format!("Account '{name}' already exists"));
  }
  let eol = line_ending(journal);
  let line = Item::Raw(format!("account {name}{eol}"));
  let items = &journal.items;

  let is_decl = |it: &Item| matches!(it, Item::Raw(s) if s.starts_with("account "));
  if let Some(last) = items.iter().rposition(is_decl) {
    // skip indented subdirectives belonging to that declaration
    let mut at = last + 1;
    while matches!(items.get(at), Some(Item::Raw(s)) if s.starts_with([' ', '\t']) && !s.trim().is_empty())
    {
      at += 1;
    }
    if at == items.len() {
      return Ok(append(journal, line, false));
    }
    return Ok(Splice {
      index: at,
      removed: vec![],
      inserted: vec![line],
    });
  }
  if let Some(first) = items.iter().position(|it| matches!(it, Item::Txn { .. })) {
    let inserted = vec![line, Item::Raw(eol.to_string())];
    return Ok(Splice {
      index: first,
      removed: vec![],
      inserted,
    });
  }
  Ok(append(journal, line, false))
}

/// Append an item, terminating the last line first and optionally adding a blank separator.
fn append(journal: &Journal, item: Item, separate: bool) -> Splice {
  let eol = line_ending(journal);
  let mut splice = Splice {
    index: journal.items.len(),
    removed: vec![],
    inserted: vec![],
  };

  if let Some(last) = journal.items.last() {
    let text = match last {
      Item::Raw(s) => Some(s.as_str()),
      Item::Txn { src, .. } => src.as_deref(),
    };
    if text.is_some_and(|s| !s.ends_with('\n')) {
      splice.index -= 1;
      splice.removed.push(last.clone());
      splice.inserted.push(match last {
        Item::Raw(s) => Item::Raw(format!("{s}{eol}")),
        Item::Txn { txn, src } => Item::Txn {
          txn: txn.clone(),
          src: src.as_ref().map(|s| format!("{s}{eol}")),
        },
      });
    }
    if separate && !is_blank(last) {
      splice.inserted.push(Item::Raw(eol.to_string()));
    }
  }
  splice.inserted.push(item);
  splice
}

pub fn replace_transaction(
  journal: &Journal,
  index: usize,
  txn: Transaction,
) -> Result<Splice, String> {
  validate(&txn, journal.commodity.as_deref())?;
  let old = journal
    .items
    .get(index)
    .filter(|it| matches!(it, Item::Txn { .. }));
  let old = old.ok_or("No transaction at that position")?.clone();
  Ok(Splice {
    index,
    removed: vec![old],
    inserted: vec![Item::Txn { txn, src: None }],
  })
}

/// Remove a transaction, plus one adjacent blank line so spacing stays even.
pub fn delete_transaction(journal: &Journal, index: usize) -> Result<Splice, String> {
  let items = &journal.items;
  if !matches!(items.get(index), Some(Item::Txn { .. })) {
    return Err("No transaction at that position".into());
  }
  let prev_blank = index == 0 || is_blank(&items[index - 1]);
  let next = items.get(index + 1);
  let (start, len) = match next {
    Some(n) if is_blank(n) && prev_blank => (index, 2),
    None if index > 0 && prev_blank => (index - 1, 2),
    _ => (index, 1),
  };
  Ok(Splice {
    index: start,
    removed: items[start..start + len].to_vec(),
    inserted: vec![],
  })
}

fn is_blank(item: &Item) -> bool {
  matches!(item, Item::Raw(s) if s.trim().is_empty())
}

/// Fields of a transaction as typed by a user, in the form or on the command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
  pub date: String,
  pub payee: String,
  pub note: String,
  /// (account, amount text); an empty amount is inferred.
  pub postings: Vec<(String, String)>,
}

/// Build a transaction from a draft. Bare numbers get the journal commodity.
pub fn build(draft: &Draft, journal: &Journal) -> Result<Transaction, String> {
  let date = parse_date(&draft.date)?;
  let mut txn = Transaction::new(date, draft.payee.trim());
  if txn.description.is_empty() {
    return Err("Payee is required".into());
  }
  let note = draft.note.trim();
  if !note.is_empty() {
    txn.comment = Some(note.to_string());
  }
  let commodity = journal.commodity_or_default();
  let (style, _) = journal.display_style();
  for (account, amount) in &draft.postings {
    let (account, amount) = (account.trim(), amount.trim());
    if account.is_empty() && amount.is_empty() {
      continue;
    }
    let amount = if amount.is_empty() {
      None
    } else {
      let mut s = amount;
      let mut a = parser::amount(&mut s).map_err(|e| format!("Amount: {e}"))?;
      if !s.trim().is_empty() {
        return Err(format!("Amount: unexpected '{}'", s.trim()));
      }
      if a.commodity.is_empty() {
        a.commodity = commodity.clone();
        a.style = style;
      }
      Some(a)
    };
    txn.postings.push(Posting::new(account, amount));
  }
  Ok(txn)
}

/// Build and validate a draft.
pub fn build_valid(draft: &Draft, journal: &Journal) -> Result<Transaction, String> {
  let txn = build(draft, journal)?;
  validate(&txn, journal.commodity.as_deref())?;
  Ok(txn)
}

pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
  let s = s.trim();
  for fmt in ["%Y-%m-%d", "%Y/%m/%d", "%Y.%m.%d"] {
    if let Ok(d) = NaiveDate::parse_from_str(s, fmt) {
      return Ok(d);
    }
  }
  Err(format!("Invalid date '{s}', use YYYY-MM-DD"))
}

/// The draft for an existing transaction, if the simple form can represent it.
pub fn draft_of(txn: &Transaction) -> Result<Draft, String> {
  if !txn.postings.iter().all(|p| p.is_simple()) {
    return Err(
      "This transaction has costs, assertions or posting comments; edit it in a text editor".into(),
    );
  }
  Ok(Draft {
    date: txn.date.format("%Y-%m-%d").to_string(),
    payee: txn.description.clone(),
    note: txn.comment.clone().unwrap_or_default(),
    postings: txn
      .postings
      .iter()
      .map(|p| {
        (
          p.account.clone(),
          p.amount.as_ref().map(|a| a.to_string()).unwrap_or_default(),
        )
      })
      .collect(),
  })
}

/// Apply a draft to an existing transaction, keeping status, code and comment lines.
pub fn merge_draft(original: &Transaction, edited: Transaction) -> Transaction {
  Transaction {
    status: original.status,
    code: original.code.clone(),
    comment_lines: original.comment_lines.clone(),
    ..edited
  }
}
#[cfg(test)]
mod tests {
  use super::*;
  use crate::ledger::{parser::parse, writer::write};

  const TEXT: &str = "2026-01-01 a\n  x  1 PLN\n  y\n\n2026-01-02 b\n  x  2 PLN\n  y\n";

  #[test]
  fn add_and_undo() {
    let mut j = parse(TEXT, None).unwrap();
    let txn = j.transaction(0).unwrap().clone();
    let s = add_transaction(&j, txn).unwrap();
    apply(&mut j, &s);
    assert!(write(&j).starts_with(TEXT));
    assert!(write(&j).ends_with("\n\n2026-01-01 a\n    x  1 PLN\n    y\n"));
    apply(&mut j, &s.inverse());
    assert_eq!(write(&j), TEXT);
  }

  #[test]
  fn add_without_trailing_newline() {
    let mut j = parse("2026-01-01 a\n  x  1 PLN\n  y", None).unwrap();
    let txn = j.transaction(0).unwrap().clone();
    let s = add_transaction(&j, txn).unwrap();
    apply(&mut j, &s);
    assert_eq!(
      write(&j),
      "2026-01-01 a\n  x  1 PLN\n  y\n\n2026-01-01 a\n    x  1 PLN\n    y\n"
    );
  }

  #[test]
  fn delete_keeps_spacing() {
    let mut j = parse(TEXT, None).unwrap();
    let s = delete_transaction(&j, 0).unwrap();
    apply(&mut j, &s);
    assert_eq!(write(&j), "2026-01-02 b\n  x  2 PLN\n  y\n");
    let mut j = parse(TEXT, None).unwrap();
    let s = delete_transaction(&j, 2).unwrap();
    apply(&mut j, &s);
    assert_eq!(write(&j), "2026-01-01 a\n  x  1 PLN\n  y\n");
  }

  #[test]
  fn validation() {
    let j = parse(TEXT, None).unwrap();
    let mut t = j.transaction(0).unwrap().clone();
    assert!(validate(&t, Some("PLN")).is_ok());
    assert!(validate(&t, Some("EUR")).is_err());
    t.postings[1].amount = t.postings[0].amount.clone();
    assert!(validate(&t, Some("PLN")).unwrap_err().contains("balance"));
  }

  #[test]
  fn assertions_in_date_order() {
    let text = "2026-01-02 b\n  x  2 PLN = 3 PLN\n  y\n\n2026-01-01 a\n  x  1 PLN = 1 PLN\n  y\n";
    assert!(check(&parse(text, None).unwrap()).is_empty());
    let bad = text.replace("= 3 PLN", "= 2 PLN");
    let problems = check(&parse(&bad, None).unwrap());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].line, 2);
  }

  fn with_account(text: &str, name: &str) -> String {
    let mut j = parse(text, None).unwrap();
    let s = add_account(&j, name).unwrap();
    apply(&mut j, &s);
    let out = write(&j);
    apply(&mut j, &s.inverse());
    assert_eq!(write(&j), text);
    out
  }

  #[test]
  fn add_account_placement() {
    let decls =
      "; top\naccount assets:bank\naccount equity\n  ; note\n\n2026-01-01 a\n  x  1 PLN\n  y\n";
    assert_eq!(
      with_account(decls, "expenses:pets"),
      decls.replace("  ; note\n", "  ; note\naccount expenses:pets\n")
    );
    assert_eq!(
      with_account(TEXT, "expenses:pets"),
      format!("account expenses:pets\n\n{TEXT}")
    );
    assert_eq!(with_account("", "assets:cash"), "account assets:cash\n");
    assert_eq!(
      with_account("; only a comment", "a"),
      "; only a comment\naccount a\n"
    );
    assert_eq!(with_account("account b", "a"), "account b\naccount a\n");
  }

  #[test]
  fn add_account_rejects_bad_names() {
    let j = parse(TEXT, None).unwrap();
    for bad in ["", "x", "a  b", "a\tb", "(virtual)", "expenses:", "a;b"] {
      assert!(add_account(&j, bad).is_err(), "{bad:?} should be rejected");
    }
    assert!(add_account(&j, "expenses:pet food").is_ok());
  }
}
