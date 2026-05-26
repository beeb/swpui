mod builder;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use rat_widget::scrolled::{Scroll, ScrollArea, ScrollAreaState, ScrollState};
use ratatui::{
    buffer::Buffer,
    crossterm::event::{KeyCode, KeyEvent, KeyEventKind},
    layout::{Alignment, Rect},
    style::{Color, Style},
    symbols::border,
    widgets::{Block, Paragraph, StatefulWidget, Widget as _},
};
use unicode_width::UnicodeWidthStr as _;

use crate::{
    config::MatchMode, preview::data::PreviewData, search::FileMatches, types::Pane,
    ui::preview::builder::PreviewBuilder, utils::trim_start_to_width,
};

/// Per-frame mutable state for the [`Preview`] widget.
#[derive(Default)]
pub struct PreviewState {
    scroll: ScrollState,
    data: HashMap<PathBuf, Arc<PreviewData>>,
    error: HashMap<PathBuf, String>,
    loading: bool,
    /// Extra line offset within the selected match (for tall matches that exceed the viewport).
    line_offset: usize,
    /// Max value for `line_offset`, recomputed during each render.
    line_offset_max: usize,
    selected_match: usize,
}

impl PreviewState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Currently selected match index within the active file's preview.
    pub fn selected_match(&self) -> usize {
        self.selected_match
    }

    /// Reset selection and scroll position.
    ///
    /// Called after navigating to a different file or match.
    pub fn reset_position(&mut self) {
        self.selected_match = 0;
        self.line_offset = 0;
        self.scroll.clear();
    }

    /// Reset to default state.
    pub fn clear(&mut self) {
        self.data.clear();
        self.error.clear();
        self.loading = false;
        self.reset_position();
    }

    /// Clamp the selected match index to a non-empty file's match count and reset the line
    /// offset within the current match.
    pub fn clamp_match(&mut self, match_count: usize) {
        self.selected_match = self.selected_match.min(match_count.saturating_sub(1));
        self.line_offset = 0;
    }

    /// Set whether a fetch for the active file is still pending.
    pub fn set_loading(&mut self, loading: bool) {
        self.loading = loading;
    }

    /// Check whether there's preview data for the given path.
    pub fn has_data(&self, path: &Path) -> bool {
        self.data.contains_key(path)
    }

    /// Insert (or replace) cached preview data for `path`. Clears any previously stored error.
    pub fn set_data(&mut self, path: PathBuf, data: Arc<PreviewData>) {
        self.error.remove(&path);
        self.data.insert(path, data);
    }

    /// Record an error for `path`. Clears any cached data for the same path.
    pub fn set_error(&mut self, path: PathBuf, message: String) {
        self.data.remove(&path);
        self.error.insert(path, message);
    }

    /// Drop both data and error caches for `path`.
    pub fn remove_path(&mut self, path: &Path) {
        self.data.remove(path);
        self.error.remove(path);
    }

    /// Drop cached entries (both data and error) for paths not satisfying `predicate`.
    pub fn retain<F: FnMut(&Path) -> bool>(&mut self, mut predicate: F) {
        self.data.retain(|p, _| predicate(p));
        self.error.retain(|p, _| predicate(p));
    }

    /// Handle a key event for the preview.
    ///
    /// Returns [`PreviewOutcome::Continue`] when the event isn't recognized so the caller can fall
    /// back to its own handling.
    ///
    /// `match_count` is the number of matches in the active file, used to clamp downward navigation.
    pub fn handle_key_event(&mut self, match_count: usize, key: KeyEvent) -> PreviewOutcome {
        if key.kind != KeyEventKind::Press {
            return PreviewOutcome::Continue;
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.move_down(match_count);
                PreviewOutcome::Used
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.move_up();
                PreviewOutcome::Used
            }
            KeyCode::Char(' ') => PreviewOutcome::ToggleSkip,
            KeyCode::Enter => PreviewOutcome::Apply,
            KeyCode::Char('h') | KeyCode::Esc | KeyCode::Left => PreviewOutcome::Leave,
            _ => PreviewOutcome::Continue,
        }
    }

    /// Move the selection down by one row: scroll within the current match if room remains, otherwise advance
    /// to the next match (clamped to `match_count`).
    fn move_down(&mut self, match_count: usize) {
        if match_count == 0 {
            return;
        }
        if self.line_offset < self.line_offset_max {
            self.line_offset += 1;
        } else {
            let new = (self.selected_match + 1).min(match_count - 1);
            if new != self.selected_match {
                self.selected_match = new;
                self.line_offset = 0;
            }
        }
    }

    /// Move the selection up by one row: scroll within the current match if scrolled, otherwise step back
    /// to the previous match (and scroll to its bottom).
    fn move_up(&mut self) {
        if self.line_offset > 0 {
            self.line_offset -= 1;
        } else {
            let new = self.selected_match.saturating_sub(1);
            if new != self.selected_match {
                self.selected_match = new;
                // scroll to bottom of the previous match
                self.line_offset = usize::MAX;
            }
        }
    }
}

/// Preview widget for the matches in a single file.
pub struct Preview<'a> {
    pub file: Option<&'a FileMatches>,
    pub replacement: &'a str,
    pub mode: MatchMode,
    pub focused: bool,
    pub border_style: Style,
}

impl Preview<'_> {
    fn format_title(&self, area_width: u16) -> String {
        let title_max = area_width.saturating_sub(2) as usize; // border chars
        let digit = Pane::Preview.digit();
        self.file.map_or_else(
            || format!("\u{2500}[{digit}]\u{2500}Preview"),
            |fm| {
                let path_str = fm
                    .responsive_path
                    .as_ref()
                    .map_or(fm.path.to_string_lossy().into(), ToString::to_string);
                let prefix = format!("\u{2500}[{digit}]\u{2500}Preview: ");
                let full = format!("{prefix}{path_str}");
                if full.width() <= title_max {
                    full
                } else {
                    // truncate path from the left with ellipsis
                    let path_str =
                        trim_start_to_width(&path_str, title_max - prefix.width(), true).0;
                    format!("{prefix}\u{2026}{path_str}")
                }
            },
        )
    }
}

impl StatefulWidget for Preview<'_> {
    type State = PreviewState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let title = self.format_title(area.width);
        let block = Block::bordered()
            .border_set(border::ROUNDED)
            .border_style(self.border_style)
            .title(title);

        let Some(fm) = self.file else {
            let inner = block.inner(area);
            block.render(area, buf);
            render_message(buf, inner, "Select a file", Style::default().dim());
            return;
        };

        if let Some(err) = state.error.get(&fm.path) {
            let inner = block.inner(area);
            let msg = err.clone();
            block.render(area, buf);
            render_message(buf, inner, &msg, Style::default().fg(Color::Red));
            return;
        }

        let Some(preview) = state.data.get(&fm.path).cloned() else {
            let inner = block.inner(area);
            block.render(area, buf);
            let msg = if state.loading {
                "Loading preview\u{2026}"
            } else {
                ""
            };
            render_message(buf, inner, msg, Style::default().dim());
            return;
        };

        let v_scroll = Scroll::vertical().style(self.border_style);
        let scroll_area = ScrollArea::new()
            .block(Some(&block))
            .v_scroll(Some(&v_scroll));
        let inner = scroll_area.inner(area, None, Some(&state.scroll));

        let inner_height = inner.height as usize;

        let builder = PreviewBuilder::new(
            &fm.matches,
            &preview,
            self.replacement,
            self.mode,
            self.focused,
            state.selected_match,
            inner.width,
        );

        let (total_lines, selected_range) = builder.layout();

        // compute how far we can scroll within the selected match
        let selected_height = selected_range.end - selected_range.start;
        state.line_offset_max = selected_height.saturating_sub(inner_height);
        state.line_offset = state.line_offset.min(state.line_offset_max);

        // set up scroll state with counted totals so we know the visible offset
        state.scroll.set_page_len(inner_height);
        state
            .scroll
            .set_max_offset(total_lines.saturating_sub(inner_height));
        state.scroll.scroll_to_range(selected_range);

        // apply the extra line offset for tall matches
        if state.line_offset > 0 {
            state.scroll.scroll_down(state.line_offset);
        }

        let offset = state.scroll.offset;
        let visible_range = offset..(offset + inner_height);
        let lines = builder.build(visible_range);

        scroll_area.render(
            area,
            buf,
            &mut ScrollAreaState::new()
                .area(area)
                .v_scroll(&mut state.scroll),
        );

        #[expect(clippy::cast_possible_truncation)]
        Paragraph::new(lines)
            .scroll((offset as u16, 0))
            .render(inner, buf);
    }
}

/// Outcome of [`handle_key_event`](PreviewState::handle_key_event).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewOutcome {
    /// Event not handled by the preview.
    Continue,

    /// Event consumed by the preview (navigation only).
    Used,

    /// User pressed the apply key on the currently selected match.
    Apply,

    /// User asked to toggle the `skip` flag on the currently selected match.
    ToggleSkip,

    /// User asked to leave the preview pane.
    Leave,
}

/// Render a centered message in the given inner area.
fn render_message(buf: &mut Buffer, inner: Rect, msg: &str, style: Style) {
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let line_y = inner.y + inner.height.saturating_sub(1) / 2;
    let line_rect = Rect {
        x: inner.x,
        y: line_y,
        width: inner.width,
        height: 1,
    };
    Paragraph::new(msg)
        .style(style)
        .alignment(Alignment::Center)
        .render(line_rect, buf);
}
