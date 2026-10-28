use super::dashboard::{flow_cell, truncate};
use super::widgets::{MUTED, block, heading, selected, signed};
use crate::analysis::series::transaction_category;
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
  let detail_h = app
    .selected_transaction()
    .map(|(_, t)| detail_height(t))
    .unwrap_or(3);
  let [list_area, detail_area] =
    Layout::vertical([Constraint::Min(5), Constraint::Length(detail_h)]).areas(area);
  let j = &app.journal;

  let rows: Vec<Row> = app
    .tx_rows
    .iter()
    .filter_map(|i| j.transaction(*i))
    .map(|t| {
      let (amount, style) = flow_cell(t);
      Row::new(vec![
        Cell::from(t.date.format("%Y-%m-%d").to_string()),
        Cell::from(t.status.symbol()),
        Cell::from(t.description.clone()),
        Cell::from(transaction_category(t).to_string()),
        Cell::from(Line::styled(amount, style).right_aligned()),
      ])
    })
    .collect();
  let title = format!(
    "Transactions  filter: {} \u{b7} {} rows",
    app.filter.describe(),
    app.tx_rows.len()
  );
  let table = Table::new(
    rows,
    [
      Constraint::Length(10),
      Constraint::Length(2),
      Constraint::Fill(3),
      Constraint::Fill(2),
      Constraint::Length(12),
    ],
  )
  .header(
    Row::new([
      Cell::from("Date"),
      Cell::from("St"),
      Cell::from("Payee"),
      Cell::from("Account"),
      Cell::from(Line::from("Amount").right_aligned()),
    ])
    .style(heading()),
  )
  .row_highlight_style(selected())
  .highlight_symbol("\u{25b6} ")
  .block(block(&title));
  let mut state =
    TableState::default().with_selected((!app.tx_rows.is_empty()).then_some(app.tx_selected));
  f.render_stateful_widget(table, list_area, &mut state);

  let Some((_, t)) = app.selected_transaction() else {
    f.render_widget(
      Paragraph::new("No transactions").block(block("Selected")),
      detail_area,
    );
    return;
  };
  let mut lines = vec![];
  for c in t.comment.iter().chain(&t.comment_lines) {
    lines.push(Line::styled(format!("; {c}"), Style::new().fg(MUTED)));
  }
  let acct_w = t
    .postings
    .iter()
    .map(|p| p.account.chars().count())
    .max()
    .unwrap_or(0)
    .max(20);
  for (p, amt) in t.resolved_postings() {
    let q = amt.as_ref().map(|a| a.quantity).unwrap_or_default();
    let mut spans = vec![
      Span::raw(format!(
        "  {}{:<acct_w$}  ",
        status_prefix(p.status.symbol()),
        p.account
      )),
      Span::styled(format!("{:>14}", j.format(q, true)), signed(q)),
    ];
    if p.amount.is_none() {
      spans.push(Span::styled("  (inferred)", Style::new().fg(MUTED)));
    }
    if let Some(a) = &p.assertion {
      spans.push(Span::styled(format!("  = {a}"), Style::new().fg(MUTED)));
    }
    if let Some(c) = &p.comment {
      spans.push(Span::styled(format!("  ; {c}"), Style::new().fg(MUTED)));
    }
    lines.push(Line::from(spans));
  }
  let code = t
    .code
    .as_ref()
    .map(|c| format!(" ({c})"))
    .unwrap_or_default();
  let title = format!(
    "Selected: {}{code} {}  \u{b7} line {}",
    t.date.format("%Y-%m-%d"),
    truncate(&t.description, 40),
    t.line
  );
  let title = if t.line == 0 {
    title.split(" \u{b7} line").next().unwrap_or("").to_string()
  } else {
    title
  };
  f.render_widget(Paragraph::new(lines).block(block(&title)), detail_area);
}

fn status_prefix(sym: &str) -> String {
  if sym.is_empty() {
    String::new()
  } else {
    format!("{sym} ")
  }
}

fn detail_height(t: &crate::ledger::model::Transaction) -> u16 {
  let comments = t.comment.iter().count() + t.comment_lines.len();
  (t.postings.len() + comments + 2).min(12) as u16
}
