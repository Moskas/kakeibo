use super::widgets::{block, selected, signed};
use crate::analysis::balance::AccountKind;
use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Cell, Row, Table, TableState};

pub fn render(f: &mut Frame, area: Rect, app: &App) {
  let rows: Vec<Row> = app
    .acct_rows
    .iter()
    .map(|r| {
      let marker = match (r.has_children, r.expanded) {
        (true, true) => "\u{25bc} ",
        (true, false) => "\u{25b8} ",
        (false, _) => "  ",
      };
      let leaf = r.name.rsplit(':').next().unwrap_or(&r.name);
      let label = format!("{}{marker}{leaf}", "  ".repeat(r.depth));
      let ytd = r.depth == 0
        && matches!(
          AccountKind::of(&r.name),
          AccountKind::Income | AccountKind::Expense
        );
      let style = if r.depth == 0 {
        Style::new().add_modifier(Modifier::BOLD)
      } else {
        Style::new()
      };
      Row::new(vec![
        Cell::from(label).style(style),
        Cell::from(
          Line::styled(app.journal.format(r.balance, true), signed(r.balance)).right_aligned(),
        ),
        Cell::from(if ytd { "(YTD)" } else { "" }),
      ])
    })
    .collect();
  let title = format!("Accounts \u{b7} as of {}", app.period.last_day());
  let table = Table::new(
    rows,
    [
      Constraint::Fill(1),
      Constraint::Length(18),
      Constraint::Length(6),
    ],
  )
  .row_highlight_style(selected())
  .block(
    block(&title)
      .title_bottom(" Enter/Space expand \u{b7} Enter on a leaf shows its transactions "),
  );
  let mut state =
    TableState::default().with_selected((!app.acct_rows.is_empty()).then_some(app.acct_selected));
  f.render_stateful_widget(table, area, &mut state);
}
