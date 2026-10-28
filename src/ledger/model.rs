use chrono::NaiveDate;
use rust_decimal::Decimal;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Status {
  #[default]
  Unmarked,
  Pending,
  Cleared,
}

impl Status {
  pub fn symbol(self) -> &'static str {
    match self {
      Status::Unmarked => "",
      Status::Pending => "!",
      Status::Cleared => "*",
    }
  }
}

/// How an amount was written, so regenerated text looks like the source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AmountStyle {
  pub commodity_left: bool,
  pub spaced: bool,
}

impl Default for AmountStyle {
  fn default() -> Self {
    AmountStyle {
      commodity_left: false,
      spaced: true,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
  pub quantity: Decimal,
  pub commodity: String,
  pub style: AmountStyle,
}

impl Amount {
  pub fn new(quantity: Decimal, commodity: &str) -> Self {
    Amount {
      quantity,
      commodity: commodity.to_string(),
      style: AmountStyle::default(),
    }
  }

  pub fn with_quantity(&self, quantity: Decimal) -> Self {
    Amount {
      quantity,
      ..self.clone()
    }
  }
}

fn quote_commodity(c: &str) -> String {
  let plain = c
    .chars()
    .all(|ch| !ch.is_ascii_digit() && !ch.is_whitespace() && !"-+.,;@=\"*!".contains(ch));
  if plain {
    c.to_string()
  } else {
    format!("\"{c}\"")
  }
}

impl fmt::Display for Amount {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    if self.commodity.is_empty() {
      return write!(f, "{}", self.quantity);
    }
    let c = quote_commodity(&self.commodity);
    let sp = if self.style.spaced { " " } else { "" };
    if self.style.commodity_left {
      if self.quantity.is_sign_negative() {
        write!(f, "-{c}{sp}{}", self.quantity.abs())
      } else {
        write!(f, "{c}{sp}{}", self.quantity)
      }
    } else {
      write!(f, "{}{sp}{c}", self.quantity)
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cost {
  Unit(Amount),
  Total(Amount),
}

impl Cost {
  pub fn amount(&self) -> &Amount {
    match self {
      Cost::Unit(a) | Cost::Total(a) => a,
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Posting {
  pub status: Status,
  pub account: String,
  pub amount: Option<Amount>,
  pub cost: Option<Cost>,
  pub assertion: Option<Amount>,
  /// Same-line comment, without the leading `;`.
  pub comment: Option<String>,
  /// Following indented comment lines, without the leading `;`.
  pub comment_lines: Vec<String>,
  /// 1-based source line, 0 for postings not read from a file.
  pub line: usize,
}

impl Posting {
  pub fn new(account: &str, amount: Option<Amount>) -> Self {
    Posting {
      account: account.to_string(),
      amount,
      ..Default::default()
    }
  }

  pub fn is_simple(&self) -> bool {
    self.status == Status::Unmarked
      && self.cost.is_none()
      && self.assertion.is_none()
      && self.comment.is_none()
      && self.comment_lines.is_empty()
  }

  /// The amount this posting contributes to the transaction balance, in cost terms.
  pub fn balancing_amount(&self) -> Option<Amount> {
    let amt = self.amount.as_ref()?;
    Some(match &self.cost {
      None => amt.clone(),
      Some(Cost::Unit(c)) => c.with_quantity(c.quantity * amt.quantity),
      Some(Cost::Total(c)) => {
        let q = if amt.quantity.is_sign_negative() {
          -c.quantity.abs()
        } else {
          c.quantity.abs()
        };
        c.with_quantity(q)
      }
    })
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
  pub date: NaiveDate,
  pub status: Status,
  pub code: Option<String>,
  pub description: String,
  pub comment: Option<String>,
  pub comment_lines: Vec<String>,
  pub postings: Vec<Posting>,
  pub line: usize,
}

impl Transaction {
  pub fn new(date: NaiveDate, description: &str) -> Self {
    Transaction {
      date,
      status: Status::Unmarked,
      code: None,
      description: description.to_string(),
      comment: None,
      comment_lines: Vec::new(),
      postings: Vec::new(),
      line: 0,
    }
  }

  /// Amount of the single elided posting, if exactly one is elided.
  pub fn inferred_amount(&self) -> Option<Amount> {
    let mut elided = self.postings.iter().filter(|p| p.amount.is_none());
    elided.next()?;
    if elided.next().is_some() {
      return None;
    }
    let mut sum = Decimal::ZERO;
    let mut template: Option<Amount> = None;
    for a in self.postings.iter().filter_map(|p| p.balancing_amount()) {
      sum += a.quantity;
      template.get_or_insert(a);
    }
    template.map(|t| t.with_quantity(-sum))
  }

  /// Every posting with its amount filled in (elided amount inferred).
  pub fn resolved_postings(&self) -> Vec<(&Posting, Option<Amount>)> {
    let inferred = self.inferred_amount();
    self
      .postings
      .iter()
      .map(|p| (p, p.amount.clone().or_else(|| inferred.clone())))
      .collect()
  }

  /// Searchable text: description, comments and account names.
  pub fn matches(&self, needle: &str) -> bool {
    let n = needle.to_lowercase();
    let hit = |s: &str| s.to_lowercase().contains(&n);
    hit(&self.description)
      || self.comment.as_deref().is_some_and(hit)
      || self.comment_lines.iter().any(|c| hit(c))
      || self.postings.iter().any(|p| hit(&p.account))
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
  /// A raw source line including its line terminator.
  Raw(String),
  /// A transaction; `src` holds its original text until it is modified.
  Txn {
    txn: Transaction,
    src: Option<String>,
  },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Journal {
  pub items: Vec<Item>,
  /// The single commodity used by this journal.
  pub commodity: Option<String>,
}

impl Journal {
  pub fn transactions(&self) -> impl Iterator<Item = (usize, &Transaction)> {
    self
      .items
      .iter()
      .enumerate()
      .filter_map(|(i, it)| match it {
        Item::Txn { txn, .. } => Some((i, txn)),
        Item::Raw(_) => None,
      })
  }

  pub fn transaction(&self, index: usize) -> Option<&Transaction> {
    match self.items.get(index) {
      Some(Item::Txn { txn, .. }) => Some(txn),
      _ => None,
    }
  }

  /// Account names used in postings or declared with `account`, sorted.
  pub fn accounts(&self) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for it in &self.items {
      match it {
        Item::Txn { txn, .. } => {
          for p in &txn.postings {
            set.insert(p.account.clone());
          }
        }
        Item::Raw(line) => {
          if let Some(rest) = line.strip_prefix("account ") {
            let name = rest.split(&[';', '\t'][..]).next().unwrap_or("");
            let name = name.split("  ").next().unwrap_or("").trim();
            if !name.is_empty() {
              set.insert(name.to_string());
            }
          }
        }
      }
    }
    set.into_iter().collect()
  }

  /// Descriptions, most recently used first, without duplicates.
  pub fn payees(&self) -> Vec<String> {
    let mut txns: Vec<&Transaction> = self.transactions().map(|(_, t)| t).collect();
    txns.sort_by_key(|t| std::cmp::Reverse(t.date));
    let mut seen = std::collections::HashSet::new();
    txns
      .into_iter()
      .filter(|t| seen.insert(t.description.clone()))
      .map(|t| t.description.clone())
      .collect()
  }

  /// Most recent transaction with this description.
  pub fn last_with_payee(&self, payee: &str) -> Option<&Transaction> {
    self
      .transactions()
      .map(|(_, t)| t)
      .filter(|t| t.description == payee)
      .max_by_key(|t| t.date)
  }

  pub fn commodity_or_default(&self) -> String {
    self.commodity.clone().unwrap_or_default()
  }
}

impl Journal {
  /// Display style and precision for the journal commodity, taken from the source amounts.
  pub fn display_style(&self) -> (AmountStyle, u32) {
    let mut style = None;
    let mut scale = 0;
    for (_, t) in self.transactions() {
      for p in &t.postings {
        if let Some(a) = &p.amount {
          style.get_or_insert(a.style);
          scale = scale.max(a.quantity.scale());
        }
      }
    }
    (style.unwrap_or_default(), scale)
  }

  /// Format a quantity in the journal commodity, e.g. `-84.10 PLN` or `1,234.50 PLN`.
  pub fn format(&self, q: Decimal, grouped: bool) -> String {
    let (style, scale) = self.display_style();
    let n = format_number(q.abs(), scale, grouped);
    let sign = if q.is_sign_negative() && !q.round_dp(scale).is_zero() {
      "-"
    } else {
      ""
    };
    let amount = Amount {
      quantity: Decimal::ZERO,
      commodity: self.commodity_or_default(),
      style,
    };
    let c = quote_commodity(&amount.commodity);
    let sp = if style.spaced { " " } else { "" };
    if amount.commodity.is_empty() {
      format!("{sign}{n}")
    } else if style.commodity_left {
      format!("{sign}{c}{sp}{n}")
    } else {
      format!("{sign}{n}{sp}{c}")
    }
  }
}

/// Format a non-negative number with a fixed number of decimals.
pub fn format_number(q: Decimal, scale: u32, grouped: bool) -> String {
  let mut q = q.round_dp(scale);
  q.rescale(scale);
  let s = q.to_string();
  if !grouped {
    return s;
  }
  let (int, frac) = s
    .split_once('.')
    .map(|(a, b)| (a, Some(b)))
    .unwrap_or((&s, None));
  let mut out = String::new();
  for (k, ch) in int.chars().enumerate() {
    if k > 0 && (int.len() - k) % 3 == 0 {
      out.push(',');
    }
    out.push(ch);
  }
  if let Some(f) = frac {
    out.push('.');
    out.push_str(f);
  }
  out
}
