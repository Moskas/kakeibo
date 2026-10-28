use anyhow::{Context, Result};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// `~/.config/kakeibo/config.toml`
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
  pub journal: Option<PathBuf>,
  pub commodity: Option<String>,
  /// Action name to key, e.g. `add = "a"` or `redo = "ctrl-r"`.
  pub keys: HashMap<String, String>,
}

pub fn config_path() -> Option<PathBuf> {
  let base = std::env::var_os("XDG_CONFIG_HOME")
    .map(PathBuf::from)
    .or_else(|| home().map(|h| h.join(".config")))?;
  Some(base.join("kakeibo").join("config.toml"))
}

fn home() -> Option<PathBuf> {
  std::env::var_os("HOME").map(PathBuf::from)
}

impl Config {
  pub fn load() -> Result<Config> {
    match config_path() {
      Some(p) if p.exists() => Config::from_file(&p),
      _ => Ok(Config::default()),
    }
  }

  pub fn from_file(path: &Path) -> Result<Config> {
    let text = std::fs::read_to_string(path)?;
    toml::from_str(&text).with_context(|| format!("reading config {}", path.display()))
  }

  /// `--file`, then `$LEDGER_FILE`, then the config file, then `~/.hledger.journal`.
  pub fn journal_path(&self, flag: Option<&Path>) -> PathBuf {
    if let Some(p) = flag {
      return p.to_path_buf();
    }
    if let Some(p) = std::env::var_os("LEDGER_FILE").filter(|p| !p.is_empty()) {
      return expand_tilde(Path::new(&p));
    }
    if let Some(p) = &self.journal {
      return expand_tilde(p);
    }
    home().unwrap_or_default().join(".hledger.journal")
  }
}

fn expand_tilde(p: &Path) -> PathBuf {
  match (p.strip_prefix("~"), home()) {
    (Ok(rest), Some(h)) => h.join(rest),
    _ => p.to_path_buf(),
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
  Quit,
  Dashboard,
  Transactions,
  Accounts,
  Reports,
  Down,
  Up,
  Top,
  Bottom,
  PageDown,
  PageUp,
  Open,
  Add,
  AddAccount,
  Edit,
  Delete,
  Search,
  SearchNext,
  SearchPrev,
  Filter,
  PrevPeriod,
  NextPeriod,
  Undo,
  Redo,
  Command,
  Help,
}

/// (action, config name, description, default keys)
const BINDINGS: &[(Action, &str, &str, &[&str])] = &[
  (Action::Dashboard, "dashboard", "Dashboard view", &["1"]),
  (
    Action::Transactions,
    "transactions",
    "Transactions view",
    &["2"],
  ),
  (Action::Accounts, "accounts", "Accounts view", &["3"]),
  (Action::Reports, "reports", "Reports view", &["4"]),
  (Action::Down, "down", "Move down", &["j", "down"]),
  (Action::Up, "up", "Move up", &["k", "up"]),
  (Action::Top, "top", "First row", &["g", "home"]),
  (Action::Bottom, "bottom", "Last row", &["G", "end"]),
  (
    Action::PageDown,
    "page_down",
    "Page down",
    &["ctrl-d", "pagedown"],
  ),
  (Action::PageUp, "page_up", "Page up", &["ctrl-u", "pageup"]),
  (Action::Open, "open", "Open or expand", &["enter", "space"]),
  (Action::Add, "add", "Add transaction", &["a"]),
  (Action::AddAccount, "add_account", "Add account", &["A"]),
  (Action::Edit, "edit", "Edit transaction", &["e"]),
  (Action::Delete, "delete", "Delete transaction", &["d"]),
  (Action::Search, "search", "Search", &["/"]),
  (Action::SearchNext, "search_next", "Next match", &["n"]),
  (Action::SearchPrev, "search_prev", "Previous match", &["N"]),
  (Action::Filter, "filter", "Filter transactions", &["f"]),
  (Action::PrevPeriod, "prev_period", "Previous period", &["["]),
  (Action::NextPeriod, "next_period", "Next period", &["]"]),
  (Action::Undo, "undo", "Undo", &["u"]),
  (Action::Redo, "redo", "Redo", &["ctrl-r"]),
  (Action::Command, "command", "Command line", &[":"]),
  (Action::Help, "help", "Help", &["?"]),
  (Action::Quit, "quit", "Quit", &["q"]),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
  pub code: KeyCode,
  pub ctrl: bool,
}

impl Key {
  pub fn parse(s: &str) -> Option<Key> {
    let (ctrl, name) = match s.strip_prefix("ctrl-").or_else(|| s.strip_prefix("C-")) {
      Some(rest) => (true, rest),
      None => (false, s),
    };
    let code = match name.to_lowercase().as_str() {
      "enter" | "return" => KeyCode::Enter,
      "space" => KeyCode::Char(' '),
      "tab" => KeyCode::Tab,
      "esc" | "escape" => KeyCode::Esc,
      "up" => KeyCode::Up,
      "down" => KeyCode::Down,
      "left" => KeyCode::Left,
      "right" => KeyCode::Right,
      "home" => KeyCode::Home,
      "end" => KeyCode::End,
      "pageup" => KeyCode::PageUp,
      "pagedown" => KeyCode::PageDown,
      "backspace" => KeyCode::Backspace,
      _ => {
        let mut chars = name.chars();
        let c = chars.next()?;
        if chars.next().is_some() {
          return None;
        }
        KeyCode::Char(if ctrl { c.to_ascii_lowercase() } else { c })
      }
    };
    Some(Key { code, ctrl })
  }

  pub fn from_event(ev: &KeyEvent) -> Key {
    let ctrl = ev.modifiers.contains(KeyModifiers::CONTROL);
    let code = match ev.code {
      KeyCode::Char(c) if ctrl => KeyCode::Char(c.to_ascii_lowercase()),
      c => c,
    };
    Key { code, ctrl }
  }

  pub fn label(&self) -> String {
    let name = match self.code {
      KeyCode::Enter => "Enter".into(),
      KeyCode::Char(' ') => "Space".into(),
      KeyCode::Up => "Up".into(),
      KeyCode::Down => "Down".into(),
      KeyCode::Home => "Home".into(),
      KeyCode::End => "End".into(),
      KeyCode::PageUp => "PgUp".into(),
      KeyCode::PageDown => "PgDn".into(),
      KeyCode::Tab => "Tab".into(),
      KeyCode::Esc => "Esc".into(),
      KeyCode::Char(c) => c.to_string(),
      other => format!("{other:?}"),
    };
    if self.ctrl {
      format!("Ctrl-{name}")
    } else {
      name
    }
  }
}

#[derive(Debug, Clone)]
pub struct KeyMap {
  bindings: Vec<(Action, Vec<Key>)>,
}

impl Default for KeyMap {
  fn default() -> Self {
    KeyMap::new(&HashMap::new()).expect("default keymap is valid")
  }
}

impl KeyMap {
  /// Defaults, with each action named in `overrides` rebound to the given keys
  /// (comma-separated, e.g. `"x, ctrl-x"`).
  pub fn new(overrides: &HashMap<String, String>) -> Result<KeyMap> {
    for name in overrides.keys() {
      if !BINDINGS.iter().any(|(_, n, _, _)| n == name) {
        anyhow::bail!("unknown key action '{name}' in config");
      }
    }
    let mut bindings = Vec::new();
    for (action, name, _, defaults) in BINDINGS {
      let keys: Vec<Key> = match overrides.get(*name) {
        Some(spec) => spec
          .split(',')
          .map(|k| {
            Key::parse(k.trim()).ok_or_else(|| anyhow::anyhow!("invalid key '{k}' for {name}"))
          })
          .collect::<Result<_>>()?,
        None => defaults
          .iter()
          .map(|k| Key::parse(k).expect("valid default key"))
          .collect(),
      };
      bindings.push((*action, keys));
    }
    Ok(KeyMap { bindings })
  }

  pub fn action(&self, ev: &KeyEvent) -> Option<Action> {
    let key = Key::from_event(ev);
    self
      .bindings
      .iter()
      .find(|(_, keys)| keys.contains(&key))
      .map(|(a, _)| *a)
  }

  pub fn label(&self, action: Action) -> String {
    self
      .bindings
      .iter()
      .find(|(a, _)| *a == action)
      .and_then(|(_, keys)| keys.first())
      .map(Key::label)
      .unwrap_or_default()
  }

  pub fn describe(&self) -> Vec<(String, &'static str)> {
    BINDINGS
      .iter()
      .zip(&self.bindings)
      .map(|((_, _, desc, _), (_, keys))| {
        (
          keys.iter().map(Key::label).collect::<Vec<_>>().join(" / "),
          *desc,
        )
      })
      .collect()
  }
}
#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_config() {
    let c: Config = toml::from_str(
      "journal = \"~/finance/main.journal\"\ncommodity = \"PLN\"\n[keys]\nadd = \"n\"\n",
    )
    .unwrap();
    assert_eq!(c.commodity.as_deref(), Some("PLN"));
    assert_eq!(c.keys["add"], "n");
    assert!(toml::from_str::<Config>("bogus = 1").is_err());
  }
}
