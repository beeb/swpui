use std::path::Path;

use crate::{
    app::App,
    prelude::OrPanic as _,
    preview::{PreviewCommand, PreviewRequest, PreviewResult},
};

impl App {
    pub fn reset_preview_state(&mut self) {
        let _ = self.preview_cmd_tx.send(PreviewCommand::Clear);
        self.preview.clear();
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

        let is_wanted = |p: &Path| wanted.iter().any(|w| w.as_deref() == Some(p));
        self.preview.retain(is_wanted);

        let pattern = self.search_input.text().to_string();
        let mode = self.options.match_mode;
        for slot in wanted.iter().flatten() {
            if self.preview.has_data(slot) {
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
        self.preview
            .set_loading(!self.preview.has_data(&active_path));
    }

    pub fn poll_preview_results(&mut self) {
        while let Ok(result) = self.preview_result_rx.try_recv() {
            let active = self
                .results
                .get(self.selected_file())
                .map(|fm| fm.path.clone());
            match result {
                PreviewResult::Ready { path, data, .. } => {
                    let is_active = Some(&path) == active.as_ref();
                    self.preview.set_data(path, data);
                    if is_active {
                        self.preview.set_loading(false);
                    }
                }
                PreviewResult::Updated {
                    path,
                    matches,
                    hash: content_hash,
                    data,
                    ..
                } => {
                    let is_active = Some(&path) == active.as_ref();
                    self.preview.set_data(path.clone(), data);
                    let Some(fm) = self.results.iter_mut().find(|fm| fm.path == path) else {
                        continue;
                    };
                    fm.matches = matches;
                    fm.hash = content_hash;
                    if is_active {
                        self.preview.reset_position();
                        self.preview.set_loading(false);
                    }
                }
                PreviewResult::Removed { path, .. } => {
                    let Some(idx) = self.results.iter().position(|fm| fm.path == path) else {
                        continue;
                    };
                    self.results.remove(idx);
                    self.preview.remove_path(&path);
                    self.clamp_selection();
                    self.dispatch_preview();
                }
                PreviewResult::Error { path, message, .. } => {
                    let is_active = Some(&path) == active.as_ref();
                    self.preview.set_error(path, message);
                    if is_active {
                        self.preview.set_loading(false);
                    }
                }
            }
        }
    }

    pub fn invalidate_preview_for(&mut self, path: &Path) {
        let _ = self
            .preview_cmd_tx
            .send(PreviewCommand::Invalidate(path.to_path_buf()));
        self.preview.remove_path(path);
    }
}
