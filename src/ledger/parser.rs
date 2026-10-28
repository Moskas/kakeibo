use super::model::{Amount, AmountStyle, Cost, Item, Journal, Posting, Status, Transaction};
use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::fmt;
use std::str::FromStr;
use winnow::ascii::{digit1, space0};
use winnow::combinator::{alt, delimited, opt, preceded};
use winnow::prelude::*;
use winnow::token::{one_of, take_till, take_while};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
  pub line: usize,
  pub message: String,
}

impl fmt::Display for ParseError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "line {}: {}", self.line, self.message)
  }
}

impl std::error::Error for ParseError {}

fn err<T>(line: usize, message: impl Into<String>) -> Result<T, ParseError> {
  Err(ParseError {
    line,
    message: message.into(),
  })
}

/// Parse a journal. `commodity` is the configured commodity; when `None` the first one seen wins.
pub fn parse(text: &str, commodity: Option<&str>) -> Result<Journal, ParseError> {
  let lines: Vec<&str> = text.split_inclusive('\n').collect();
  let mut items = Vec::new();
  let mut i = 0;
  let mut in_comment_block = false;
  let mut after_directive = false;

  while i < lines.len() {
    let raw = lines[i];
    let line = strip_eol(raw);
    let lineno = i + 1;

    if in_comment_block {
      if line.trim_end() == "end comment" {
        in_comment_block = false;
      }
      items.push(Item::Raw(raw.to_string()));
      i += 1;
      continue;
    }

    if line.trim().is_empty() {
      after_directive = false;
      items.push(Item::Raw(raw.to_string()));
      i += 1;
      continue;
    }

    let first = line.chars().next().unwrap_or(' ');
    if first.is_ascii_digit() {
      let mut end = i + 1;
      while end < lines.len() {
        let l = strip_eol(lines[end]);
        if l.starts_with([' ', '\t']) && !l.trim().is_empty() {
          end += 1;
        } else {
          break;
        }
      }
      let src: String = lines[i..end].concat();
      let txn = parse_transaction(&lines[i..end], lineno)?;
      items.push(Item::Txn {
        txn,
        src: Some(src),
      });
      after_directive = false;
      i = end;
      continue;
    }

    if first == ' ' || first == '\t' {
      if !after_directive {
        return err(lineno, "unexpected indented line outside a transaction");
      }
      items.push(Item::Raw(raw.to_string()));
      i += 1;
      continue;
    }

    if matches!(first, ';' | '#' | '*') {
      after_directive = false;
      items.push(Item::Raw(raw.to_string()));
      i += 1;
      continue;
    }

    let word = line.split_whitespace().next().unwrap_or("");
    match word {
      "comment" => in_comment_block = true,
      "account" | "commodity" | "payee" | "tag" => after_directive = true,
      "include" => {
        return err(
          lineno,
          "include directives are not supported; kakeibo reads a single file",
        );
      }
      "~" => return err(lineno, "periodic transactions are not supported"),
      "=" => return err(lineno, "automated transactions are not supported"),
      "P" => {
        return err(
          lineno,
          "price directives are not supported (single commodity only)",
        );
      }
      _ => return err(lineno, format!("unsupported directive '{word}'")),
    }
    items.push(Item::Raw(raw.to_string()));
    i += 1;
  }

  let mut journal = Journal {
    items,
    commodity: commodity.map(str::to_string),
  };
  check_commodities(&mut journal)?;
  Ok(journal)
}

fn strip_eol(s: &str) -> &str {
  s.strip_suffix('\n')
    .map(|s| s.strip_suffix('\r').unwrap_or(s))
    .unwrap_or(s)
}

fn check_commodities(journal: &mut Journal) -> Result<(), ParseError> {
  let mut expected = journal.commodity.clone();
  for item in &journal.items {
    let Item::Txn { txn, .. } = item else {
      continue;
    };
    for p in &txn.postings {
      let amounts = [
        p.amount.as_ref(),
        p.cost.as_ref().map(Cost::amount),
        p.assertion.as_ref(),
      ];
      for a in amounts.into_iter().flatten() {
        if a.commodity.is_empty() && a.quantity.is_zero() {
          continue;
        }
        match &expected {
          None => expected = Some(a.commodity.clone()),
          Some(c) if *c != a.commodity => {
            return err(
              p.line,
              format!(
                "commodity '{}' differs from journal commodity '{c}'; multiple commodities are \
                 not supported",
                a.commodity
              ),
            );
          }
          Some(_) => {}
        }
      }
    }
  }
  journal.commodity = expected;
  Ok(())
}

fn parse_transaction(lines: &[&str], lineno: usize) -> Result<Transaction, ParseError> {
  let header = strip_eol(lines[0]);
  let mut txn = parse_header(header).map_err(|m| ParseError {
    line: lineno,
    message: m,
  })?;
  txn.line = lineno;

  for (k, raw) in lines.iter().enumerate().skip(1) {
    let line_no = lineno + k;
    let body = strip_eol(raw).trim_start();
    if let Some(c) = body.strip_prefix(';') {
      let c = c.trim().to_string();
      match txn.postings.last_mut() {
        Some(p) => p.comment_lines.push(c),
        None => txn.comment_lines.push(c),
      }
      continue;
    }
    if body.starts_with('#') {
      return err(
        line_no,
        "'#' comments inside transactions are not supported; use ';'",
      );
    }
    let mut posting = parse_posting(body).map_err(|m| ParseError {
      line: line_no,
      message: m,
    })?;
    posting.line = line_no;
    txn.postings.push(posting);
  }

  if txn.postings.iter().filter(|p| p.amount.is_none()).count() > 1 {
    return err(lineno, "more than one posting has no amount");
  }
  Ok(txn)
}

fn date_part(i: &mut &str) -> ModalResult<u32> {
  digit1.try_map(u32::from_str).parse_next(i)
}

fn date(i: &mut &str) -> ModalResult<NaiveDate> {
  let year = digit1.try_map(i32::from_str).parse_next(i)?;
  let sep = one_of(['-', '/', '.']).parse_next(i)?;
  let month = date_part.parse_next(i)?;
  one_of(sep).parse_next(i)?;
  let day = date_part.parse_next(i)?;
  NaiveDate::from_ymd_opt(year, month, day)
    .ok_or_else(|| winnow::error::ErrMode::Cut(winnow::error::ContextError::new()))
}

fn status(i: &mut &str) -> ModalResult<Status> {
  opt(one_of(['*', '!']))
    .map(|c| match c {
      Some('*') => Status::Cleared,
      Some('!') => Status::Pending,
      _ => Status::Unmarked,
    })
    .parse_next(i)
}

fn parse_header(line: &str) -> Result<Transaction, String> {
  let mut i = line;
  let d = date
    .parse_next(&mut i)
    .map_err(|_| format!("invalid date in '{line}'"))?;
  if i.starts_with('=') {
    return Err("secondary dates are not supported".into());
  }
  if !i.is_empty() && !i.starts_with([' ', '\t']) {
    return Err(format!("invalid date in '{line}'"));
  }
  let _ = space0::<_, winnow::error::ContextError>.parse_next(&mut i);
  let st = status
    .parse_next(&mut i)
    .map_err(|_: winnow::error::ErrMode<_>| "bad status")?;
  let _ = space0::<_, winnow::error::ContextError>.parse_next(&mut i);
  let code: Option<&str> = opt(delimited('(', take_till(0.., ')'), ')'))
    .parse_next(&mut i)
    .map_err(|_: winnow::error::ErrMode<winnow::error::ContextError>| "bad code")?;

  let (desc, comment) = split_comment(i);
  let mut txn = Transaction::new(d, desc.trim());
  txn.status = st;
  txn.code = code.map(str::to_string);
  txn.comment = comment;
  Ok(txn)
}

fn split_comment(s: &str) -> (&str, Option<String>) {
  match s.find(';') {
    Some(pos) => (&s[..pos], Some(s[pos + 1..].trim().to_string())),
    None => (s, None),
  }
}

fn parse_posting(body: &str) -> Result<Posting, String> {
  let mut i = body;
  let st = status
    .parse_next(&mut i)
    .map_err(|_: winnow::error::ErrMode<_>| "bad status")?;
  let i = i.trim_start_matches([' ', '\t']);

  // hledger ends an account name at two consecutive whitespace characters.
  let bytes = i.as_bytes();
  let ws = |b: u8| b == b' ' || b == b'\t';
  let acct_end = (0..bytes.len().saturating_sub(1))
    .find(|&k| ws(bytes[k]) && ws(bytes[k + 1]))
    .unwrap_or(i.len());
  let (mut account, mut rest) = (&i[..acct_end], &i[acct_end..]);
  if let Some(pos) = account.find(';') {
    rest = &i[pos..];
    account = &i[..pos];
  }
  let account = account.trim_end();
  if account.is_empty() {
    return Err("posting has no account".into());
  }
  if account.starts_with(['(', '[']) {
    return Err("virtual postings are not supported".into());
  }
  if account.contains('\t') {
    return Err("separate account and amount with two spaces or tabs, not one tab".into());
  }

  let mut p = Posting::new(account, None);
  p.status = st;

  let (amount_text, comment) = split_comment(rest);
  p.comment = comment;
  let mut s = amount_text.trim();
  if s.is_empty() {
    return Ok(p);
  }
  if s.starts_with('=') {
    return Err("balance assignments are not supported".into());
  }

  p.amount = Some(amount(&mut s).map_err(|e| format!("invalid amount: {e}"))?);
  s = s.trim_start();
  if let Some(r) = s.strip_prefix("@@") {
    let mut r = r.trim_start();
    p.cost = Some(Cost::Total(
      amount(&mut r).map_err(|e| format!("invalid cost: {e}"))?,
    ));
    s = r.trim_start();
  } else if let Some(r) = s.strip_prefix('@') {
    let mut r = r.trim_start();
    p.cost = Some(Cost::Unit(
      amount(&mut r).map_err(|e| format!("invalid cost: {e}"))?,
    ));
    s = r.trim_start();
  }
  if let Some(r) = s.strip_prefix('=') {
    if r.starts_with(['=', '*']) {
      return Err("only '=' balance assertions are supported".into());
    }
    let mut r = r.trim_start();
    p.assertion = Some(amount(&mut r).map_err(|e| format!("invalid assertion: {e}"))?);
    s = r.trim_start();
  }
  if !s.is_empty() {
    return Err(format!("unexpected text after amount: '{s}'"));
  }
  Ok(p)
}

fn commodity<'a>(i: &mut &'a str) -> ModalResult<&'a str> {
  alt((
    delimited('"', take_till(1.., '"'), '"'),
    take_while(1.., |c: char| {
      !c.is_ascii_digit() && !c.is_whitespace() && !"-+.,;@=\"*!()[]{}".contains(c)
    }),
  ))
  .parse_next(i)
}

fn sign(i: &mut &str) -> ModalResult<bool> {
  opt(one_of(['-', '+']))
    .map(|c| c == Some('-'))
    .parse_next(i)
}

fn number_text<'a>(i: &mut &'a str) -> ModalResult<&'a str> {
  take_while(1.., |c: char| c.is_ascii_digit() || c == '.' || c == ',').parse_next(i)
}

/// Parse `[-]COMMODITY [-]NUMBER` or `[-]NUMBER [COMMODITY]` from the start of `s`.
pub fn amount(s: &mut &str) -> Result<Amount, String> {
  let start = *s;
  let mut i = *s;
  let neg1 = sign.parse_next(&mut i).map_err(|_| "bad sign")?;
  let left: Option<&str> = opt(commodity)
    .parse_next(&mut i)
    .map_err(|_| "bad commodity")?;
  let (neg, num, comm, style) = if let Some(c) = left {
    let spaced = !space0::<_, winnow::error::ContextError>
      .parse_next(&mut i)
      .unwrap()
      .is_empty();
    let neg2 = sign.parse_next(&mut i).map_err(|_| "bad sign")?;
    let num = number_text
      .parse_next(&mut i)
      .map_err(|_| format!("no number in '{start}'"))?;
    (
      neg1 ^ neg2,
      num,
      c,
      AmountStyle {
        commodity_left: true,
        spaced,
      },
    )
  } else {
    let num = number_text
      .parse_next(&mut i)
      .map_err(|_| format!("no number in '{start}'"))?;
    let mut probe = i;
    let ws = space0::<_, winnow::error::ContextError>
      .parse_next(&mut probe)
      .unwrap();
    let c: Option<&str> = opt(preceded(space0, commodity))
      .parse_next(&mut i)
      .map_err(|_| "bad commodity")?;
    let comm = c.unwrap_or("");
    (
      neg1,
      num,
      comm,
      AmountStyle {
        commodity_left: false,
        spaced: !ws.is_empty() || c.is_none(),
      },
    )
  };
  let mut q = parse_number(num)?;
  if neg {
    q = -q;
  }
  *s = i;
  Ok(Amount {
    quantity: q,
    commodity: comm.to_string(),
    style,
  })
}

/// Parse a number with `.` as decimal mark and optional `,` digit groups.
fn parse_number(text: &str) -> Result<Decimal, String> {
  let (int_part, frac) = match text.split_once('.') {
    Some((a, b)) => (a, Some(b)),
    None => (text, None),
  };
  if let Some(f) = frac
    && f.contains(['.', ','])
  {
    return Err(format!(
      "'{text}': use '.' as decimal mark and ',' for digit groups"
    ));
  }
  if int_part.contains(',') {
    let groups: Vec<&str> = int_part.split(',').collect();
    let ok =
      !groups[0].is_empty() && groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3);
    if !ok || (frac.is_none() && groups.len() == 2) {
      return Err(format!("'{text}' is ambiguous; use '.' as decimal mark"));
    }
  }
  let int_digits = int_part.replace(',', "");
  let normalized = match frac {
    Some(f) if !f.is_empty() => {
      format!(
        "{}.{f}",
        if int_digits.is_empty() {
          "0"
        } else {
          &int_digits
        }
      )
    }
    _ => int_digits,
  };
  Decimal::from_str(&normalized).map_err(|e| format!("'{text}': {e}"))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn dec(s: &str) -> Decimal {
    s.parse().unwrap()
  }

  fn amt(s: &str) -> Amount {
    let mut i = s;
    let a = amount(&mut i).unwrap();
    assert!(i.is_empty(), "leftover '{i}'");
    a
  }

  #[test]
  fn amounts() {
    let a = amt("-84.10 PLN");
    assert_eq!(a.quantity, dec("-84.10"));
    assert_eq!(a.quantity.scale(), 2);
    assert_eq!(a.commodity, "PLN");
    assert!(!a.style.commodity_left);
    assert_eq!(amt("$-5").quantity, dec("-5"));
    assert_eq!(amt("-$5").quantity, dec("-5"));
    assert!(amt("$5").style.commodity_left);
    assert_eq!(amt("1,234.50 PLN").quantity, dec("1234.50"));
    assert_eq!(amt("1,234,567 PLN").quantity, dec("1234567"));
    assert_eq!(amt("10PLN").commodity, "PLN");
    assert_eq!(amt("\"ACME 1\" 3").commodity, "ACME 1");
    assert_eq!(amt("0").commodity, "");
  }

  #[test]
  fn rejects_ambiguous_numbers() {
    for s in ["84,10 PLN", "1,000 PLN", "1.000,50 PLN", "12,34.5"] {
      let mut i = s;
      assert!(amount(&mut i).is_err(), "{s} should fail");
    }
  }

  #[test]
  fn amount_display_keeps_precision() {
    assert_eq!(amt("-84.10 PLN").to_string(), "-84.10 PLN");
    assert_eq!(amt("$-5.5").to_string(), "-$5.5");
    assert_eq!(amt("10PLN").to_string(), "10PLN");
  }

  #[test]
  fn header() {
    let t = parse_header("2026/10/27 * (42) Biedronka  ; weekly shop").unwrap();
    assert_eq!(t.date, NaiveDate::from_ymd_opt(2026, 10, 27).unwrap());
    assert_eq!(t.status, Status::Cleared);
    assert_eq!(t.code.as_deref(), Some("42"));
    assert_eq!(t.description, "Biedronka");
    assert_eq!(t.comment.as_deref(), Some("weekly shop"));
    assert_eq!(parse_header("2026.1.5 Kebab").unwrap().description, "Kebab");
    assert!(parse_header("2026-10-27=2026-10-28 x").is_err());
    assert!(parse_header("2026-13-01 x").is_err());
    assert!(parse_header("2026-10/01 x").is_err());
  }

  #[test]
  fn postings() {
    let p = parse_posting("! assets:my bank  -84.10 PLN @ 1 PLN = 100 PLN ; tag:x").unwrap();
    assert_eq!(p.status, Status::Pending);
    assert_eq!(p.account, "assets:my bank");
    assert_eq!(p.amount.unwrap().quantity, dec("-84.10"));
    assert!(matches!(p.cost, Some(Cost::Unit(_))));
    assert_eq!(p.assertion.unwrap().quantity, dec("100"));
    assert_eq!(p.comment.as_deref(), Some("tag:x"));
    let p = parse_posting("expenses:food\t\t12 PLN").unwrap();
    assert_eq!(p.account, "expenses:food");
    assert!(parse_posting("assets:cash").unwrap().amount.is_none());
    assert!(parse_posting("expenses:food\t12 PLN").is_err());
    assert!(parse_posting("(budget)  5 PLN").is_err());
    assert!(parse_posting("[budget]  5 PLN").is_err());
    assert!(parse_posting("assets:cash  = 5 PLN").is_err());
    assert!(parse_posting("assets:cash  5 PLN == 5 PLN").is_err());
  }

  #[test]
  fn rejects_unsupported() {
    let e = parse("; hi\ninclude other.journal\n", None).unwrap_err();
    assert_eq!(e.line, 2);
    assert!(e.message.contains("include"));
    assert!(parse("~ monthly\n  a  1\n", None).is_err());
    assert!(parse("= expenses\n  a  1\n", None).is_err());
    assert!(parse("P 2026-01-01 EUR 4.3 PLN\n", None).is_err());
    let e = parse("2026-01-01 x\n  a  1 PLN\n  b  -1 EUR\n", None).unwrap_err();
    assert_eq!(e.line, 3);
    let e = parse("2026-01-01 x\n  a  1 PLN\n  b\n", Some("EUR")).unwrap_err();
    assert_eq!(e.line, 2);
    assert!(parse("2026-01-01 x\n  a\n  b\n", None).is_err());
  }

  #[test]
  fn journal_structure() {
    let text = "; top\naccount assets:checking\n\n2026-01-01 Open\n    ; note\n    assets:checking  10 PLN\n      ; posting note\n    equity\n";
    let j = parse(text, None).unwrap();
    assert_eq!(j.items.len(), 4);
    assert_eq!(j.commodity.as_deref(), Some("PLN"));
    let t = j.transaction(3).unwrap();
    assert_eq!(t.line, 4);
    assert_eq!(t.comment_lines, vec!["note"]);
    assert_eq!(t.postings[0].comment_lines, vec!["posting note"]);
    assert_eq!(t.inferred_amount().unwrap().quantity, dec("-10"));
    assert_eq!(j.accounts(), vec!["assets:checking", "equity"]);
  }
}
