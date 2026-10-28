use super::model::{Cost, Item, Journal, Status, Transaction};
use anyhow::{Context, Result};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// Serialize the journal. Untouched spans are copied verbatim.
pub fn write(journal: &Journal) -> String {
  let eol = line_ending(journal);
  let mut out = String::new();
  for item in &journal.items {
    match item {
      Item::Raw(s) => out.push_str(s),
      Item::Txn { src: Some(s), .. } => out.push_str(s),
      Item::Txn { txn, src: None } => out.push_str(&render_transaction(txn, eol)),
    }
  }
  out
}

pub fn line_ending(journal: &Journal) -> &'static str {
  let crlf = journal.items.iter().any(|it| match it {
    Item::Raw(s) | Item::Txn { src: Some(s), .. } => s.ends_with("\r\n"),
    _ => false,
  });
  if crlf { "\r\n" } else { "\n" }
}

/// Render a transaction in hledger syntax, terminated by `eol`.
pub fn render_transaction(txn: &Transaction, eol: &str) -> String {
  let mut out = txn.date.format("%Y-%m-%d").to_string();
  if txn.status != Status::Unmarked {
    out.push(' ');
    out.push_str(txn.status.symbol());
  }
  if let Some(code) = &txn.code {
    out.push_str(&format!(" ({code})"));
  }
  if !txn.description.is_empty() {
    out.push(' ');
    out.push_str(&txn.description);
  }
  if let Some(c) = &txn.comment {
    out.push_str(&format!("  ; {c}"));
  }
  out.push_str(eol);
  for c in &txn.comment_lines {
    out.push_str(&format!("    ; {c}{eol}"));
  }

  let label = |p: &super::model::Posting| match p.status {
    Status::Unmarked => p.account.clone(),
    s => format!("{} {}", s.symbol(), p.account),
  };
  let acct_w = txn
    .postings
    .iter()
    .map(|p| label(p).chars().count())
    .max()
    .unwrap_or(0);
  let amounts: Vec<String> = txn
    .postings
    .iter()
    .map(|p| p.amount.as_ref().map(|a| a.to_string()).unwrap_or_default())
    .collect();
  let amt_w = amounts.iter().map(|a| a.chars().count()).max().unwrap_or(0);

  for (p, amt) in txn.postings.iter().zip(&amounts) {
    let mut line = format!("    {:<acct_w$}", label(p));
    if !amt.is_empty() {
      line.push_str(&format!("  {amt:>amt_w$}"));
    }
    match &p.cost {
      Some(Cost::Unit(c)) => line.push_str(&format!(" @ {c}")),
      Some(Cost::Total(c)) => line.push_str(&format!(" @@ {c}")),
      None => {}
    }
    if let Some(a) = &p.assertion {
      line.push_str(&format!(" = {a}"));
    }
    if let Some(c) = &p.comment {
      line.push_str(&format!("  ; {c}"));
    }
    out.push_str(line.trim_end());
    out.push_str(eol);
    for c in &p.comment_lines {
      out.push_str(&format!("      ; {c}{eol}"));
    }
  }
  out
}

/// Directory containing `path`; `.` for a bare file name.
pub fn parent_dir(path: &Path) -> PathBuf {
  match path.parent() {
    Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
    _ => PathBuf::from("."),
  }
}

/// Write `contents` to `path` atomically, keeping a `.bak` copy of the previous file.
pub fn save_atomic(path: &Path, contents: &str) -> Result<()> {
  let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
  let dir = parent_dir(&target);

  let old_perms = fs::metadata(&target).ok().map(|m| m.permissions());
  if target.exists() {
    let mut bak = target.clone().into_os_string();
    bak.push(".bak");
    fs::copy(&target, &bak)
      .with_context(|| format!("creating backup {}", bak.to_string_lossy()))?;
  }

  let mut tmp = tempfile::NamedTempFile::new_in(&dir)
    .with_context(|| format!("creating temp file in {}", dir.display()))?;
  tmp.write_all(contents.as_bytes())?;
  tmp.as_file().sync_all()?;
  if let Some(p) = old_perms {
    fs::set_permissions(tmp.path(), p)?;
  }
  tmp
    .persist(&target)
    .with_context(|| format!("replacing {}", target.display()))?;
  if let Ok(d) = fs::File::open(&dir) {
    let _ = d.sync_all();
  }
  Ok(())
}
