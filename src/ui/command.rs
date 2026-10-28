use crate::analysis::series::Month;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
  Write { force: bool },
  Quit { force: bool },
  WriteQuit { force: bool },
  Reload { force: bool },
  Goto(Month),
}

/// Parse a `:` command line (without the colon).
pub fn parse(line: &str) -> Result<Command, String> {
  let mut words = line.split_whitespace();
  let cmd = words.next().unwrap_or("");
  let arg = words.next();
  if words.next().is_some() {
    return Err(format!("Too many arguments: {line}"));
  }
  Ok(match (cmd, arg) {
    ("w" | "write", None) => Command::Write { force: false },
    ("w!" | "write!", None) => Command::Write { force: true },
    ("q" | "quit", None) => Command::Quit { force: false },
    ("q!" | "quit!", None) => Command::Quit { force: true },
    ("wq" | "x", None) => Command::WriteQuit { force: false },
    ("wq!" | "x!", None) => Command::WriteQuit { force: true },
    ("reload" | "e", None) => Command::Reload { force: false },
    ("reload!" | "e!", None) => Command::Reload { force: true },
    ("goto", Some(m)) => {
      Command::Goto(Month::parse(m).ok_or(format!("Invalid month '{m}', use YYYY-MM"))?)
    }
    ("goto", None) => return Err("Usage: :goto YYYY-MM".into()),
    ("", _) => return Err("Empty command".into()),
    _ => return Err(format!("Unknown command: {line}")),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn commands() {
    assert_eq!(parse("w"), Ok(Command::Write { force: false }));
    assert_eq!(parse("w!"), Ok(Command::Write { force: true }));
    assert_eq!(parse("q!"), Ok(Command::Quit { force: true }));
    assert_eq!(
      parse("goto 2026-10"),
      Ok(Command::Goto(Month {
        year: 2026,
        month: 10
      }))
    );
    assert!(parse("goto 2026-13").is_err());
    assert!(parse("frobnicate").is_err());
  }
}
