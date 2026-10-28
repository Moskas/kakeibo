use super::dashboard::fmt_short;
use super::widgets::{ACCENT, NEGATIVE, POSITIVE, bar, block, heading, signed};
use crate::analysis::series::{categories, flow_between, monthly_flows, net_worth_series};
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Gauge, Row, Sparkline, Table};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

pub fn render(f: &mut Frame, area: Rect, app: &App) {
  let [year_area, cat_area, bottom] = Layout::vertical([
    Constraint::Length(15),
    Constraint::Min(4),
    Constraint::Length(4),
  ])
  .areas(area);
  let [spark_area, gauge_area] =
    Layout::horizontal([Constraint::Percentage(65), Constraint::Percentage(35)]).areas(bottom);
  let j = &app.journal;
  let p = app.period;

  let flows = monthly_flows(j, &p.year_months());
  let bar_w = (year_area.width as usize)
    .saturating_sub(4 + 3 * 13 + 6)
    .clamp(4, 30);
  let rows: Vec<Row> = flows
    .iter()
    .map(|(m, fl)| {
      let spending = fl.spending.to_f64().unwrap_or(0.0);
      let over = fl.spending > fl.income;
      let spend_bar = Span::styled(
        bar(
          spending,
          fl.income.max(fl.spending).to_f64().unwrap_or(0.0),
          bar_w,
          '\u{2588}',
          '\u{2591}',
        ),
        Style::new().fg(if over { NEGATIVE } else { ACCENT }),
      );
      let style = if *m == p { heading() } else { Style::new() };
      Row::new(vec![
        Cell::from(m.short_name()).style(style),
        Cell::from(Line::from(vec![spend_bar])),
        Cell::from(Line::styled(fmt_short(fl.income), Style::new().fg(POSITIVE)).right_aligned()),
        Cell::from(Line::styled(fmt_short(fl.spending), Style::new().fg(NEGATIVE)).right_aligned()),
        Cell::from(Line::styled(fmt_short(fl.saved()), signed(fl.saved())).right_aligned()),
      ])
    })
    .collect();
  let table = Table::new(
    rows,
    [
      Constraint::Length(4),
      Constraint::Length(bar_w as u16),
      Constraint::Length(12),
      Constraint::Length(12),
      Constraint::Length(12),
    ],
  )
  .header(Row::new(["", "Out / In", "In", "Out", "Saved"].map(Cell::from)).style(heading()))
  .block(block(&format!("Income vs spending \u{b7} {}", p.year)));
  f.render_widget(table, year_area);

  let cats = categories(j, p.first_day(), p.last_day(), 2);
  let total: Decimal = cats.iter().map(|(_, q)| *q).sum();
  let cmax = cats
    .first()
    .map(|c| c.1.to_f64().unwrap_or(0.0))
    .unwrap_or(0.0);
  let cbar_w = (cat_area.width as usize)
    .saturating_sub(16 + 12 + 8 + 6)
    .clamp(4, 60);
  let rows: Vec<Row> = cats
    .iter()
    .map(|(name, q)| {
      let pct = if total.is_zero() {
        Decimal::ZERO
      } else {
        (*q * Decimal::from(100) / total).round()
      };
      Row::new(vec![
        Cell::from(name.clone()),
        Cell::from(Span::styled(
          bar(q.to_f64().unwrap_or(0.0), cmax, cbar_w, '\u{2587}', ' '),
          Style::new().fg(ACCENT),
        )),
        Cell::from(Line::from(fmt_short(*q)).right_aligned()),
        Cell::from(Line::from(format!("({pct}%)")).right_aligned()),
      ])
    })
    .collect();
  let table = Table::new(
    rows,
    [
      Constraint::Length(16),
      Constraint::Length(cbar_w as u16),
      Constraint::Length(12),
      Constraint::Length(7),
    ],
  )
  .block(block(&format!("Category breakdown ({})", p.long_name())));
  f.render_widget(table, cat_area);

  let months = p.trailing(12);
  let worth = net_worth_series(j, &months);
  let low = worth.iter().min().copied().unwrap_or_default();
  let data: Vec<u64> = worth
    .iter()
    .map(|w| (*w - low).round().to_u64().unwrap_or(0) + 1)
    .collect();
  let last = worth.last().copied().unwrap_or_default();
  f.render_widget(
    Sparkline::default()
      .data(&data)
      .style(Style::new().fg(ACCENT))
      .block(block(&format!(
        "Net worth, 12 months \u{b7} {}",
        j.format(last, true)
      ))),
    spark_area,
  );

  let flow = flow_between(j, p.first_day(), p.last_day());
  let ratio = if flow.income > Decimal::ZERO {
    (flow.saved() / flow.income)
      .to_f64()
      .unwrap_or(0.0)
      .clamp(0.0, 1.0)
  } else {
    0.0
  };
  let label = if flow.income > Decimal::ZERO {
    format!(
      "{:.0}% of income saved",
      (flow.saved() / flow.income * Decimal::from(100)).round()
    )
  } else {
    "no income this month".to_string()
  };
  f.render_widget(
    Gauge::default()
      .ratio(ratio)
      .label(label)
      .gauge_style(Style::new().fg(POSITIVE))
      .block(block("Savings rate")),
    gauge_area,
  );
}
