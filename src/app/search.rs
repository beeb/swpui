use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use tracing::debug;

use crate::{
    app::App,
    glob::{GlobErrorOrigin, GlobFilters},
    path::ResponsivePath,
    search::{SearchRequest, SearchResult, WorkerCommand},
    utils::results_mem_bytes,
};

const DEBOUNCE: Duration = Duration::from_millis(100);

impl App {
    pub fn poll_search_results(&mut self) {
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

    pub fn debounce_search(&mut self) {
        if !self.pending_search && !self.pending_rebuild {
            return;
        }
        if let Some(last) = self.last_keystroke
            && last.elapsed() >= DEBOUNCE
        {
            if self.pending_rebuild {
                self.apply_glob_edit();
            } else {
                self.dispatch_search();
            }
            self.pending_search = false;
            self.pending_rebuild = false;
        }
    }

    pub fn schedule_search(&mut self) {
        self.last_keystroke = Some(Instant::now());
        self.pending_search = true;
    }

    pub fn schedule_rebuild(&mut self) {
        self.last_keystroke = Some(Instant::now());
        self.pending_rebuild = true;
    }

    pub fn rebuild_file_list(&mut self) {
        let _ = self
            .cmd_tx
            .send(WorkerCommand::Rebuild((&self.options).into()));
        self.dispatch_search();
    }

    pub fn drop_results_in_background(&mut self) {
        let old = std::mem::take(&mut self.results);
        thread::spawn(move || drop(old));
    }

    pub fn dispatch_search(&mut self) {
        debug!(
            old_len = self.results.len(),
            old_capacity = self.results.capacity(),
            old_mem_bytes = results_mem_bytes(&self.results),
            "clearing results"
        );
        self.drop_results_in_background();
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
        self.preview.reset_position();
        let _ = self.cmd_tx.send(
            SearchRequest {
                pattern: pattern.to_string(),
                mode: self.options.match_mode,
                generation: self.generation,
            }
            .into(),
        );
    }

    /// Parse and validate the filter inputs.
    ///
    /// On success store them and rebuild the file list.
    fn apply_glob_edit(&mut self) {
        self.include_input.set_invalid(false);
        self.exclude_input.set_invalid(false);
        let globs = GlobFilters::parse(self.include_input.text(), self.exclude_input.text());
        if globs == self.options.globs {
            return;
        }
        if let Err(e) = globs.overrides(&self.root) {
            match e.origin {
                GlobErrorOrigin::Include => self.include_input.set_invalid(true),
                GlobErrorOrigin::Exclude => self.exclude_input.set_invalid(true),
                GlobErrorOrigin::Build => {
                    self.include_input.set_invalid(true);
                    self.exclude_input.set_invalid(true);
                }
            }
            self.status_message = Some(e.to_string());
            return;
        }
        self.options.globs = globs;
        self.rebuild_file_list();
    }
}
