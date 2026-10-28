use crate::analysis::balance::{AccountKind, balances_as_of, balances_between, rollup};
use crate::analysis::series::Month;
use crate::config::{Action, KeyMap};
use crate::ledger::model::{Journal, Transaction};
use crate::ledger::ops::{self, Draft, Problem};
use crate::ledger::undo::UndoStack;
use crate::ledger::{parser, writer};
use crate::ui::command::{self, Command};
use crate::ui::form::{FormOutcome, FormState};
use crate::ui::widgets::Input;
use chrono::NaiveDate;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use rust_decimal::Decimal;
use std::collections::BTreeSet;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
  Dashboard,
  Transactions,
  Accounts,
  Reports,
}

impl View {
  pub const ALL: [View; 4] = [
    View::Dashboard,
    View::Transactions,
    View::Accounts,
    View::Reports,
  ];

  pub fn title(self) -> &'static str {
    match self {
      View::Dashboard => "Dashboard",
      View::Transactions => "Transactions",
      View::Accounts => "Accounts",
      View::Reports => "Reports",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
  Delete(usize),
  Quit,
}

#[derive(Debug, Clone)]
pub struct Conflict {
  pub theirs: String,
  pub show_diff: bool,
  pub scroll: u16,
}

#[derive(Debug, Clone)]
pub struct FilterForm {
  pub fields: [Input; 4],
  pub focus: usize,
  pub error: Option<String>,
}

impl FilterForm {
  pub const LABELS: [&'static str; 4] = ["Account", "Payee", "From", "To"];
}

#[derive(Debug, Clone)]
pub struct AccountForm {
  pub input: Input,
  pub error: Option<String>,
}
#[derive(Debug, Clone)]
pub enum Mode {
  Normal,
  Command(Input),
  Search(Input),
  Form(Box<FormState>),
  Filter(FilterForm),
  AddAccount(AccountForm),
  Confirm(Confirm),
  Help,
  Conflict(Conflict),
}

#[derive(Debug, Clone)]
pub enum Message {
  Key(KeyEvent),
  Resize,
  FileChanged,
}

#[derive(Debug, Clone)]
pub struct Status {
  pub text: String,
  pub error: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TxFilter {
  pub account: String,
  pub payee: String,
  pub from: Option<NaiveDate>,
  pub to: Option<NaiveDate>,
}

impl TxFilter {
  pub fn accepts(&self, t: &Transaction) -> bool {
    let acct = self.account.to_lowercase();
    (acct.is_empty()
      || t
        .postings
        .iter()
        .any(|p| p.account.to_lowercase().contains(&acct)))
      && (self.payee.is_empty()
        || t
          .description
          .to_lowercase()
          .contains(&self.payee.to_lowercase()))
      && self.from.is_none_or(|d| t.date >= d)
      && self.to.is_none_or(|d| t.date <= d)
  }

  /// The month this filter covers exactly, if any.
  pub fn month(&self) -> Option<Month> {
    let (from, to) = (self.from?, self.to?);
    let m = Month::of(from);
    (from == m.first_day() && to == m.last_day()).then_some(m)
  }

  pub fn describe(&self) -> String {
    let mut parts = Vec::new();
    if !self.account.is_empty() {
      parts.push(self.account.clone());
    }
    if !self.payee.is_empty() {
      parts.push(format!("payee:{}", self.payee));
    }
    match (self.month(), self.from, self.to) {
      (Some(m), _, _) => parts.push(m.long_name()),
      (None, Some(f), Some(t)) => parts.push(format!("{f}..{t}")),
      (None, Some(f), None) => parts.push(format!("from {f}")),
      (None, None, Some(t)) => parts.push(format!("to {t}")),
      _ => {}
    }
    if parts.is_empty() {
      "all".into()
    } else {
      parts.join(" \u{b7} ")
    }
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRow {
  pub name: String,
  pub depth: usize,
  pub balance: Decimal,
  pub has_children: bool,
  pub expanded: bool,
}

pub struct App {
  pub path: PathBuf,
  pub journal: Journal,
  pub commodity: Option<String>,
  /// File contents as last read or written, to detect external changes and our own writes.
  pub on_disk: String,
  pub dirty: bool,
  pub undo: UndoStack,
  pub view: View,
  pub mode: Mode,
  pub today: NaiveDate,
  pub period: Month,
  pub status: Option<Status>,
  pub should_quit: bool,
  pub keymap: KeyMap,
  pub problems: Vec<Problem>,

  pub filter: TxFilter,
  pub tx_rows: Vec<usize>,
  pub tx_selected: usize,
  pub search: Option<String>,

  pub expanded: BTreeSet<String>,
  pub acct_rows: Vec<AccountRow>,
  pub acct_selected: usize,
  pub page: usize,
  pub pending_reload: bool,
}

impl App {
  pub fn new(path: PathBuf, journal: Journal, on_disk: String, today: NaiveDate) -> App {
    let mut app = App {
      path,
      commodity: journal.commodity.clone(),
      journal,
      on_disk,
      dirty: false,
      undo: UndoStack::default(),
      view: View::Dashboard,
      mode: Mode::Normal,
      today,
      period: Month::of(today),
      status: None,
      should_quit: false,
      keymap: KeyMap::default(),
      problems: Vec::new(),
      filter: TxFilter::default(),
      tx_rows: Vec::new(),
      tx_selected: 0,
      search: None,
      expanded: BTreeSet::new(),
      acct_rows: Vec::new(),
      acct_selected: 0,
      page: 10,
      pending_reload: false,
    };
    app.expanded = app
      .journal
      .accounts()
      .iter()
      .filter_map(|a| a.split(':').next())
      .map(str::to_string)
      .collect();
    app.journal_changed();
    if !app.problems.is_empty() {
      app.error(format!(
        "{} problem(s), first: {}",
        app.problems.len(),
        app.problems[0]
      ));
    }
    app
  }

  pub fn info(&mut self, text: impl Into<String>) {
    self.status = Some(Status {
      text: text.into(),
      error: false,
    });
  }

  pub fn error(&mut self, text: impl Into<String>) {
    self.status = Some(Status {
      text: text.into(),
      error: true,
    });
  }

  /// Recompute everything derived from the journal.
  pub fn journal_changed(&mut self) {
    self.dirty = writer::write(&self.journal) != self.on_disk;
    self.problems = ops::check(&self.journal);
    self.refresh_rows();
  }

  pub fn refresh_rows(&mut self) {
    let selected_item = self.tx_rows.get(self.tx_selected).copied();
    let mut rows: Vec<(usize, &Transaction)> = self
      .journal
      .transactions()
      .filter(|(_, t)| self.filter.accepts(t))
      .collect();
    rows.sort_by(|a, b| b.1.date.cmp(&a.1.date).then(b.0.cmp(&a.0)));
    self.tx_rows = rows.into_iter().map(|(i, _)| i).collect();
    if let Some(pos) = selected_item.and_then(|i| self.tx_rows.iter().position(|r| *r == i)) {
      self.tx_selected = pos;
    }
    self.tx_selected = self.tx_selected.min(self.tx_rows.len().saturating_sub(1));
    self.acct_rows = self.account_rows();
    self.acct_selected = self
      .acct_selected
      .min(self.acct_rows.len().saturating_sub(1));
  }

  /// Account tree: balance-sheet accounts as of the period end, income and expenses year to date.
  fn account_rows(&self) -> Vec<AccountRow> {
    let end = self.period.last_day();
    let year_start = NaiveDate::from_ymd_opt(self.period.year, 1, 1).expect("valid date");
    let mut flat = balances_as_of(&self.journal, Some(end));
    flat.retain(|a, _| {
      !matches!(
        AccountKind::of(a),
        AccountKind::Income | AccountKind::Expense
      )
    });
    for (a, q) in balances_between(&self.journal, year_start, end) {
      if matches!(
        AccountKind::of(&a),
        AccountKind::Income | AccountKind::Expense
      ) {
        flat.insert(a, q);
      }
    }
    for a in self.journal.accounts() {
      flat.entry(a).or_default();
    }
    let tree = rollup(&flat);
    // sort by segments so children follow their parent even when a sibling like `assets2` exists
    let mut names: Vec<&String> = tree.keys().collect();
    names.sort_by(|a, b| a.split(':').cmp(b.split(':')));
    let parents: BTreeSet<&str> = names.iter().flat_map(|n| ancestors(n)).collect();
    let mut rows = Vec::new();
    for name in names {
      let visible = ancestors(name).all(|p| self.expanded.contains(p));
      if !visible {
        continue;
      }
      rows.push(AccountRow {
        name: name.clone(),
        depth: name.matches(':').count(),
        balance: tree[name],
        has_children: parents.contains(name.as_str()),
        expanded: self.expanded.contains(name),
      });
    }
    rows
  }

  pub fn selected_transaction(&self) -> Option<(usize, &Transaction)> {
    let idx = *self.tx_rows.get(self.tx_selected)?;
    self.journal.transaction(idx).map(|t| (idx, t))
  }

  pub fn update(&mut self, msg: Message) {
    match msg {
      Message::Resize => {}
      Message::FileChanged => self.file_changed(),
      Message::Key(key) => {
        if !matches!(self.mode, Mode::Command(_) | Mode::Search(_)) {
          self.status = None;
        }
        self.handle_key(key);
        // a change deferred while a form or delete prompt held item indices
        if self.pending_reload && matches!(self.mode, Mode::Normal) {
          self.pending_reload = false;
          self.file_changed();
        }
      }
    }
  }

  fn handle_key(&mut self, key: KeyEvent) {
    let mode = std::mem::replace(&mut self.mode, Mode::Normal);
    self.mode = match mode {
      Mode::Normal => {
        self.normal_key(&key);
        return;
      }
      Mode::Help => Mode::Normal,
      Mode::Command(mut input) => match key.code {
        KeyCode::Esc => Mode::Normal,
        KeyCode::Enter => {
          self.run_command(&input.value);
          return;
        }
        KeyCode::Backspace if input.value.is_empty() => Mode::Normal,
        _ => {
          input.handle(&key);
          Mode::Command(input)
        }
      },
      Mode::Search(mut input) => match key.code {
        KeyCode::Esc => Mode::Normal,
        KeyCode::Enter => {
          let q = input.value.trim().to_string();
          if !q.is_empty() {
            self.search = Some(q);
            self.view = View::Transactions;
            self.search_step(true, true);
          }
          Mode::Normal
        }
        _ => {
          input.handle(&key);
          Mode::Search(input)
        }
      },
      Mode::Form(mut form) => match form.handle(&key, &self.journal) {
        FormOutcome::Continue => Mode::Form(form),
        FormOutcome::Cancel => Mode::Normal,
        FormOutcome::Save(txn) => {
          self.save_form(form.editing, txn);
          Mode::Normal
        }
      },
      Mode::Filter(mut ff) => self.filter_key(&key, &mut ff).unwrap_or(Mode::Filter(ff)),
      Mode::AddAccount(mut af) => match key.code {
        KeyCode::Esc => Mode::Normal,
        KeyCode::Enter => match self.add_account(&af.input.value) {
          Ok(()) => Mode::Normal,
          Err(e) => {
            af.error = Some(e);
            Mode::AddAccount(af)
          }
        },
        _ => {
          af.input.handle(&key);
          af.error = None;
          Mode::AddAccount(af)
        }
      },
      Mode::Confirm(c) => match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
          match c {
            Confirm::Delete(idx) => self.delete(idx),
            Confirm::Quit => self.should_quit = true,
          }
          Mode::Normal
        }
        KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') => Mode::Normal,
        _ => Mode::Confirm(c),
      },
      Mode::Conflict(mut c) => match key.code {
        KeyCode::Char('k') if !c.show_diff => {
          self.on_disk = c.theirs;
          self.journal_changed();
          self.info("Kept your changes; :w will overwrite the file on disk");
          Mode::Normal
        }
        KeyCode::Char('r') => {
          self.reload_from(c.theirs);
          Mode::Normal
        }
        KeyCode::Char('d') => {
          c.show_diff = !c.show_diff;
          Mode::Conflict(c)
        }
        KeyCode::Esc if c.show_diff => {
          c.show_diff = false;
          Mode::Conflict(c)
        }
        KeyCode::Char('j') | KeyCode::Down if c.show_diff => {
          c.scroll = c.scroll.saturating_add(1);
          Mode::Conflict(c)
        }
        KeyCode::Char('k') | KeyCode::Up if c.show_diff => {
          c.scroll = c.scroll.saturating_sub(1);
          Mode::Conflict(c)
        }
        _ => Mode::Conflict(c),
      },
    };
  }

  fn normal_key(&mut self, key: &KeyEvent) {
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
      self.quit(false);
      return;
    }
    let Some(action) = self.keymap.action(key) else {
      return;
    };
    match action {
      Action::Quit => self.quit(false),
      Action::Dashboard => self.view = View::Dashboard,
      Action::Transactions => self.view = View::Transactions,
      Action::Accounts => self.view = View::Accounts,
      Action::Reports => self.view = View::Reports,
      Action::Down => self.move_selection(1),
      Action::Up => self.move_selection(-1),
      Action::PageDown => self.move_selection(self.page as isize),
      Action::PageUp => self.move_selection(-(self.page as isize)),
      Action::Top => self.move_selection(isize::MIN / 2),
      Action::Bottom => self.move_selection(isize::MAX / 2),
      Action::Open => self.open(),
      Action::AddAccount => {
        // start under the selected account's branch
        let prefix = match (self.view, self.acct_rows.get(self.acct_selected)) {
          (View::Accounts, Some(r)) if r.has_children => format!("{}:", r.name),
          (View::Accounts, Some(r)) => match r.name.rsplit_once(':') {
            Some((parent, _)) => format!("{parent}:"),
            None => String::new(),
          },
          _ => String::new(),
        };
        self.mode = Mode::AddAccount(AccountForm {
          input: Input::new(&prefix),
          error: None,
        });
      }
      Action::Add => {
        let draft = Draft {
          date: self.today.format("%Y-%m-%d").to_string(),
          ..Default::default()
        };
        self.mode = Mode::Form(Box::new(FormState::new(&draft, None, &self.journal)));
      }
      Action::Edit => self.edit_selected(),
      Action::Delete => match self.selected_in_view() {
        Some(idx) => self.mode = Mode::Confirm(Confirm::Delete(idx)),
        None => self.error("No transaction selected"),
      },
      Action::Search => self.mode = Mode::Search(Input::default()),
      Action::SearchNext => self.search_step(true, false),
      Action::SearchPrev => self.search_step(false, false),
      Action::Filter => {
        let f = &self.filter;
        let date = |d: Option<NaiveDate>| d.map(|d| d.to_string()).unwrap_or_default();
        self.mode = Mode::Filter(FilterForm {
          fields: [
            Input::new(&f.account),
            Input::new(&f.payee),
            Input::new(&date(f.from)),
            Input::new(&date(f.to)),
          ],
          focus: 0,
          error: None,
        });
      }
      Action::PrevPeriod => self.set_period(self.period.prev()),
      Action::NextPeriod => self.set_period(self.period.next()),
      Action::Undo => {
        if self.undo.undo(&mut self.journal) {
          self.journal_changed();
          self.info("Undone");
        } else {
          self.error("Nothing to undo");
        }
      }
      Action::Redo => {
        if self.undo.redo(&mut self.journal) {
          self.journal_changed();
          self.info("Redone");
        } else {
          self.error("Nothing to redo");
        }
      }
      Action::Command => self.mode = Mode::Command(Input::default()),
      Action::Help => self.mode = Mode::Help,
    }
  }

  fn selected_in_view(&self) -> Option<usize> {
    match self.view {
      View::Transactions => self.selected_transaction().map(|(i, _)| i),
      _ => None,
    }
  }

  fn move_selection(&mut self, delta: isize) {
    let (sel, len) = match self.view {
      View::Transactions => (&mut self.tx_selected, self.tx_rows.len()),
      View::Accounts => (&mut self.acct_selected, self.acct_rows.len()),
      _ => return,
    };
    if len == 0 {
      return;
    }
    *sel = (*sel as isize)
      .saturating_add(delta)
      .clamp(0, len as isize - 1) as usize;
  }

  fn open(&mut self) {
    match self.view {
      View::Accounts => {
        let Some(row) = self.acct_rows.get(self.acct_selected).cloned() else {
          return;
        };
        if row.has_children {
          if !self.expanded.remove(&row.name) {
            self.expanded.insert(row.name);
          }
          self.acct_rows = self.account_rows();
        } else {
          self.filter = TxFilter {
            account: row.name,
            ..Default::default()
          };
          self.tx_selected = 0;
          self.refresh_rows();
          self.view = View::Transactions;
        }
      }
      View::Transactions => self.edit_selected(),
      _ => {}
    }
  }

  fn edit_selected(&mut self) {
    let Some(idx) = self.selected_in_view() else {
      self.error("Select a transaction in the Transactions view first");
      return;
    };
    let txn = self
      .journal
      .transaction(idx)
      .expect("selected row is a transaction");
    match ops::draft_of(txn) {
      Ok(draft) => {
        self.mode = Mode::Form(Box::new(FormState::new(&draft, Some(idx), &self.journal)))
      }
      Err(e) => self.error(e),
    }
  }

  fn save_form(&mut self, editing: Option<usize>, txn: Transaction) {
    let result = match editing {
      Some(idx) => {
        let original = self
          .journal
          .transaction(idx)
          .expect("edited transaction exists");
        let merged = ops::merge_draft(original, txn);
        if without_lines(&merged) == without_lines(original) {
          self.info("No changes");
          return;
        }
        ops::replace_transaction(&self.journal, idx, merged)
      }
      None => ops::add_transaction(&self.journal, txn),
    };
    match result {
      Ok(splice) => {
        let target = splice.index + splice.inserted.len() - 1;
        self.undo.perform(&mut self.journal, splice);
        self.journal_changed();
        if let Some(pos) = self.tx_rows.iter().position(|r| *r == target) {
          self.tx_selected = pos;
        }
        self.info(if editing.is_some() {
          "Edited (:w to save)"
        } else {
          "Added (:w to save)"
        });
      }
      Err(e) => self.error(e),
    }
  }

  fn add_account(&mut self, name: &str) -> Result<(), String> {
    let splice = ops::add_account(&self.journal, name)?;
    let name = name.trim().to_string();
    self.undo.perform(&mut self.journal, splice);
    self.expanded.extend(ancestors(&name).map(str::to_string));
    self.journal_changed();
    if let Some(pos) = self.acct_rows.iter().position(|r| r.name == name) {
      self.acct_selected = pos;
    }
    self.view = View::Accounts;
    self.info(format!("Added account {name} (:w to save)"));
    Ok(())
  }

  fn delete(&mut self, idx: usize) {
    match ops::delete_transaction(&self.journal, idx) {
      Ok(splice) => {
        self.undo.perform(&mut self.journal, splice);
        self.journal_changed();
        self.info("Deleted (u to undo, :w to save)");
      }
      Err(e) => self.error(e),
    }
  }

  fn set_period(&mut self, m: Month) {
    if self.view == View::Transactions
      && let Some(cur) = self.filter.month()
    {
      let shifted = if m > self.period {
        cur.next()
      } else {
        cur.prev()
      };
      self.filter.from = Some(shifted.first_day());
      self.filter.to = Some(shifted.last_day());
    }
    self.period = m;
    self.refresh_rows();
  }

  fn search_step(&mut self, forward: bool, include_current: bool) {
    let Some(q) = self.search.clone() else {
      self.error("No previous search");
      return;
    };
    let n = self.tx_rows.len();
    if n == 0 {
      self.error(format!("No match for '{q}'"));
      return;
    }
    for k in 0..n {
      let step = if include_current { k } else { k + 1 };
      let pos = if forward {
        (self.tx_selected + step) % n
      } else {
        (self.tx_selected + n * 2 - step) % n
      };
      if self
        .journal
        .transaction(self.tx_rows[pos])
        .is_some_and(|t| t.matches(&q))
      {
        self.tx_selected = pos;
        self.view = View::Transactions;
        return;
      }
    }
    self.error(format!("No match for '{q}'"));
  }

  fn filter_key(&mut self, key: &KeyEvent, ff: &mut FilterForm) -> Option<Mode> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
      KeyCode::Esc => return Some(Mode::Normal),
      KeyCode::Tab | KeyCode::Down => ff.focus = (ff.focus + 1) % 4,
      KeyCode::BackTab | KeyCode::Up => ff.focus = (ff.focus + 3) % 4,
      KeyCode::Char('p') if ctrl => {
        ff.fields[2].set(&self.period.first_day().to_string());
        ff.fields[3].set(&self.period.last_day().to_string());
      }
      KeyCode::Char('l') if ctrl => ff.fields.iter_mut().for_each(|f| f.set("")),
      KeyCode::Enter => {
        let date = |s: &str| {
          if s.trim().is_empty() {
            Ok(None)
          } else {
            ops::parse_date(s).map(Some)
          }
        };
        match (date(&ff.fields[2].value), date(&ff.fields[3].value)) {
          (Ok(from), Ok(to)) => {
            self.filter = TxFilter {
              account: ff.fields[0].value.trim().to_string(),
              payee: ff.fields[1].value.trim().to_string(),
              from,
              to,
            };
            self.tx_selected = 0;
            self.refresh_rows();
            self.view = View::Transactions;
            return Some(Mode::Normal);
          }
          (Err(e), _) | (_, Err(e)) => ff.error = Some(e),
        }
      }
      _ => {
        ff.fields[ff.focus].handle(key);
      }
    }
    None
  }

  fn run_command(&mut self, line: &str) {
    match command::parse(line) {
      Ok(Command::Write { force }) => {
        self.write(force);
      }
      Ok(Command::Quit { force }) => self.quit(force),
      Ok(Command::WriteQuit { force }) => {
        if self.write(force) {
          self.should_quit = true;
        }
      }
      Ok(Command::Reload { force }) => {
        if self.dirty && !force {
          self.error("Unsaved changes; use :reload! to discard them");
        } else {
          match std::fs::read_to_string(&self.path) {
            Ok(text) => self.reload_from(text),
            Err(e) => self.error(format!("Reload failed: {e}")),
          }
        }
      }
      Ok(Command::Goto(m)) => {
        self.period = m;
        self.refresh_rows();
        self.info(format!("Period {}", m.long_name()));
      }
      Err(e) => self.error(e),
    }
  }

  fn quit(&mut self, force: bool) {
    if self.dirty && !force {
      self.mode = Mode::Confirm(Confirm::Quit);
    } else {
      self.should_quit = true;
    }
  }

  /// Save the journal. Returns true on success.
  /// Save the journal. Unless forced, refuses when the file changed since it was read.
  pub fn write(&mut self, force: bool) -> bool {
    if !force
      && let Ok(theirs) = std::fs::read_to_string(&self.path)
      && theirs != self.on_disk
    {
      self.mode = Mode::Conflict(Conflict {
        theirs,
        show_diff: false,
        scroll: 0,
      });
      self.error("Journal changed on disk; not written (:w! to overwrite)");
      return false;
    }
    let text = writer::write(&self.journal);
    match writer::save_atomic(&self.path, &text) {
      Ok(()) => {
        self.on_disk = text;
        self.dirty = false;
        self.info(format!("Wrote {}", self.path.display()));
        true
      }
      Err(e) => {
        self.error(format!("Write failed: {e:#}"));
        false
      }
    }
  }

  fn reload_from(&mut self, text: String) {
    match parser::parse(&text, self.commodity.as_deref()) {
      Ok(j) => {
        self.journal = j;
        self.on_disk = text;
        self.undo.clear();
        self.journal_changed();
        self.info("Reloaded from disk");
      }
      Err(e) => self.error(format!("Reload failed: {e}")),
    }
  }

  fn file_changed(&mut self) {
    let Ok(text) = std::fs::read_to_string(&self.path) else {
      return;
    };
    if text == self.on_disk {
      return;
    }
    if let Mode::Conflict(c) = &mut self.mode {
      c.theirs = text;
      return;
    }
    if matches!(self.mode, Mode::Form(_) | Mode::Confirm(Confirm::Delete(_))) {
      // these hold item indices; handle the change once they close
      self.pending_reload = true;
      self.error("Journal changed on disk; it will be handled when this closes");
      return;
    }
    if !self.dirty {
      self.reload_from(text);
      return;
    }
    self.mode = Mode::Conflict(Conflict {
      theirs: text,
      show_diff: false,
      scroll: 0,
    });
  }
}

/// A transaction with source line numbers cleared, for content comparison.
fn without_lines(t: &Transaction) -> Transaction {
  let mut t = t.clone();
  t.line = 0;
  t.postings.iter_mut().for_each(|p| p.line = 0);
  t
}

fn ancestors(name: &str) -> impl Iterator<Item = &str> {
  name.match_indices(':').map(move |(i, _)| &name[..i])
}

#[cfg(test)]
mod tests {
  use super::*;

  const TEXT: &str = "2026-09-01 Opening\n  assets:bank  100.00 PLN\n  equity\n\n\
                      2026-10-02 Shop\n  expenses:food  25 PLN\n  assets:bank\n";

  fn app() -> App {
    let j = parser::parse(TEXT, None).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
    App::new(
      PathBuf::from("/nonexistent/test.journal"),
      j,
      TEXT.to_string(),
      today,
    )
  }

  fn press(app: &mut App, c: KeyCode) {
    app.update(Message::Key(KeyEvent::new(c, KeyModifiers::NONE)));
  }

  fn typ(app: &mut App, s: &str) {
    s.chars().for_each(|c| press(app, KeyCode::Char(c)));
  }

  #[test]
  fn delete_undo_redo_tracks_dirty() {
    let mut a = app();
    press(&mut a, KeyCode::Char('2'));
    assert_eq!(a.tx_rows.len(), 2);
    press(&mut a, KeyCode::Char('d'));
    press(&mut a, KeyCode::Char('y'));
    assert_eq!(a.tx_rows.len(), 1);
    assert!(a.dirty);
    press(&mut a, KeyCode::Char('u'));
    assert_eq!(a.tx_rows.len(), 2);
    assert!(!a.dirty);
    a.update(Message::Key(KeyEvent::new(
      KeyCode::Char('r'),
      KeyModifiers::CONTROL,
    )));
    assert!(a.dirty);
  }

  #[test]
  fn quit_asks_when_dirty() {
    let mut a = app();
    press(&mut a, KeyCode::Char('q'));
    assert!(a.should_quit);
    let mut a = app();
    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Char('d'));
    press(&mut a, KeyCode::Char('y'));
    typ(&mut a, ":q");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.mode, Mode::Confirm(Confirm::Quit)));
    press(&mut a, KeyCode::Char('n'));
    typ(&mut a, ":q!");
    press(&mut a, KeyCode::Enter);
    assert!(a.should_quit);
  }

  #[test]
  fn search_and_filter() {
    let mut a = app();
    typ(&mut a, "/open");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.view, View::Transactions);
    assert_eq!(a.selected_transaction().unwrap().1.description, "Opening");
    press(&mut a, KeyCode::Char('f'));
    typ(&mut a, "food");
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.tx_rows.len(), 1);
  }

  #[test]
  fn account_tree_expands() {
    let mut a = app();
    press(&mut a, KeyCode::Char('3'));
    let names: Vec<_> = a.acct_rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
      names,
      vec![
        "assets",
        "assets:bank",
        "equity",
        "expenses",
        "expenses:food"
      ]
    );
    press(&mut a, KeyCode::Enter);
    assert_eq!(a.acct_rows.len(), 4);
  }

  #[test]
  fn external_change_on_clean_buffer_reloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.journal");
    std::fs::write(&path, TEXT).unwrap();
    let mut a = app();
    a.path = path.clone();
    std::fs::write(
      &path,
      format!("{TEXT}\n2026-10-03 New\n  expenses:food  1 PLN\n  assets:bank\n"),
    )
    .unwrap();
    a.update(Message::FileChanged);
    assert_eq!(a.journal.transactions().count(), 3);

    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Char('d'));
    press(&mut a, KeyCode::Char('y'));
    std::fs::write(&path, TEXT).unwrap();
    a.update(Message::FileChanged);
    assert!(matches!(a.mode, Mode::Conflict(_)));
    press(&mut a, KeyCode::Char('k'));
    assert!(a.write(false));
    a.update(Message::FileChanged);
    assert!(matches!(a.mode, Mode::Normal));
  }

  #[test]
  fn reload_waits_for_open_form() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.journal");
    std::fs::write(&path, TEXT).unwrap();
    let mut a = app();
    a.path = path.clone();
    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Char('j'));
    press(&mut a, KeyCode::Char('e'));
    assert!(matches!(a.mode, Mode::Form(_)));

    let prepended = format!("2026-01-01 Prepended\n  expenses:x  1 PLN\n  assets:bank\n\n{TEXT}");
    std::fs::write(&path, &prepended).unwrap();
    a.update(Message::FileChanged);
    assert!(matches!(a.mode, Mode::Form(_)));
    a.update(Message::Key(KeyEvent::new(
      KeyCode::Char('s'),
      KeyModifiers::CONTROL,
    )));
    // unchanged form closes, then the deferred reload picks up the new file
    assert!(matches!(a.mode, Mode::Normal));
    assert_eq!(a.journal.transactions().count(), 3);
    assert_eq!(writer::write(&a.journal), prepended);
  }

  #[test]
  fn write_refuses_when_disk_changed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("j.journal");
    std::fs::write(&path, TEXT).unwrap();
    let mut a = app();
    a.path = path.clone();
    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Char('d'));
    press(&mut a, KeyCode::Char('y'));
    std::fs::write(&path, "; external\n").unwrap();
    typ(&mut a, ":w");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.mode, Mode::Conflict(_)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "; external\n");
    press(&mut a, KeyCode::Char('k'));
    typ(&mut a, ":w");
    press(&mut a, KeyCode::Enter);
    assert!(!a.dirty);
    assert_eq!(
      std::fs::read_to_string(&path).unwrap(),
      writer::write(&a.journal)
    );
  }

  #[test]
  fn saving_untouched_edit_keeps_source() {
    let mut a = app();
    press(&mut a, KeyCode::Char('2'));
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.mode, Mode::Form(_)));
    a.update(Message::Key(KeyEvent::new(
      KeyCode::Char('s'),
      KeyModifiers::CONTROL,
    )));
    assert!(!a.dirty);
    assert_eq!(writer::write(&a.journal), TEXT);
  }

  #[test]
  fn account_tree_with_similar_names() {
    let text = "2026-10-01 x\n  assets:a  1 PLN\n  assets2  1 PLN\n  equity\n";
    let j = parser::parse(text, None).unwrap();
    let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
    let a = App::new(PathBuf::from("/nonexistent/j"), j, text.to_string(), today);
    let rows: Vec<_> = a
      .acct_rows
      .iter()
      .map(|r| (r.name.as_str(), r.has_children))
      .collect();
    assert_eq!(
      rows,
      vec![
        ("assets", true),
        ("assets:a", false),
        ("assets2", false),
        ("equity", false)
      ]
    );
  }

  #[test]
  fn add_account_from_accounts_view() {
    let mut a = app();
    press(&mut a, KeyCode::Char('3'));
    // selection on `assets`, which has children, so the prompt starts with `assets:`
    press(&mut a, KeyCode::Char('A'));
    typ(&mut a, "cash");
    press(&mut a, KeyCode::Enter);
    assert!(matches!(a.mode, Mode::Normal));
    assert!(a.dirty);
    assert_eq!(a.acct_rows[a.acct_selected].name, "assets:cash");
    assert!(writer::write(&a.journal).starts_with("account assets:cash\n\n2026-09-01"));

    press(&mut a, KeyCode::Char('A'));
    a.update(Message::Key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL)));
    typ(&mut a, "assets:cash");
    press(&mut a, KeyCode::Enter);
    let Mode::AddAccount(af) = &a.mode else {
      panic!("prompt should stay open")
    };
    assert!(af.error.as_deref().unwrap().contains("already exists"));
    press(&mut a, KeyCode::Esc);

    press(&mut a, KeyCode::Char('u'));
    assert!(!a.dirty);
    assert!(a.acct_rows.iter().all(|r| r.name != "assets:cash"));
  }
}
