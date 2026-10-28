use crate::analysis::balance::{balances_as_of, rollup};
use crate::analysis::series::Month;
use crate::config::Config;
use crate::ledger::model::Journal;
use crate::ledger::{self, ops, writer};
use anyhow::{Result, bail};
use chrono::{Local, NaiveDate};
use clap::{Parser, Subcommand};
use rust_decimal::Decimal;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "kakeibo", version, about = "TUI and CLI for hledger journals")]
pub struct Cli {
  /// Journal file (default: $LEDGER_FILE, config `journal`, ~/.hledger.journal)
  #[arg(short, long, global = true)]
  pub file: Option<PathBuf>,
  /// Config file (default: ~/.config/kakeibo/config.toml)
  #[arg(long, global = true)]
  pub config: Option<PathBuf>,
  #[command(subcommand)]
  pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
  /// Add a transaction; missing fields are prompted for
  Add {
    #[arg(short, long)]
    date: Option<String>,
    #[arg(short, long)]
    payee: Option<String>,
    /// Account receiving the amount, e.g. expenses:groceries
    #[arg(short, long)]
    account: Option<String>,
    /// Amount for --account; a bare number uses the journal commodity
    #[arg(short = 'm', long, allow_hyphen_values = true)]
    amount: Option<String>,
    /// Balancing account, e.g. assets:checking
    #[arg(short = 'g', long)]
    against: Option<String>,
    #[arg(short, long)]
    note: Option<String>,
    /// Print the transaction without writing it
    #[arg(long)]
    dry_run: bool,
  },
  /// Show account balances
  Bal {
    /// Only accounts containing this text (case-insensitive)
    account: Option<String>,
    /// Show parent accounts with rolled-up balances
    #[arg(long)]
    tree: bool,
    /// Balances as of the end of this month (YYYY-MM)
    #[arg(long)]
    end: Option<String>,
  },
  /// Show postings with a running total
  Reg {
    /// Only postings to accounts containing this text (case-insensitive)
    account: Option<String>,
  },
  /// Validate the journal and report balance assertion failures
  Check,
}

pub fn load_config(cli: &Cli) -> Result<Config> {
  match &cli.config {
    Some(p) => Config::from_file(p),
    None => Config::load(),
  }
}

pub fn run(cli: &Cli, command: &Command) -> Result<()> {
  let config = load_config(cli)?;
  let path = config.journal_path(cli.file.as_deref());
  let commodity = config.commodity.as_deref();

  if let Command::Check = command {
    let journal = match ledger::load(&path, commodity) {
      Ok((j, _)) => j,
      Err(e) => bail!("{e:#}"),
    };
    let problems = ops::check(&journal);
    for p in &problems {
      eprintln!("{}:{}: {}", path.display(), p.line, p.message);
    }
    if !problems.is_empty() {
      bail!("{} problem(s) found", problems.len());
    }
    println!("ok: {} transactions", journal.transactions().count());
    return Ok(());
  }

  if let Command::Add {
    date,
    payee,
    account,
    amount,
    against,
    note,
    dry_run,
  } = command
  {
    let (journal, _) = load_or_empty(&path, commodity)?;
    let mut prompt = Prompter::new();
    let today = Local::now().date_naive().format("%Y-%m-%d").to_string();
    let draft = ops::Draft {
      date: prompt.field("Date", date, Some(&today))?,
      payee: prompt.field("Payee", payee, None)?,
      postings: vec![
        (
          prompt.field("Account", account, None)?,
          prompt.field("Amount", amount, None)?,
        ),
        (prompt.field("Against", against, None)?, String::new()),
      ],
      note: prompt.field("Note", note, Some(""))?,
    };
    let txn = ops::build_valid(&draft, &journal).map_err(anyhow::Error::msg)?;
    let splice = ops::add_transaction(&journal, txn).map_err(anyhow::Error::msg)?;
    let mut journal = journal;
    ops::apply(&mut journal, &splice);
    if let Some(ledger::model::Item::Txn { txn, .. }) = journal.items.last() {
      print!("{}", writer::render_transaction(txn, "\n"));
    }
    if !dry_run {
      writer::save_atomic(&path, &writer::write(&journal))?;
      eprintln!("Saved to {}", path.display());
    }
    return Ok(());
  }

  let (journal, _) = ledger::load(&path, commodity)?;
  let mut out = std::io::stdout().lock();
  match command {
    Command::Bal { account, tree, end } => {
      let end = match end {
        Some(m) => Some(
          Month::parse(m)
            .ok_or_else(|| anyhow::anyhow!("bad month '{m}'"))?
            .last_day(),
        ),
        None => None,
      };
      write!(
        out,
        "{}",
        balance_report(&journal, account.as_deref(), *tree, end)
      )?;
    }
    Command::Reg { account } => write!(out, "{}", register_report(&journal, account.as_deref()))?,
    Command::Add { .. } | Command::Check => unreachable!(),
  }
  Ok(())
}

fn load_or_empty(path: &Path, commodity: Option<&str>) -> Result<(Journal, String)> {
  if path.exists() {
    ledger::load(path, commodity)
  } else {
    Ok((
      Journal {
        items: vec![],
        commodity: commodity.map(str::to_string),
      },
      String::new(),
    ))
  }
}

struct Prompter {
  interactive: bool,
}

impl Prompter {
  fn new() -> Self {
    Prompter {
      interactive: std::io::stdin().is_terminal(),
    }
  }

  fn field(
    &mut self,
    label: &str,
    given: &Option<String>,
    default: Option<&str>,
  ) -> Result<String> {
    if let Some(v) = given {
      return Ok(v.clone());
    }
    if !self.interactive {
      return match default {
        Some(d) => Ok(d.to_string()),
        None => bail!("missing --{}", label.to_lowercase()),
      };
    }
    match default {
      Some(d) if !d.is_empty() => eprint!("{label} [{d}]: "),
      _ => eprint!("{label}: "),
    }
    std::io::stderr().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let line = line.trim().to_string();
    Ok(if line.is_empty() {
      default.unwrap_or("").to_string()
    } else {
      line
    })
  }
}

fn matches(account: &str, filter: Option<&str>) -> bool {
  filter.is_none_or(|f| account.to_lowercase().contains(&f.to_lowercase()))
}

/// hledger-style balance report.
pub fn balance_report(
  journal: &Journal,
  filter: Option<&str>,
  tree: bool,
  end: Option<NaiveDate>,
) -> String {
  let flat = balances_as_of(journal, end);
  let rows: Vec<(String, Decimal)> = if tree {
    rollup(
      &flat
        .into_iter()
        .filter(|(a, _)| matches(a, filter))
        .collect(),
    )
    .into_iter()
    .filter(|(_, q)| !q.is_zero())
    .map(|(a, q)| {
      let depth = a.matches(':').count();
      let leaf = a.rsplit(':').next().unwrap_or(&a).to_string();
      (format!("{}{leaf}", "  ".repeat(depth)), q)
    })
    .collect()
  } else {
    flat
      .into_iter()
      .filter(|(a, q)| !q.is_zero() && matches(a, filter))
      .collect()
  };

  let total: Decimal = if tree {
    rows
      .iter()
      .filter(|(a, _)| !a.starts_with(' '))
      .map(|(_, q)| *q)
      .sum()
  } else {
    rows.iter().map(|(_, q)| *q).sum()
  };
  let fmt = |q: Decimal| {
    if q.is_zero() {
      "0".to_string()
    } else {
      journal.format(q, false)
    }
  };
  let width = rows
    .iter()
    .map(|(_, q)| fmt(*q).len())
    .chain([fmt(total).len(), 20])
    .max()
    .unwrap();
  let mut out = String::new();
  for (a, q) in &rows {
    out.push_str(&format!("{:>width$}  {a}\n", fmt(*q)));
  }
  out.push_str(&format!("{}\n{:>width$}\n", "-".repeat(width), fmt(total)));
  out
}

fn fit(s: &str, w: usize) -> String {
  if s.chars().count() <= w {
    format!("{s:<w$}")
  } else {
    let mut t: String = s.chars().take(w.saturating_sub(2)).collect();
    t.push_str("..");
    t
  }
}

/// hledger-style register report.
pub fn register_report(journal: &Journal, filter: Option<&str>) -> String {
  let mut txns: Vec<_> = journal.transactions().map(|(_, t)| t).collect();
  txns.sort_by_key(|t| t.date);
  let mut total = Decimal::ZERO;
  let mut out = String::new();
  for t in txns {
    let mut first = true;
    for (p, amt) in t.resolved_postings() {
      if !matches(&p.account, filter) {
        continue;
      }
      let q = amt.map(|a| a.quantity).unwrap_or_default();
      total += q;
      let head = if first {
        format!("{} {}", t.date.format("%Y-%m-%d"), fit(&t.description, 20))
      } else {
        " ".repeat(31)
      };
      first = false;
      let tot = if total.is_zero() {
        "0".to_string()
      } else {
        journal.format(total, false)
      };
      out.push_str(&format!(
        "{head} {} {:>12} {:>12}\n",
        fit(&p.account, 22),
        journal.format(q, false),
        tot
      ));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ledger::parser::parse;

  const TEXT: &str = "2026-01-01 Opening balance\n  assets:bank  100.00 PLN\n  equity\n\n\
                      2026-01-02 Shop\n  expenses:food  25 PLN\n  assets:bank\n";

  #[test]
  fn bal_flat_and_tree() {
    let j = parse(TEXT, None).unwrap();
    assert_eq!(
      balance_report(&j, None, false, None),
      "           75.00 PLN  assets:bank\n         -100.00 PLN  equity\n           25.00 PLN  \
       expenses:food\n--------------------\n                   0\n"
    );
    let tree = balance_report(&j, Some("food"), true, None);
    assert_eq!(
      tree,
      "           25.00 PLN  expenses\n           25.00 PLN    food\n--------------------\n           25.00 PLN\n"
    );
  }

  #[test]
  fn reg_running_total() {
    let j = parse(TEXT, None).unwrap();
    let r = register_report(&j, Some("bank"));
    let lines: Vec<&str> = r.lines().collect();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("2026-01-01 Opening balance      assets:bank"));
    assert!(lines[1].ends_with("-25.00 PLN    75.00 PLN"));
  }
}
