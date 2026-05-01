use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    slice,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use rat_widget::{
    event::TextOutcome,
    list::ListState,
    scrolled::ScrollState,
    text_input::{self, TextInputState},
};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
};

use tracing::debug;

use crate::{
    path::ResponsivePath,
    prelude::OrPanic as _,
    preview::{
        PreviewCommand, PreviewRequest, PreviewResult, PreviewWorker, WantedSet, data::PreviewData,
    },
    replace,
    search::SearchWorker,
    spinner::SpinnerState,
    types::{FileMatches, MatchMode, Pane, SearchRequest, SearchResult, WorkerCommand},
    ui,
    utils::{self, results_mem_bytes},
};

const DEBOUNCE: Duration = Duration::from_millis(100);
const POLL_TIMEOUT: Duration = Duration::from_millis(16);

#[expect(clippy::struct_excessive_bools)]
pub struct App {
    pub root: PathBuf,
    pub search_input: TextInputState,
    pub replace_input: TextInputState,
    pub match_mode: MatchMode,
    pub results: Vec<FileMatches>,
    pub focused_pane: Pane,
    pub file_list: ListState,
    pub selected_match: usize,
    /// Extra line offset within the selected match (for tall matches that exceed the viewport).
    pub preview_line_offset: usize,
    /// Max value for `preview_line_offset`, computed during render.
    pub preview_line_offset_max: usize,
    pub preview_scroll: ScrollState,
    pub status_message: Option<String>,
    pub searching: bool,
    pub truncated: bool,
    pub spinner: SpinnerState,
    pub confirm_apply_all: bool,
    pub include_hidden: bool,
    pub preview_data: HashMap<PathBuf, Arc<PreviewData>>,
    pub preview_error: HashMap<PathBuf, String>,
    pub preview_loading: bool,
    exit: bool,
    generation: u64,
    last_keystroke: Option<Instant>,
    pending_search: bool,
    cmd_tx: mpsc::Sender<WorkerCommand>,
    result_rx: mpsc::Receiver<SearchResult>,
    cancelled: Arc<AtomicBool>,
    preview_wanted: WantedSet,
    preview_cmd_tx: mpsc::Sender<PreviewCommand>,
    preview_result_rx: mpsc::Receiver<PreviewResult>,
    preview_generation: u64,
}

impl App {
    pub fn new(root: PathBuf) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (result_tx, result_rx) = mpsc::channel();
        let cancelled = Arc::new(AtomicBool::new(false));

        let worker = SearchWorker::new(root.clone(), cmd_rx, result_tx, Arc::clone(&cancelled))?;
        thread::spawn(move || worker.run());

        let (preview_cmd_tx, preview_cmd_rx) = mpsc::channel();
        let (preview_result_tx, preview_result_rx) = mpsc::channel();
        let preview_wanted: WantedSet = Arc::new(RwLock::new([None, None, None]));
        let preview_worker = PreviewWorker::new(
            preview_cmd_rx,
            preview_result_tx,
            Arc::clone(&preview_wanted),
        );
        thread::spawn(move || preview_worker.run());

        Ok(Self {
            root,
            search_input: TextInputState::new(),
            replace_input: TextInputState::new(),
            match_mode: MatchMode::default(),
            results: Vec::new(),
            focused_pane: Pane::default(),
            file_list: ListState::default(),
            selected_match: 0,
            preview_line_offset: 0,
            preview_line_offset_max: 0,
            preview_scroll: ScrollState::new(),
            status_message: None,
            searching: false,
            truncated: false,
            spinner: SpinnerState::default(),
            confirm_apply_all: false,
            include_hidden: true,
            preview_data: HashMap::new(),
            preview_error: HashMap::new(),
            preview_loading: false,
            exit: false,
            generation: 0,
            last_keystroke: None,
            pending_search: false,
            cmd_tx,
            result_rx,
            cancelled,
            preview_wanted,
            preview_cmd_tx,
            preview_result_rx,
            preview_generation: 0,
        })
    }

    pub fn selected_file(&self) -> usize {
        self.file_list.selected().unwrap_or_default()
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> anyhow::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.poll_events()?;
            self.poll_search_results();
            self.poll_preview_results();
            self.maybe_send_search();
            if self.searching {
                self.spinner.tick();
            }
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame) {
        ui::render(self, frame);
    }

    fn poll_events(&mut self) -> anyhow::Result<()> {
        if event::poll(POLL_TIMEOUT)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            self.handle_key(key);
        }
        Ok(())
    }

    fn poll_search_results(&mut self) {
        while let Ok(result) = self.result_rx.try_recv() {
            match result {
                SearchResult::FileMatches {
                    generation,
                    mut file_matches,
                } if generation == self.generation => {
                    file_matches.responsive_path =
                        ResponsivePath::new(&file_matches.path, Some(&self.root)).ok();
                    self.results.push(file_matches);
                }
                SearchResult::Complete {
                    generation,
                    truncated,
                } if generation == self.generation => {
                    self.results.sort_unstable_by(|a, b| a.path.cmp(&b.path));
                    self.searching = false;
                    self.truncated = truncated;
                    self.dispatch_preview();
                    let total: usize = self.results.iter().map(|fm| fm.matches.len()).sum();
                    debug!(
                        generation,
                        files = self.results.len(),
                        results_capacity = self.results.capacity(),
                        results_mem_bytes = results_mem_bytes(&self.results),
                        total_matches = total,
                        truncated,
                        "received search complete"
                    );
                    if truncated {
                        self.status_message = Some(format!("Results capped at {total} matches"));
                    }
                }
                SearchResult::Error {
                    generation,
                    message,
                } if generation == self.generation => {
                    self.status_message = Some(message);
                    self.searching = false;
                    self.search_input.set_invalid(true);
                }
                SearchResult::FileListReady { count, truncated } => {
                    if truncated {
                        self.status_message = Some(format!("File list capped at {count} files"));
                    }
                }
                SearchResult::FileMatches { .. }
                | SearchResult::Complete { .. }
                | SearchResult::Error { .. } => {}
            }
        }
    }

    fn maybe_send_search(&mut self) {
        if !self.pending_search {
            return;
        }
        if let Some(last) = self.last_keystroke
            && last.elapsed() >= DEBOUNCE
        {
            self.dispatch_search();
            self.pending_search = false;
        }
    }

    fn schedule_search(&mut self) {
        self.last_keystroke = Some(Instant::now());
        self.pending_search = true;
    }

    fn reset_preview_state(&mut self) {
        let _ = self.preview_cmd_tx.send(PreviewCommand::Clear);
        self.preview_data.clear();
        self.preview_error.clear();
        self.preview_loading = false;
        *self.preview_wanted.write().or_panic("poisoned lock") = [None, None, None];
    }

    fn dispatch_search(&mut self) {
        debug!(
            old_len = self.results.len(),
            old_capacity = self.results.capacity(),
            old_mem_bytes = results_mem_bytes(&self.results),
            "clearing results"
        );
        Self::drop_results_in_background(&mut self.results);
        self.status_message = None;
        self.truncated = false;
        self.search_input.set_invalid(false);
        self.cancelled.store(true, Ordering::Relaxed); // cancel any ongoing search
        self.reset_preview_state();
        self.generation += 1;
        let pattern = self.search_input.text();
        if pattern.is_empty() {
            self.searching = false;
            return;
        }
        self.searching = true;
        self.file_list.select(Some(0));
        self.selected_match = 0;
        self.preview_line_offset = 0;
        self.preview_scroll.clear();
        let _ = self.cmd_tx.send(WorkerCommand::Search(SearchRequest {
            pattern: pattern.to_string(),
            mode: self.match_mode,
            generation: self.generation,
        }));
    }

    fn dispatch_preview(&mut self) {
        if self.results.is_empty() {
            self.reset_preview_state();
            return;
        }
        let active_idx = self.selected_file();
        let active_path = self.results[active_idx].path.clone();
        let next_path = self.results.get(active_idx + 1).map(|fm| fm.path.clone());
        let prev_path = active_idx
            .checked_sub(1)
            .and_then(|i| self.results.get(i).map(|fm| fm.path.clone()));
        let wanted = [Some(active_path.clone()), next_path, prev_path];
        self.preview_wanted
            .write()
            .or_panic("poisoned lock")
            .clone_from(&wanted);

        let is_wanted = |p: &PathBuf| wanted.iter().any(|w| w.as_ref() == Some(p));
        self.preview_data.retain(|p, _| is_wanted(p));
        self.preview_error.retain(|p, _| is_wanted(p));

        let pattern = self.search_input.text().to_string();
        let mode = self.match_mode;
        for slot in wanted.iter().flatten() {
            if self.preview_data.contains_key(slot) {
                // data is already available
                continue;
            }
            let Some(fm) = self.results.iter().find(|fm| &fm.path == slot) else {
                continue;
            };
            let byte_ranges: Box<[(usize, usize)]> = fm
                .matches
                .iter()
                .map(|m| (m.byte_offset_start, m.byte_offset_end))
                .collect();
            self.preview_generation += 1;
            let _ = self
                .preview_cmd_tx
                .send(PreviewCommand::Request(PreviewRequest {
                    path: slot.clone(),
                    byte_ranges,
                    content_hash: fm.content_hash,
                    pattern: pattern.clone(),
                    mode,
                    generation: self.preview_generation,
                }));
        }
        self.preview_loading = !self.preview_data.contains_key(&active_path);
    }

    fn poll_preview_results(&mut self) {
        while let Ok(result) = self.preview_result_rx.try_recv() {
            let active = self
                .results
                .get(self.selected_file())
                .map(|fm| fm.path.clone());
            match result {
                PreviewResult::Ready { path, data, .. } => {
                    self.preview_error.remove(&path);
                    self.preview_data.insert(path.clone(), data);
                    if Some(&path) == active.as_ref() {
                        self.preview_loading = false;
                    }
                }
                PreviewResult::Updated {
                    path,
                    matches,
                    content_hash,
                    data,
                    ..
                } => {
                    self.preview_error.remove(&path);
                    self.preview_data.insert(path.clone(), data);
                    let Some(fm) = self.results.iter_mut().find(|fm| fm.path == path) else {
                        continue;
                    };
                    fm.matches = matches;
                    fm.content_hash = content_hash;
                    if Some(&path) == active.as_ref() {
                        self.selected_match = 0;
                        self.preview_line_offset = 0;
                        self.preview_scroll.clear();
                        self.preview_loading = false;
                    }
                }
                PreviewResult::Removed { path, .. } => {
                    let Some(idx) = self.results.iter().position(|fm| fm.path == path) else {
                        continue;
                    };
                    self.results.remove(idx);
                    self.preview_data.remove(&path);
                    self.preview_error.remove(&path);
                    self.clamp_selection();
                    self.dispatch_preview();
                }
                PreviewResult::Error { path, message, .. } => {
                    self.preview_data.remove(&path);
                    self.preview_error.insert(path.clone(), message);
                    if Some(&path) == active.as_ref() {
                        self.preview_loading = false;
                    }
                }
            }
        }
    }

    fn invalidate_preview_for(&mut self, path: &PathBuf) {
        let _ = self
            .preview_cmd_tx
            .send(PreviewCommand::Invalidate(path.clone()));
        self.preview_data.remove(path);
        self.preview_error.remove(path);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // confirmation modal intercepts all keys
        if self.confirm_apply_all {
            match key.code {
                KeyCode::Char('y') => {
                    self.confirm_apply_all = false;
                    self.apply_all();
                }
                _ => self.confirm_apply_all = false,
            }
            return;
        }

        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit = true;
                return;
            }
            KeyCode::BackTab => {
                self.focused_pane = self.focused_pane.prev();
                while self.searching && !self.focused_pane.is_input() {
                    self.focused_pane = self.focused_pane.prev();
                }
                return;
            }
            KeyCode::Tab => {
                self.focused_pane = self.focused_pane.next();
                while self.searching && !self.focused_pane.is_input() {
                    self.focused_pane = self.focused_pane.next();
                }
                return;
            }
            KeyCode::Char('r')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.match_mode = self.match_mode.toggle();
                if !self.search_input.text().is_empty() {
                    self.dispatch_search();
                }
                return;
            }
            KeyCode::Char('d')
                if key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.include_hidden = !self.include_hidden;
                let _ = self.cmd_tx.send(WorkerCommand::Rebuild {
                    include_hidden: self.include_hidden,
                });
                self.dispatch_search();
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
            Pane::FileList if !self.searching => self.handle_file_list_key(key),
            Pane::Preview if !self.searching => self.handle_preview_key(key),
            _ => {}
        }
    }

    fn handle_non_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.exit = true,
            KeyCode::Char('s') => self.toggle_skip_file(),
            KeyCode::Char('f') => self.apply_file(),
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
                let next = (self.selected_file() + 1).min(self.results.len() - 1);
                self.file_list.select(Some(next));
                self.selected_match = 0;
                self.preview_line_offset = 0;
                self.preview_scroll.clear();
                self.dispatch_preview();
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                let prev = self.selected_file().saturating_sub(1);
                self.file_list.select(Some(prev));
                self.selected_match = 0;
                self.preview_line_offset = 0;
                self.preview_scroll.clear();
                self.dispatch_preview();
                return;
            }
            KeyCode::Char('l') | KeyCode::Enter | KeyCode::Right if !self.results.is_empty() => {
                self.focused_pane = Pane::Preview;
                return;
            }
            _ => {}
        }
        self.handle_non_input_key(key);
    }

    fn handle_preview_key(&mut self, key: KeyEvent) {
        let sel = self.selected_file();
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(fm) = self.results.get(sel)
                    && !fm.matches.is_empty()
                {
                    if self.preview_line_offset < self.preview_line_offset_max {
                        self.preview_line_offset += 1;
                    } else {
                        let new = (self.selected_match + 1).min(fm.matches.len() - 1);
                        if new != self.selected_match {
                            self.selected_match = new;
                            self.preview_line_offset = 0;
                        }
                    }
                }
                return;
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if self.preview_line_offset > 0 {
                    self.preview_line_offset -= 1;
                } else {
                    let new = self.selected_match.saturating_sub(1);
                    if new != self.selected_match {
                        self.selected_match = new;
                        // scroll to bottom of the previous match
                        self.preview_line_offset = usize::MAX;
                    }
                }
                return;
            }
            KeyCode::Char(' ') => {
                if let Some(fm) = self.results.get_mut(sel)
                    && let Some(m) = fm.matches.get_mut(self.selected_match)
                {
                    m.skip = !m.skip;
                }
                return;
            }
            KeyCode::Enter => {
                self.apply_single_match();
                return;
            }
            KeyCode::Char('h') | KeyCode::Esc | KeyCode::Left => {
                self.focused_pane = Pane::FileList;
                return;
            }
            _ => {}
        }
        self.handle_non_input_key(key);
    }

    fn toggle_skip_file(&mut self) {
        let sel = self.selected_file();
        let Some(fm) = self.results.get_mut(sel) else {
            return;
        };
        let all_skipped = fm.matches.iter().all(|m| m.skip);
        for m in &mut fm.matches {
            m.skip = !all_skipped;
        }
    }

    fn apply_all(&mut self) {
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.match_mode);
        let mut to_remove = Vec::with_capacity(self.results.len());
        for (i, fm) in self.results.iter().enumerate() {
            if replace::has_overlapping_matches(&fm.matches) {
                self.status_message = Some(format!(
                    "Overlapping matches in {}, skipping",
                    fm.path.display()
                ));
                continue;
            }
            if let Err(e) = Self::apply_to_file(fm, &replacement, self.match_mode) {
                self.status_message = Some(format!("{}: {e}", fm.path.display()));
            } else {
                to_remove.push((i, fm.path.clone()));
            }
        }
        if to_remove.len() == self.results.len() {
            Self::drop_results_in_background(&mut self.results);
        } else {
            for (i, p) in to_remove.into_iter().rev() {
                self.results.swap_remove(i);
                self.invalidate_preview_for(&p);
            }
        }
        self.clamp_selection();
        self.dispatch_preview();
    }

    fn apply_file(&mut self) {
        let sel = self.selected_file();
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.match_mode);
        let Some(fm) = self.results.get(sel) else {
            return;
        };
        if replace::has_overlapping_matches(&fm.matches) {
            self.status_message = Some(format!("Overlapping matches in {}", fm.path.display()));
            return;
        }
        let path_to_remove = fm.path.clone();
        if let Err(e) = Self::apply_to_file(fm, &replacement, self.match_mode) {
            self.status_message = Some(e.to_string());
        } else {
            self.results.remove(sel);
            self.invalidate_preview_for(&path_to_remove);
            self.clamp_selection();
            self.dispatch_preview();
        }
    }

    fn apply_single_match(&mut self) {
        let sel = self.selected_file();
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.match_mode);
        let Some(fm) = self.results.get_mut(sel) else {
            return;
        };
        let Some(m) = fm.matches.get(self.selected_match) else {
            return;
        };
        if m.skip {
            return;
        }
        let content = match fs::read_to_string(&fm.path) {
            Ok(c) => c,
            Err(e) => {
                self.status_message = Some(format!("{}: {e}", fm.path.display()));
                return;
            }
        };
        let new_content =
            replace::apply_replacements(content, slice::from_ref(m), &replacement, self.match_mode);
        if let Err(e) = replace::write_file(&fm.path, &new_content) {
            self.status_message = Some(format!("{}: {e}", fm.path.display()));
            return;
        }
        let path_to_remove = fm.path.clone();
        // remove this match from the results
        // if no matches left, remove the file
        fm.matches.remove(self.selected_match);
        if fm.matches.is_empty() {
            self.results.remove(sel);
        }
        self.invalidate_preview_for(&path_to_remove);
        self.clamp_selection();
        self.dispatch_preview();
    }

    fn apply_to_file(fm: &FileMatches, replacement: &str, mode: MatchMode) -> anyhow::Result<()> {
        if utils::is_file_stale(&fm.path, fm.content_hash)? {
            anyhow::bail!("file modified externally, skipping");
        }
        let content = fs::read_to_string(&fm.path)?;
        let new_content = replace::apply_replacements(content, &fm.matches, replacement, mode);
        replace::write_file(&fm.path, &new_content)?;
        Ok(())
    }

    fn drop_results_in_background(results: &mut Vec<FileMatches>) {
        let old = std::mem::take(results);
        thread::spawn(move || drop(old));
    }

    fn clamp_selection(&mut self) {
        if self.results.is_empty() {
            self.file_list.select(Some(0));
            self.selected_match = 0;
            self.preview_line_offset = 0;
            self.preview_scroll.clear();
            self.focused_pane = Pane::FileList;
        } else {
            let clamped = self.selected_file().min(self.results.len() - 1);
            self.file_list.select(Some(clamped));
            let match_count = self.results[clamped].matches.len();
            self.selected_match = self.selected_match.min(match_count.saturating_sub(1));
            self.preview_line_offset = 0;
        }
    }
}
