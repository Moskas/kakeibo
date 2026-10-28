use super::widgets::{ACCENT, MUTED, NEGATIVE, POSITIVE, bar, block, signed};
use crate::analysis::balance::net_worth;
use crate::analysis::series::{categories, flow_between, monthly_flows, transaction_flow};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Paragraph};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
  let [top, chart, bottom] = Layout::vertical([
    Constraint::Length(4),
    Constraint::Min(7),
    Constraint::Percentage(40),
  ])
  .areas(area);
  let [worth_area, month_area] =
    Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)]).areas(top);
  let [cat_area, recent_area] =
    Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(bottom);
  let j = &app.journal;
  let fmt = |q: Decimal| j.format(q, true);
  let p = app.period;

  let now = net_worth(j, Some(p.last_day()));
  let delta = now - net_worth(j, Some(p.prev().last_day()));
  let arrow = if delta.is_sign_negative() {
    "\u{25bc}"
  } else {
    "\u{25b2}"
  };
  f.render_widget(
    Paragraph::new(vec![
      Line::styled(fmt(now), Style::new().add_modifier(Modifier::BOLD)),
      Line::from(vec![
        Span::styled(format!("{arrow} {}", fmt(delta.abs())), signed(delta)),
        Span::styled(" vs last mo", Style::new().fg(MUTED)),
      ]),
    ])
    .block(block("Net worth")),
    worth_area,
  );

  let flow = flow_between(j, p.first_day(), p.last_day());
  let saved = flow.saved();
  let rate = if flow.income.is_zero() {
    String::new()
  } else {
    format!(" ({}%)", (saved * Decimal::from(100) / flow.income).round())
  };
  f.render_widget(
    Paragraph::new(vec![
      Line::from(vec![
        Span::styled("In  ", Style::new().fg(MUTED)),
        Span::styled(fmt(flow.income), Style::new().fg(POSITIVE)),
        Span::styled("   Out  ", Style::new().fg(MUTED)),
        Span::styled(fmt(flow.spending), Style::new().fg(NEGATIVE)),
      ]),
      Line::from(vec![
        Span::styled("Saved  ", Style::new().fg(MUTED)),
        Span::styled(format!("{}{rate}", fmt(saved)), signed(saved)),
      ]),
    ])
    .block(block(&format!("This month \u{b7} {}", p.long_name()))),
    month_area,
  );

  let months = p.trailing(12);
  let flows = monthly_flows(j, &months);
  let inner_w = chart.width.saturating_sub(2) as usize;
  let bar_w = ((inner_w / months.len()).saturating_sub(1)).clamp(1, 8) as u16;
  let bars: Vec<Bar> = flows
    .iter()
    .map(|(m, fl)| {
      let v = fl.spending.round().to_u64().unwrap_or(0);
      let style = if *m == p {
        Style::new().fg(ACCENT)
      } else {
        Style::new().fg(NEGATIVE)
      };
      Bar::default()
        .value(v)
        .label(Line::from(m.short_name()))
        .style(style)
        .text_value(String::new())
    })
    .collect();
  f.render_widget(
    BarChart::default()
      .block(block("Spending by month (last 12)"))
      .data(BarGroup::default().bars(&bars))
      .bar_width(bar_w)
      .bar_gap(1),
    chart,
  );

  let cats = categories(j, p.first_day(), p.last_day(), 2);
  let max = cats
    .first()
    .map(|c| c.1.to_f64().unwrap_or(0.0))
    .unwrap_or(0.0);
  let rows = cat_area.height.saturating_sub(2) as usize;
  let name_w = 14;
  let amount_w = cats
    .iter()
    .map(|(_, q)| fmt_short(*q).len())
    .max()
    .unwrap_or(0);
  let bar_w = (cat_area.width as usize)
    .saturating_sub(name_w + amount_w + 5)
    .max(1);
  let lines: Vec<Line> = cats
    .iter()
    .take(rows)
    .map(|(name, q)| {
      Line::from(vec![
        Span::raw(format!("{:<name_w$}", truncate(name, name_w - 1))),
        Span::styled(
          bar(q.to_f64().unwrap_or(0.0), max, bar_w, '\u{2588}', ' '),
          Style::new().fg(ACCENT),
        ),
        Span::raw(format!(" {:>amount_w$}", fmt_short(*q))),
      ])
    })
    .collect();
  f.render_widget(
    Paragraph::new(lines).block(block("Top categories")),
    cat_area,
  );

  let mut recent: Vec<_> = j
    .transactions()
    .filter(|(_, t)| t.date <= p.last_day())
    .collect();
  recent.sort_by(|a, b| b.1.date.cmp(&a.1.date).then(b.0.cmp(&a.0)));
  let rows = recent_area.height.saturating_sub(2) as usize;
  let width = recent_area.width.saturating_sub(2) as usize;
  let lines: Vec<Line> = recent
    .iter()
    .take(rows)
    .map(|(_, t)| {
      let (amount, style) = flow_cell(t);
      let desc_w = width.saturating_sub(6 + amount.len() + 1);
      Line::from(vec![
        Span::styled(t.date.format("%m-%d ").to_string(), Style::new().fg(MUTED)),
        Span::raw(format!("{:<desc_w$}", truncate(&t.description, desc_w))),
        Span::styled(format!(" {amount}"), style),
      ])
    })
    .collect();
  f.render_widget(Paragraph::new(lines).block(block("Recent")), recent_area);
}

/// A number without commodity, keeping its own precision.
pub fn fmt_short(q: Decimal) -> String {
  let s = crate::ledger::model::format_number(q.abs(), q.scale(), true);
  if q.is_sign_negative() && !q.is_zero() {
    format!("-{s}")
  } else {
    s
  }
}

pub fn truncate(s: &str, w: usize) -> String {
  if s.chars().count() <= w {
    s.to_string()
  } else {
    let mut t: String = s.chars().take(w.saturating_sub(1)).collect();
    t.push('\u{2026}');
    t
  }
}

/// Signed amount text and style for a transaction row; transfers are shown unsigned.
pub fn flow_cell(t: &crate::ledger::model::Transaction) -> (String, Style) {
  let q = transaction_flow(t);
  if crate::analysis::series::is_transfer(t) {
    return (fmt_short(q.abs()), Style::new().fg(MUTED));
  }
  let plus = if q.is_sign_positive() && !q.is_zero() {
    "+"
  } else {
    ""
  };
  (format!("{plus}{}", fmt_short(q)), signed(q))
}
