use super::widgets::{Input, NEGATIVE, POSITIVE, block, centered, heading};
use crate::ledger::model::{Journal, Transaction};
use crate::ledger::ops::{self, Draft};
use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, List, ListItem, Paragraph};

const MAX_SUGGESTIONS: usize = 6;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
  Date,
  Payee,
  Account(usize),
  Amount(usize),
  Note,
  Save,
  Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FormOutcome {
  Continue,
  Cancel,
  Save(Transaction),
}

#[derive(Debug, Clone)]
pub struct FormState {
  /// Item index of the transaction being edited, `None` for a new one.
  pub editing: Option<usize>,
  pub date: Input,
  pub payee: Input,
  pub postings: Vec<(Input, Input)>,
  pub note: Input,
  pub focus: Field,
  pub suggestion: usize,
  pub validation: Result<(), String>,
  pub commodity: String,
}

impl FormState {
  pub fn new(draft: &Draft, editing: Option<usize>, journal: &Journal) -> Self {
    let mut postings: Vec<(Input, Input)> = draft
      .postings
      .iter()
      .map(|(a, m)| (Input::new(a), Input::new(m)))
      .collect();
    while postings.len() < 2 {
      postings.push(Default::default());
    }
    let mut f = FormState {
      editing,
      date: Input::new(&draft.date),
      payee: Input::new(&draft.payee),
      postings,
      note: Input::new(&draft.note),
      focus: if editing.is_some() {
        Field::Date
      } else {
        Field::Payee
      },
      suggestion: 0,
      validation: Ok(()),
      commodity: journal.commodity_or_default(),
    };
    f.revalidate(journal);
    f
  }

  pub fn draft(&self) -> Draft {
    Draft {
      date: self.date.value.clone(),
      payee: self.payee.value.clone(),
      note: self.note.value.clone(),
      postings: self
        .postings
        .iter()
        .map(|(a, m)| (a.value.clone(), m.value.clone()))
        .collect(),
    }
  }

  fn fields(&self) -> Vec<Field> {
    let mut v = vec![Field::Date, Field::Payee];
    for i in 0..self.postings.len() {
      v.push(Field::Account(i));
      v.push(Field::Amount(i));
    }
    v.extend([Field::Note, Field::Save, Field::Cancel]);
    v
  }

  fn input_mut(&mut self, field: Field) -> Option<&mut Input> {
    match field {
      Field::Date => Some(&mut self.date),
      Field::Payee => Some(&mut self.payee),
      Field::Account(i) => self.postings.get_mut(i).map(|p| &mut p.0),
      Field::Amount(i) => self.postings.get_mut(i).map(|p| &mut p.1),
      Field::Note => Some(&mut self.note),
      Field::Save | Field::Cancel => None,
    }
  }

  fn move_focus(&mut self, delta: isize) {
    let fields = self.fields();
    let pos = fields.iter().position(|f| *f == self.focus).unwrap_or(0) as isize;
    let n = fields.len() as isize;
    self.focus = fields[((pos + delta).rem_euclid(n)) as usize];
    self.suggestion = 0;
  }

  /// Autocomplete candidates for the focused field.
  pub fn suggestions(&self, journal: &Journal) -> Vec<String> {
    let (candidates, current) = match self.focus {
      Field::Payee => (journal.payees(), &self.payee.value),
      Field::Account(i) => (journal.accounts(), &self.postings[i].0.value),
      _ => return vec![],
    };
    let needle = current.to_lowercase();
    if needle.is_empty() {
      return vec![];
    }
    let mut prefix: Vec<String> = Vec::new();
    let mut inner: Vec<String> = Vec::new();
    for c in candidates {
      let l = c.to_lowercase();
      if l == needle {
        continue;
      } else if l.starts_with(&needle) {
        prefix.push(c);
      } else if l.contains(&needle) {
        inner.push(c);
      }
    }
    prefix.extend(inner);
    prefix.truncate(MAX_SUGGESTIONS);
    prefix
  }

  fn revalidate(&mut self, journal: &Journal) {
    self.validation = ops::build_valid(&self.draft(), journal).map(|_| ());
  }

  fn accept_suggestion(&mut self, journal: &Journal, value: String) {
    let focus = self.focus;
    if let Some(input) = self.input_mut(focus) {
      input.set(&value);
    }
    // a known payee fills empty postings from its most recent transaction
    let blank = self
      .postings
      .iter()
      .all(|(a, m)| a.value.is_empty() && m.value.is_empty());
    if focus == Field::Payee
      && self.editing.is_none()
      && blank
      && let Some(d) = journal
        .last_with_payee(&value)
        .and_then(|t| ops::draft_of(t).ok())
    {
      self.postings = d
        .postings
        .iter()
        .map(|(a, m)| (Input::new(a), Input::new(m)))
        .collect();
    }
    self.suggestion = 0;
  }

  pub fn handle(&mut self, key: &KeyEvent, journal: &Journal) -> FormOutcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let suggestions = self.suggestions(journal);
    match key.code {
      KeyCode::Esc => return FormOutcome::Cancel,
      KeyCode::Char('s') if ctrl => return self.try_save(journal),
      KeyCode::Char('a') if ctrl => {
        self.postings.push(Default::default());
        self.focus = Field::Account(self.postings.len() - 1);
      }
      KeyCode::Char('x') if ctrl => {
        if let Field::Account(i) | Field::Amount(i) = self.focus
          && self.postings.len() > 2
        {
          self.postings.remove(i);
          self.focus = Field::Account(i.min(self.postings.len() - 1));
        }
      }
      KeyCode::Tab => self.move_focus(1),
      KeyCode::BackTab => self.move_focus(-1),
      KeyCode::Down if !suggestions.is_empty() => {
        self.suggestion = (self.suggestion + 1) % suggestions.len();
      }
      KeyCode::Up if !suggestions.is_empty() => {
        self.suggestion = (self.suggestion + suggestions.len() - 1) % suggestions.len();
      }
      KeyCode::Down => self.move_focus(1),
      KeyCode::Up => self.move_focus(-1),
      KeyCode::Enter => match self.focus {
        Field::Save => return self.try_save(journal),
        Field::Cancel => return FormOutcome::Cancel,
        _ if !suggestions.is_empty() => {
          let pick = suggestions[self.suggestion.min(suggestions.len() - 1)].clone();
          self.accept_suggestion(journal, pick);
          self.move_focus(1);
        }
        _ => self.move_focus(1),
      },
      _ => {
        let focus = self.focus;
        if let Some(input) = self.input_mut(focus)
          && input.handle(key)
        {
          self.suggestion = 0;
        }
      }
    }
    self.revalidate(journal);
    FormOutcome::Continue
  }

  fn try_save(&mut self, journal: &Journal) -> FormOutcome {
    match ops::build_valid(&self.draft(), journal) {
      Ok(t) => FormOutcome::Save(t),
      Err(e) => {
        self.validation = Err(e);
        FormOutcome::Continue
      }
    }
  }
}

const LABEL_W: usize = 9;
const INPUT_W: usize = 22;
const AMOUNT_W: usize = 10;

pub fn render(f: &mut Frame, form: &FormState, journal: &Journal) {
  let title = if form.editing.is_some() {
    "Edit transaction"
  } else {
    "New transaction"
  };
  let rows = 2 + form.postings.len() + 1 + 2 + 1;
  let width = (LABEL_W + INPUT_W + AMOUNT_W + 12) as u16;
  let area = centered(f.area(), width, rows as u16 + 2);
  f.render_widget(Clear, area);

  let field_line = |label: &str, input: &Input, field: Field, w: usize| -> Vec<Span<'static>> {
    let focused = form.focus == field;
    let label_style = if focused { heading() } else { Style::new() };
    let mut spans = vec![
      Span::styled(format!("{label:<LABEL_W$}"), label_style),
      Span::raw("["),
    ];
    spans.extend(input.spans(w, focused));
    spans.push(Span::raw("]"));
    spans
  };

  let mut lines: Vec<Line> = vec![
    Line::from(field_line("Date", &form.date, Field::Date, INPUT_W)),
    Line::from(field_line("Payee", &form.payee, Field::Payee, INPUT_W)),
  ];
  let mut anchor_rows: Vec<(Field, usize)> = vec![(Field::Payee, 1)];
  for (i, (acct, amt)) in form.postings.iter().enumerate() {
    let label = match i {
      0 => "Account".to_string(),
      1 => "Against".to_string(),
      n => format!("Posting{}", n + 1),
    };
    let mut spans = field_line(&label, acct, Field::Account(i), INPUT_W);
    spans.push(Span::raw(" ["));
    spans.extend(amt.spans(AMOUNT_W, form.focus == Field::Amount(i)));
    spans.push(Span::raw(format!("] {}", form.commodity)));
    anchor_rows.push((Field::Account(i), lines.len()));
    lines.push(Line::from(spans));
  }
  lines.push(Line::from(field_line(
    "Note",
    &form.note,
    Field::Note,
    INPUT_W,
  )));
  lines.push(Line::raw(""));

  let button = |label: &str, field: Field| {
    let style = if form.focus == field {
      Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
      Style::new()
    };
    Span::styled(format!("[ {label} ]"), style)
  };
  lines.push(Line::from(vec![
    Span::raw("      "),
    button("Save Ctrl-s", Field::Save),
    Span::raw("     "),
    button("Cancel Esc", Field::Cancel),
  ]));
  lines.push(match &form.validation {
    Ok(()) => Line::styled("\u{2713} Ready to save", Style::new().fg(POSITIVE)),
    Err(e) => Line::styled(format!("\u{2717} {e}"), Style::new().fg(NEGATIVE)),
  });

  let hint = " Tab next · Enter pick · ^a/^x add/remove posting ";
  f.render_widget(
    Paragraph::new(lines).block(block(title).title_bottom(hint)),
    area,
  );

  let suggestions = form.suggestions(journal);
  if suggestions.is_empty() {
    return;
  }
  let Some(row) = anchor_rows
    .iter()
    .find(|(fld, _)| *fld == form.focus)
    .map(|(_, r)| *r)
  else {
    return;
  };
  let y = area.y + 2 + row as u16;
  let x = area.x + 1 + LABEL_W as u16 + 1;
  let h = (suggestions.len() as u16 + 2).min(f.area().height.saturating_sub(y));
  let popup = Rect {
    x,
    y,
    width: (INPUT_W as u16 + 2).min(f.area().width.saturating_sub(x)),
    height: h,
  };
  let items: Vec<ListItem> = suggestions
    .iter()
    .enumerate()
    .map(|(k, s)| {
      let style = if k == form.suggestion.min(suggestions.len() - 1) {
        Style::new().bg(Color::Cyan).fg(Color::Black)
      } else {
        Style::new()
      };
      ListItem::new(Span::styled(s.clone(), style))
    })
    .collect();
  f.render_widget(Clear, popup);
  f.render_widget(
    List::new(items).block(ratatui::widgets::Block::bordered()),
    popup,
  );
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::ledger::parser::parse;

  fn key(c: KeyCode) -> KeyEvent {
    KeyEvent::new(c, KeyModifiers::NONE)
  }

  fn typ(form: &mut FormState, j: &Journal, s: &str) {
    for c in s.chars() {
      form.handle(&key(KeyCode::Char(c)), j);
    }
  }

  #[test]
  fn fill_and_save() {
    let j = parse(
      "2026-10-01 Lidl\n  expenses:groceries  57.30 PLN\n  assets:checking\n",
      None,
    )
    .unwrap();
    let draft = Draft {
      date: "2026-10-28".into(),
      ..Default::default()
    };
    let mut form = FormState::new(&draft, None, &j);
    assert_eq!(form.focus, Field::Payee);
    typ(&mut form, &j, "Li");
    assert_eq!(form.suggestions(&j), vec!["Lidl"]);
    form.handle(&key(KeyCode::Enter), &j);
    assert_eq!(form.payee.value, "Lidl");
    assert_eq!(form.postings[0].0.value, "expenses:groceries");
    assert_eq!(form.focus, Field::Account(0));
    assert!(form.validation.is_ok());

    form.focus = Field::Amount(1);
    typ(&mut form, &j, "1");
    assert!(form.validation.as_ref().unwrap_err().contains("balance"));
    form.handle(&key(KeyCode::Backspace), &j);
    form.focus = Field::Save;
    let FormOutcome::Save(t) = form.handle(&key(KeyCode::Enter), &j) else {
      panic!()
    };
    assert_eq!(
      t.postings[0].amount.as_ref().unwrap().to_string(),
      "57.30 PLN"
    );
  }
}
