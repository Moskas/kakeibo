use anyhow::{Context, Result};
use chrono::Local;
use clap::Parser;
use kakeibo::app::App;
use kakeibo::cli::{self, Cli};
use kakeibo::config::KeyMap;
use kakeibo::ledger::{self, model::Journal};
use kakeibo::{event, ui};
use std::sync::mpsc;

fn main() -> Result<()> {
  let cli = Cli::parse();
  match &cli.command {
    Some(cmd) => cli::run(&cli, cmd),
    None => run_tui(&cli),
  }
}

fn run_tui(cli: &Cli) -> Result<()> {
  let config = cli::load_config(cli)?;
  let keymap = KeyMap::new(&config.keys)?;
  let path = config.journal_path(cli.file.as_deref());
  let commodity = config.commodity.as_deref();
  // everything that can fail happens before the terminal is taken over
  let (journal, text) = if path.exists() {
    ledger::load(&path, commodity)?
  } else {
    (
      Journal {
        items: vec![],
        commodity: commodity.map(str::to_string),
      },
      String::new(),
    )
  };

  let mut app = App::new(path.clone(), journal, text, Local::now().date_naive());
  app.keymap = keymap;
  app.commodity = commodity.map(str::to_string).or(app.commodity);

  let (tx, rx) = mpsc::channel();
  let _watcher = event::watch(&path, tx.clone()).context("watching journal")?;
  event::spawn_input(tx);

  // ratatui::init installs a panic hook that restores the terminal
  let mut terminal = ratatui::init();
  let result = (|| -> Result<()> {
    while !app.should_quit {
      terminal.draw(|f| {
        app_page(&mut app, f.area().height);
        ui::view(&app, f)
      })?;
      let msg = rx.recv()?;
      app.update(msg);
      while let Ok(msg) = rx.try_recv() {
        app.update(msg);
      }
    }
    Ok(())
  })();
  ratatui::restore();
  result
}

/// Page size for PageUp/PageDown, from the terminal height.
fn app_page(app: &mut App, height: u16) {
  app.page = (height as usize).saturating_sub(12).max(1);
}
