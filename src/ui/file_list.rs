use rat_widget::{list::List, scrolled::Scroll};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::border,
    text::Line,
    widgets::{Block, ListItem, Paragraph, StatefulWidget as _},
};

use super::focused_border_style;
use crate::{app::App, types::Pane};

pub fn render(app: &mut App, frame: &mut Frame, area: Rect) {
    let truncated = if app.truncated {
        " - limit reached"
    } else {
        ""
    };
    let digit = Pane::FileList.digit();
    let title = if app.searching {
        format!(
            "\u{2500}[{}]\u{2500}{} Files ({}{} matched)",
            digit,
            app.spinner.frame(),
            app.results.len(),
            truncated,
        )
    } else {
        format!(
            "\u{2500}[{}]\u{2500}Files ({} matched{})",
            digit,
            app.results.len(),
            truncated
        )
    };
    let border_style = focused_border_style(Pane::FileList, app.focused_pane);
    let block = Block::bordered()
        .border_set(border::ROUNDED)
        .border_style(border_style)
        .title(title);

    if app.results.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new("No matches").style(Style::default().dim()),
            inner,
        );
        return;
    }

    let selected = app.file_list.selected();
    let inner = block.inner(area);
    let inner_width = inner.width as usize;
    let inner_height = inner.height as usize;
    let total_items = app.results.len();

    // set up scroll state before building items so we know the visible range
    app.file_list.scroll.set_page_len(inner_height);
    app.file_list
        .scroll
        .set_max_offset(total_items.saturating_sub(inner_height));
    app.file_list.scroll_to_selected();

    let offset = app.file_list.offset();
    let visible_end = (offset + inner_height).min(total_items);

    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, fm)| {
            let active = fm.active_match_count();
            let dimmed = Some(i) != selected && active == 0;

            // only format paths for visible items
            if i < offset || i >= visible_end {
                let item = ListItem::new("");
                if dimmed {
                    item.style(Style::default().dim())
                } else {
                    item
                }
            } else {
                let total = fm.matches.len();
                let suffix = format!(" ({active}/{total})");
                let label = fm
                    .responsive_path
                    .as_ref()
                    .map(|p| p.to_width(inner_width.saturating_sub(suffix.len())))
                    .unwrap_or(fm.path.to_string_lossy().into());
                let label = format!("{label}{suffix}");
                if dimmed {
                    ListItem::new(Line::styled(label, Style::default().dim()))
                } else {
                    ListItem::new(label)
                }
            }
        })
        .collect();

    let select_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    List::new(items)
        .block(block)
        .scroll(Scroll::vertical().style(border_style))
        .select_style(select_style)
        .focus_style(select_style)
        .render(area, frame.buffer_mut(), &mut app.file_list);
}
