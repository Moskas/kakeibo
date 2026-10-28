pub mod accounts;
pub mod command;
pub mod dashboard;
pub mod form;
pub mod reports;
pub mod transactions;
pub mod widgets;

use crate::app::{App, Confirm, Mode, View};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Tabs};
use similar::{ChangeTag, TextDiff};
use widgets::{ACCENT, MUTED, NEGATIVE, POSITIVE};

/// Render the whole screen. Pure: reads `app`, writes only to the frame.
pub fn view(app: &App, f: &mut Frame) {
  let home = std::env::var("HOME").unwrap_or_default();
  let path = app.path.display().to_string();
  let path = match path.strip_prefix(&home) {
    Some(rest) if !home.is_empty() => format!("~{rest}"),
    _ => path,
  };
  let dirty = if app.dirty { " [+]" } else { "" };
  let outer = Block::bordered()
    .title(Line::from(vec![
      Span::styled(
        " kakeibo ",
        Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
      ),
      Span::raw(format!("\u{2500} {path}{dirty} ")),
    ]))
    .title(Line::from(format!(" {} ", app.period)).right_aligned());
  let inner = outer.inner(f.area());
  f.render_widget(outer, f.area());

  let [tabs_area, body, status_area] = Layout::vertical([
    Constraint::Length(1),
    Constraint::Min(0),
    Constraint::Length(1),
  ])
  .areas(inner);

  let titles: Vec<String> = View::ALL
    .iter()
    .enumerate()
    .map(|(i, v)| format!("[{}] {}", i + 1, v.title()))
    .collect();
  let selected = View::ALL.iter().position(|v| *v == app.view).unwrap_or(0);
  f.render_widget(
    Tabs::new(titles)
      .select(selected)
      .highlight_style(Style::new().fg(ACCENT).add_modifier(Modifier::BOLD))
      .divider(" "),
    tabs_area,
  );

  match app.view {
    View::Dashboard => dashboard::render(f, body, app),
    View::Transactions => transactions::render(f, body, app),
    View::Accounts => accounts::render(f, body, app),
    View::Reports => reports::render(f, body, app),
  }
  f.render_widget(Paragraph::new(widgets::status_line(app)), status_area);

  match &app.mode {
    Mode::Form(form) => form::render(f, form, &app.journal),
    Mode::Help => widgets::help_overlay(f, app),
    Mode::Confirm(Confirm::Delete(idx)) => {
      let desc = app
        .journal
        .transaction(*idx)
        .map(|t| format!("{} {}", t.date, t.description))
        .unwrap_or_default();
      widgets::prompt(
        f,
        "Delete",
        &[
          Line::raw(format!("Delete {desc}?")),
          Line::raw(""),
          keys_line(&[("y", "yes"), ("n", "no")]),
        ],
      );
    }
    Mode::Confirm(Confirm::Quit) => widgets::prompt(
      f,
      "Unsaved changes",
      &[
        Line::raw("Quit without saving?"),
        Line::raw(""),
        keys_line(&[("y", "quit"), ("n", "stay")]),
      ],
    ),
    Mode::Filter(ff) => {
      let mut lines: Vec<Line> = ff
        .fields
        .iter()
        .enumerate()
        .map(|(i, input)| {
          let label = crate::app::FilterForm::LABELS[i];
          let style = if i == ff.focus {
            widgets::heading()
          } else {
            Style::new()
          };
          let mut spans = vec![Span::styled(format!("{label:<9}"), style), Span::raw("[")];
          spans.extend(input.spans(24, i == ff.focus));
          spans.push(Span::raw("]"));
          Line::from(spans)
        })
        .collect();
      lines.push(Line::raw(""));
      lines.push(keys_line(&[
        ("Enter", "apply"),
        ("Ctrl-p", "period"),
        ("Ctrl-l", "clear"),
        ("Esc", "cancel"),
      ]));
      if let Some(e) = &ff.error {
        lines.push(Line::styled(e.clone(), Style::new().fg(NEGATIVE)));
      }
      widgets::prompt(f, "Filter", &lines);
    }
    Mode::Conflict(c) if c.show_diff => {
      let ours = crate::ledger::writer::write(&app.journal);
      let diff = TextDiff::from_lines(&c.theirs, &ours);
      let mut lines = vec![Line::styled(
        "--- on disk  +++ yours   (j/k scroll, d/Esc back, r reload theirs)",
        Style::new().fg(MUTED),
      )];
      for group in diff.grouped_ops(2) {
        for op in group {
          for change in diff.iter_changes(&op) {
            let (sign, style) = match change.tag() {
              ChangeTag::Delete => ("-", Style::new().fg(NEGATIVE)),
              ChangeTag::Insert => ("+", Style::new().fg(POSITIVE)),
              ChangeTag::Equal => (" ", Style::new()),
            };
            lines.push(Line::styled(
              format!("{sign}{}", change.value().trim_end_matches(['\n', '\r'])),
              style,
            ));
          }
        }
        lines.push(Line::styled(
          "\u{2500}\u{2500}\u{2500}",
          Style::new().fg(MUTED),
        ));
      }
      let area = f.area().inner(ratatui::layout::Margin::new(2, 1));
      f.render_widget(Clear, area);
      f.render_widget(
        Paragraph::new(lines)
          .scroll((c.scroll, 0))
          .block(widgets::block("Diff")),
        area,
      );
    }
    Mode::Conflict(_) => widgets::prompt(
      f,
      "File changed on disk",
      &[
        Line::raw("The journal changed outside kakeibo and you have unsaved edits."),
        Line::raw(""),
        keys_line(&[
          ("k", "keep mine"),
          ("r", "reload theirs"),
          ("d", "view diff"),
        ]),
      ],
    ),
    Mode::AddAccount(af) => {
      let mut input = vec![Span::raw("Name  [")];
      input.extend(af.input.spans(36, true));
      input.push(Span::raw("]"));
      let mut lines = vec![
        Line::from(input),
        Line::styled(
          "Writes an `account` directive to the journal",
          Style::new().fg(MUTED),
        ),
        Line::raw(""),
        keys_line(&[("Enter", "add"), ("Esc", "cancel")]),
      ];
      if let Some(e) = &af.error {
        lines.push(Line::styled(e.clone(), Style::new().fg(NEGATIVE)));
      }
      widgets::prompt(f, "New account", &lines);
    }
    Mode::Normal | Mode::Command(_) | Mode::Search(_) => {}
  }
}

fn keys_line(keys: &[(&str, &str)]) -> Line<'static> {
  let mut spans = Vec::new();
  for (k, label) in keys {
    spans.push(Span::styled(
      k.to_string(),
      Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(format!(" {label}   ")));
  }
  Line::from(spans)
}
