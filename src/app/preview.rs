use std::path::PathBuf;

use crate::{
    app::App,
    prelude::OrPanic as _,
    preview::{PreviewCommand, PreviewRequest, PreviewResult},
};

impl App {
    pub fn reset_preview_state(&mut self) {
        let _ = self.preview_cmd_tx.send(PreviewCommand::Clear);
        self.preview_data.clear();
        self.preview_error.clear();
        self.preview_loading = false;
        *self.preview_wanted.write().or_panic("poisoned lock") = [None, None, None];
    }

    pub fn dispatch_preview(&mut self) {
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
        let mode = self.options.match_mode;
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
                    hash: fm.hash,
                    pattern: pattern.clone(),
                    mode,
                    generation: self.preview_generation,
                }));
        }
        self.preview_loading = !self.preview_data.contains_key(&active_path);
    }

    pub fn poll_preview_results(&mut self) {
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
                    hash: content_hash,
                    data,
                    ..
                } => {
                    self.preview_error.remove(&path);
                    self.preview_data.insert(path.clone(), data);
                    let Some(fm) = self.results.iter_mut().find(|fm| fm.path == path) else {
                        continue;
                    };
                    fm.matches = matches;
                    fm.hash = content_hash;
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

    pub fn invalidate_preview_for(&mut self, path: &PathBuf) {
        let _ = self
            .preview_cmd_tx
            .send(PreviewCommand::Invalidate(path.clone()));
        self.preview_data.remove(path);
        self.preview_error.remove(path);
    }
}
