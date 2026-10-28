use kakeibo::ledger::model::Item;
use kakeibo::ledger::{ops, parser, writer};
use std::fs;
use std::path::PathBuf;

fn fixtures() -> Vec<PathBuf> {
  let mut paths: Vec<PathBuf> = fs::read_dir("tests/fixtures")
    .unwrap()
    .map(|e| e.unwrap().path())
    .filter(|p| p.extension().is_some_and(|e| e == "journal"))
    .collect();
  paths.sort();
  assert!(!paths.is_empty());
  paths
}

#[test]
fn parse_then_write_is_identical() {
  for path in fixtures() {
    let text = fs::read_to_string(&path).unwrap();
    let journal = parser::parse(&text, None).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(writer::write(&journal), text, "{}", path.display());
  }
}

#[test]
fn rendered_transactions_reparse_to_same_model() {
  for path in fixtures() {
    let text = fs::read_to_string(&path).unwrap();
    let mut journal = parser::parse(&text, None).unwrap();
    let before: Vec<_> = journal.transactions().map(|(_, t)| t.clone()).collect();
    for item in &mut journal.items {
      if let Item::Txn { src, .. } = item {
        *src = None;
      }
    }
    let rewritten = writer::write(&journal);
    let again = parser::parse(&rewritten, None).unwrap();
    let after: Vec<_> = again.transactions().map(|(_, t)| t.clone()).collect();
    assert_eq!(before.len(), after.len());
    for (mut a, mut b) in before.into_iter().zip(after) {
      a.line = 0;
      b.line = 0;
      for p in a.postings.iter_mut().chain(b.postings.iter_mut()) {
        p.line = 0;
        // rendering normalizes amount spacing, everything else must survive
        for amt in [p.amount.as_mut(), p.assertion.as_mut()]
          .into_iter()
          .flatten()
        {
          amt.style.spaced = false;
        }
      }
      assert_eq!(a, b, "{}", path.display());
    }
  }
}

#[test]
fn editing_one_transaction_changes_only_its_span() {
  let text = fs::read_to_string("tests/fixtures/basic.journal").unwrap();
  let journal = parser::parse(&text, None).unwrap();
  let (index, txn) = journal.transactions().nth(2).unwrap();
  let Item::Txn {
    src: Some(original),
    ..
  } = &journal.items[index]
  else {
    panic!()
  };

  let mut edited = txn.clone();
  edited.description = "Lidl".into();
  let mut journal2 = journal.clone();
  ops::apply(
    &mut journal2,
    &ops::replace_transaction(&journal, index, edited.clone()).unwrap(),
  );
  let out = writer::write(&journal2);

  let pos = text.find(original.as_str()).unwrap();
  let expected = format!(
    "{}{}{}",
    &text[..pos],
    writer::render_transaction(&edited, "\n"),
    &text[pos + original.len()..]
  );
  assert_eq!(out, expected);
  assert!(out.contains("2026-09-10 ! Lidl\n"));
}

#[test]
fn crlf_is_preserved_for_new_transactions() {
  let text = fs::read_to_string("tests/fixtures/crlf.journal").unwrap();
  let mut journal = parser::parse(&text, None).unwrap();
  let txn = journal.transactions().next().unwrap().1.clone();
  let splice = ops::add_transaction(&journal, txn).unwrap();
  ops::apply(&mut journal, &splice);
  let out = writer::write(&journal);
  assert!(out.starts_with(&text));
  assert!(!out[text.len()..].replace("\r\n", "").contains('\n'));
}

#[test]
fn atomic_save_writes_backup_and_keeps_mode() {
  use std::os::unix::fs::PermissionsExt;
  let dir = tempfile::tempdir().unwrap();
  let path = dir.path().join("main.journal");
  fs::write(&path, "old\n").unwrap();
  fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
  writer::save_atomic(&path, "new\n").unwrap();
  assert_eq!(fs::read_to_string(&path).unwrap(), "new\n");
  assert_eq!(
    fs::read_to_string(dir.path().join("main.journal.bak")).unwrap(),
    "old\n"
  );
  assert_eq!(
    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
    0o640
  );
}
