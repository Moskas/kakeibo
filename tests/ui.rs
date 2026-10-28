//! Snapshot tests of rendered screens.

use chrono::NaiveDate;
use kakeibo::app::{App, Message};
use kakeibo::ledger::parser;
use kakeibo::ui;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::path::PathBuf;

fn app() -> App {
  let text = std::fs::read_to_string("tests/fixtures/sample.journal").unwrap();
  let journal = parser::parse(&text, None).unwrap();
  let today = NaiveDate::from_ymd_opt(2026, 10, 28).unwrap();
  App::new(PathBuf::from("/finance/main.journal"), journal, text, today)
}

fn keys(app: &mut App, keys: &str) {
  for c in keys.chars() {
    let code = match c {
      '\n' => KeyCode::Enter,
      '\t' => KeyCode::Tab,
      c => KeyCode::Char(c),
    };
    app.update(Message::Key(KeyEvent::new(code, KeyModifiers::NONE)));
  }
}

fn render(app: &App) -> String {
  let mut terminal = Terminal::new(TestBackend::new(100, 32)).unwrap();
  terminal.draw(|f| ui::view(app, f)).unwrap();
  let buf = terminal.backend().buffer();
  let mut out = String::new();
  for y in 0..buf.area.height {
    let line: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
    out.push_str(line.trim_end());
    out.push('\n');
  }
  out
}

#[test]
fn dashboard() {
  insta::assert_snapshot!(render(&app()));
}

#[test]
fn transactions() {
  let mut a = app();
  keys(&mut a, "2j");
  insta::assert_snapshot!(render(&a));
}

#[test]
fn accounts() {
  let mut a = app();
  keys(&mut a, "3");
  insta::assert_snapshot!(render(&a));
}

#[test]
fn reports() {
  let mut a = app();
  keys(&mut a, "4");
  insta::assert_snapshot!(render(&a));
}

#[test]
fn add_form_with_autocomplete() {
  let mut a = app();
  keys(&mut a, "aLi");
  insta::assert_snapshot!(render(&a));
}

#[test]
fn help_overlay() {
  let mut a = app();
  keys(&mut a, "?");
  insta::assert_snapshot!(render(&a));
}
