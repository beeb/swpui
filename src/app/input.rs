use rat_widget::{event::TextOutcome, text_input};
use ratatui::{
    crossterm::event::{
        Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    layout::Position,
};

use crate::{app::App, types::Pane, ui::preview};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        // confirmation modal intercepts all keys
        if self.confirm_apply_all {
            match key.code {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.confirm_apply_all = false;
                    self.apply_all();
                }
                _ => self.confirm_apply_all = false,
            }
            return;
        }

        if self.options_open {
            self.handle_options_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit = true;
                return;
            }
            KeyCode::BackTab => {
                self.focused_pane = self.focused_pane.prev(self.filter_view);
                while self.searching && !self.focused_pane.is_input() {
                    self.focused_pane = self.focused_pane.prev(self.filter_view);
                }
                return;
            }
            KeyCode::Tab => {
                self.focused_pane = self.focused_pane.next(self.filter_view);
                while self.searching && !self.focused_pane.is_input() {
                    self.focused_pane = self.focused_pane.next(self.filter_view);
                }
                return;
            }
            KeyCode::Char('r')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.options.match_mode = self.options.match_mode.toggle();
                if !self.search_input.text().is_empty() {
                    self.dispatch_search();
                }
                return;
            }
            KeyCode::Char('o')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.options_open = !self.options_open;
                return;
            }
            KeyCode::Char('g')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.toggle_filter_view();
                return;
            }
            KeyCode::Esc if self.focused_pane.is_input() && !self.searching => {
                self.focused_pane = Pane::FileList;
                return;
            }
            _ => {}
        }

        match self.focused_pane {
            Pane::SearchInput => {
                let outcome =
                    text_input::handle_events(&mut self.search_input, true, &Event::Key(key));
                if outcome == TextOutcome::TextChanged {
                    self.schedule_search();
                }
            }
            Pane::ReplaceInput => {
                text_input::handle_events(&mut self.replace_input, true, &Event::Key(key));
            }
            Pane::IncludeInput => {
                let outcome =
                    text_input::handle_events(&mut self.include_input, true, &Event::Key(key));
                if outcome == TextOutcome::TextChanged {
                    self.schedule_rebuild();
                }
            }
            Pane::ExcludeInput => {
                let outcome =
                    text_input::handle_events(&mut self.exclude_input, true, &Event::Key(key));
                if outcome == TextOutcome::TextChanged {
                    self.schedule_rebuild();
                }
            }
            Pane::FileList if !self.searching => self.handle_file_list_key(key),
            Pane::Preview if !self.searching => self.handle_preview_key(key),
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        // modals swallow mouse input, mirroring key handling
        if self.confirm_apply_all || self.options_open {
            return;
        }
        let pos = Position::new(mouse.column, mouse.row);
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => self.handle_click(pos, mouse),
            MouseEventKind::ScrollDown => self.handle_scroll(pos, ScrollDir::Down),
            MouseEventKind::ScrollUp => self.handle_scroll(pos, ScrollDir::Up),
            _ => {}
        }
    }

    fn handle_options_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit = true;
            }
            KeyCode::Esc => {
                self.options_open = false;
            }
            KeyCode::Char('o')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.options_open = false;
            }
            KeyCode::Char('r') => {
                self.options.match_mode = self.options.match_mode.toggle();
                if !self.search_input.text().is_empty() {
                    self.dispatch_search();
                }
            }
            KeyCode::Char('h') => {
                self.options.include_hidden = !self.options.include_hidden;
                self.rebuild_file_list();
            }
            KeyCode::Char('g') => {
                self.options.include_gitignored = !self.options.include_gitignored;
                self.rebuild_file_list();
            }
            _ => {}
        }
    }

    /// Swap between the search/replace and include/exclude input views.
    fn toggle_filter_view(&mut self) {
        self.filter_view = !self.filter_view;
        self.focused_pane = match (self.filter_view, self.focused_pane) {
            (true, Pane::SearchInput) => Pane::IncludeInput,
            (true, Pane::ReplaceInput) => Pane::ExcludeInput,
            (false, Pane::IncludeInput) => Pane::SearchInput,
            (false, Pane::ExcludeInput) => Pane::ReplaceInput,
            (_, pane) => pane,
        };
    }

    fn select_next_file(&mut self) {
        if self.results.is_empty() {
            return;
        }
        let next = (self.selected_file() + 1).min(self.results.len() - 1);
        self.file_list.select(Some(next));
        self.preview.reset_position();
        self.dispatch_preview();
    }

    fn select_prev_file(&mut self) {
        let prev = self.selected_file().saturating_sub(1);
        self.file_list.select(Some(prev));
        self.preview.reset_position();
        self.dispatch_preview();
    }

    fn handle_click(&mut self, pos: Position, mouse: MouseEvent) {
        let Some(pane) = self.pane_areas.pane_at(pos) else {
            return;
        };
        // during search, only input panes are focusable (mirrors Tab)
        if self.searching && !pane.is_input() {
            return;
        }
        self.focused_pane = pane;
        match pane {
            Pane::SearchInput => {
                text_input::handle_events(&mut self.search_input, true, &Event::Mouse(mouse));
            }
            Pane::ReplaceInput => {
                text_input::handle_events(&mut self.replace_input, true, &Event::Mouse(mouse));
            }
            Pane::IncludeInput => {
                text_input::handle_events(&mut self.include_input, true, &Event::Mouse(mouse));
            }
            Pane::ExcludeInput => {
                text_input::handle_events(&mut self.exclude_input, true, &Event::Mouse(mouse));
            }
            Pane::FileList => {
                if let Some(idx) = self.file_list.row_at_clicked((pos.x, pos.y))
                    && Some(idx) != self.file_list.selected()
                {
                    self.file_list.select(Some(idx));
                    self.preview.reset_position();
                    self.dispatch_preview();
                }
            }
            Pane::Preview => {
                if let Some(idx) = self.preview.match_at(pos) {
                    self.preview.select_match(idx);
                }
            }
        }
    }

    fn handle_scroll(&mut self, pos: Position, dir: ScrollDir) {
        let Some(pane) = self.pane_areas.pane_at(pos) else {
            return;
        };
        if self.searching && !pane.is_input() {
            return;
        }
        match pane {
            Pane::FileList => {
                if matches!(dir, ScrollDir::Down) {
                    self.select_next_file();
                } else {
                    self.select_prev_file();
                }
            }
            Pane::Preview => {
                let count = self
                    .results
                    .get(self.selected_file())
                    .map_or(0, |fm| fm.matches.len());
                if matches!(dir, ScrollDir::Down) {
                    self.preview.move_down(count);
                } else {
                    self.preview.move_up();
                }
            }
            Pane::SearchInput | Pane::ReplaceInput | Pane::IncludeInput | Pane::ExcludeInput => {}
        }
    }

    fn handle_non_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char('s') => self.toggle_skip_file(),
            KeyCode::Char('f') => self.apply_file(),
            KeyCode::Char(c) => {
                // TODO: rewrite as if-let guard when updating to rust 1.95
                if let Some(pane) = Pane::from_digit(c) {
                    match pane {
                        Pane::SearchInput | Pane::ReplaceInput => self.filter_view = false,
                        Pane::IncludeInput | Pane::ExcludeInput => self.filter_view = true,
                        Pane::FileList | Pane::Preview => {}
                    }
                    self.focused_pane = pane;
                }
            }
            _ => {}
        }
    }

    fn handle_file_list_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('a') if !self.results.is_empty() => {
                self.confirm_apply_all = true;
                return;
            }
            KeyCode::Char('j') | KeyCode::Down if !self.results.is_empty() => {
                self.select_next_file();
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.select_prev_file();
                return;
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right if !self.results.is_empty() => {
                self.focused_pane = Pane::Preview;
                return;
            }
            KeyCode::Char(' ') => {
                self.toggle_skip_file();
                return;
            }
            _ => {}
        }
        self.handle_non_input_key(key);
    }

    fn handle_preview_key(&mut self, key: KeyEvent) {
        let sel = self.selected_file();
        let match_count = self.results.get(sel).map_or(0, |fm| fm.matches.len());
        let outcome = self.preview.handle_key_event(match_count, key);
        match outcome {
            preview::PreviewOutcome::Used => {}
            preview::PreviewOutcome::Apply => self.apply_single_match(),
            preview::PreviewOutcome::ToggleSkip => {
                if let Some(fm) = self.results.get_mut(sel)
                    && let Some(m) = fm.matches.get_mut(self.preview.selected_match())
                {
                    m.skip = !m.skip;
                }
            }
            preview::PreviewOutcome::Leave => self.focused_pane = Pane::FileList,
            preview::PreviewOutcome::Continue => self.handle_non_input_key(key),
        }
    }
}

#[derive(Clone, Copy)]
enum ScrollDir {
    Up,
    Down,
}
