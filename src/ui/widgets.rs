use crate::app::{App, Mode};
use crate::config::Action;
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

pub const ACCENT: Color = Color::Cyan;
pub const POSITIVE: Color = Color::Green;
pub const NEGATIVE: Color = Color::Red;
pub const MUTED: Color = Color::DarkGray;

pub fn block(title: &str) -> Block<'static> {
  Block::bordered()
    .border_type(BorderType::Plain)
    .title(format!(" {title} "))
}

pub fn selected() -> Style {
  Style::new()
    .bg(Color::DarkGray)
    .add_modifier(Modifier::BOLD)
}

pub fn heading() -> Style {
  Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
}

pub fn signed(q: rust_decimal::Decimal) -> Style {
  if q.is_sign_negative() && !q.is_zero() {
    Style::new().fg(NEGATIVE)
  } else {
    Style::new().fg(POSITIVE)
  }
}

/// A rectangle of the given size centered in `area`.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
  let [v] = Layout::vertical([Constraint::Length(height.min(area.height))])
    .flex(Flex::Center)
    .areas(area);
  let [h] = Layout::horizontal([Constraint::Length(width.min(area.width))])
    .flex(Flex::Center)
    .areas(v);
  h
}

/// Horizontal bar of `width` cells, filled in proportion to `value / max`.
pub fn bar(value: f64, max: f64, width: usize, fill: char, empty: char) -> String {
  let filled = if max > 0.0 {
    ((value / max) * width as f64)
      .round()
      .clamp(0.0, width as f64) as usize
  } else {
    0
  };
  let mut s: String = std::iter::repeat_n(fill, filled).collect();
  s.extend(std::iter::repeat_n(empty, width - filled));
  s
}

/// Single-line text input with a cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Input {
  pub value: String,
  pub cursor: usize,
}

impl Input {
  pub fn new(value: &str) -> Self {
    Input {
      value: value.to_string(),
      cursor: value.chars().count(),
    }
  }

  pub fn set(&mut self, value: &str) {
    *self = Input::new(value);
  }

  fn byte_pos(&self, char_pos: usize) -> usize {
    self
      .value
      .char_indices()
      .nth(char_pos)
      .map(|(b, _)| b)
      .unwrap_or(self.value.len())
  }

  /// Handle an editing key. Returns true if the key was consumed.
  pub fn handle(&mut self, key: &KeyEvent) -> bool {
    let len = self.value.chars().count();
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
      KeyCode::Char('u') if ctrl => self.set(""),
      KeyCode::Char('w') if ctrl => {
        let before: String = self.value.chars().take(self.cursor).collect();
        let trimmed = before.trim_end();
        let cut = trimmed.rfind([' ', ':']).map(|p| p + 1).unwrap_or(0);
        let after: String = self.value.chars().skip(self.cursor).collect();
        self.value = format!("{}{after}", &trimmed[..cut]);
        self.cursor = trimmed[..cut].chars().count();
      }
      KeyCode::Char(_) if ctrl => return false,
      KeyCode::Char(c) => {
        let b = self.byte_pos(self.cursor);
        self.value.insert(b, c);
        self.cursor += 1;
      }
      KeyCode::Backspace if self.cursor > 0 => {
        self.cursor -= 1;
        let b = self.byte_pos(self.cursor);
        self.value.remove(b);
      }
      KeyCode::Delete if self.cursor < len => {
        let b = self.byte_pos(self.cursor);
        self.value.remove(b);
      }
      KeyCode::Backspace | KeyCode::Delete => {}
      KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
      KeyCode::Right => self.cursor = (self.cursor + 1).min(len),
      KeyCode::Home => self.cursor = 0,
      KeyCode::End => self.cursor = len,
      _ => return false,
    }
    true
  }

  /// Spans for the input, with the cursor shown when focused.
  pub fn spans(&self, width: usize, focused: bool) -> Vec<Span<'static>> {
    let chars: Vec<char> = self.value.chars().collect();
    let start = if focused {
      (self.cursor + 1).saturating_sub(width)
    } else {
      0
    };
    let visible: Vec<char> = chars.iter().skip(start).take(width).copied().collect();
    let mut spans = Vec::new();
    let base = if focused {
      Style::new().fg(Color::White).bg(Color::Black)
    } else {
      Style::new()
    };
    if focused {
      let cur = self.cursor - start;
      let before: String = visible.iter().take(cur).collect();
      let at: String = visible
        .get(cur)
        .map(|c| c.to_string())
        .unwrap_or(" ".into());
      let after: String = visible.iter().skip(cur + 1).collect();
      spans.push(Span::styled(before, base));
      spans.push(Span::styled(at, base.add_modifier(Modifier::REVERSED)));
      spans.push(Span::styled(after, base));
    } else {
      spans.push(Span::styled(visible.iter().collect::<String>(), base));
    }
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    spans.push(Span::styled(" ".repeat(width.saturating_sub(used)), base));
    spans
  }
}

/// Bottom line: command or search input, a status message, or key hints.
pub fn status_line(app: &App) -> Line<'static> {
  match &app.mode {
    Mode::Command(input) => {
      let mut spans = vec![Span::raw(":")];
      spans.extend(input.spans(input.value.chars().count() + 1, true));
      return Line::from(spans);
    }
    Mode::Search(input) => {
      let mut spans = vec![Span::raw("/")];
      spans.extend(input.spans(input.value.chars().count() + 1, true));
      return Line::from(spans);
    }
    _ => {}
  }
  if let Some(status) = &app.status {
    let style = if status.error {
      Style::new().fg(NEGATIVE)
    } else {
      Style::new().fg(ACCENT)
    };
    return Line::from(Span::styled(status.text.clone(), style));
  }
  let hints = [
    (Action::Command, "command"),
    (Action::Search, "search"),
    (Action::Add, "add"),
    (Action::Edit, "edit"),
    (Action::Delete, "del"),
    (Action::Help, "help"),
    (Action::Quit, "quit"),
  ];
  let mut spans = Vec::new();
  for (action, label) in hints {
    spans.push(Span::styled(
      app.keymap.label(action),
      Style::new().fg(ACCENT),
    ));
    spans.push(Span::styled(format!(" {label}  "), Style::new().fg(MUTED)));
  }
  Line::from(spans)
}

pub fn help_overlay(f: &mut Frame, app: &App) {
  let lines: Vec<Line> = app
    .keymap
    .describe()
    .into_iter()
    .map(|(keys, desc)| {
      Line::from(vec![
        Span::styled(format!("{keys:<18}"), Style::new().fg(ACCENT)),
        Span::raw(desc),
      ])
    })
    .chain([
      Line::raw(""),
      Line::styled(
        "Commands: :w  :q  :q!  :wq  :reload  :goto YYYY-MM",
        Style::new().fg(MUTED),
      ),
      Line::styled("Press any key to close", Style::new().fg(MUTED)),
    ])
    .collect();
  let area = centered(f.area(), 60, lines.len() as u16 + 2);
  f.render_widget(Clear, area);
  f.render_widget(Paragraph::new(lines).block(block("Help")), area);
}

/// A small yes/no or multiple-choice prompt.
pub fn prompt(f: &mut Frame, title: &str, text: &[Line<'static>]) {
  let width = text.iter().map(|l| l.width()).max().unwrap_or(20) as u16 + 4;
  let area = centered(f.area(), width.max(30), text.len() as u16 + 2);
  f.render_widget(Clear, area);
  f.render_widget(
    Paragraph::new(text.to_vec())
      .block(block(title))
      .wrap(Wrap { trim: false }),
    area,
  );
}

#[cfg(test)]
mod tests {
  use super::*;

  fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
  }

  #[test]
  fn input_editing() {
    let mut i = Input::new("ab");
    i.handle(&key(KeyCode::Left));
    i.handle(&key(KeyCode::Char('ł')));
    assert_eq!(i.value, "ałb");
    i.handle(&key(KeyCode::Backspace));
    i.handle(&key(KeyCode::Delete));
    assert_eq!(i.value, "a");
    let mut i = Input::new("expenses:food");
    i.handle(&KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
    assert_eq!(i.value, "expenses:");
  }

  #[test]
  fn bars() {
    assert_eq!(bar(5.0, 10.0, 4, '#', '.'), "##..");
    assert_eq!(bar(1.0, 0.0, 2, '#', '.'), "..");
  }
}
