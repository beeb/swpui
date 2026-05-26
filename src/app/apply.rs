use std::{fs, slice};

use crate::{app::App, config::MatchMode, replace, search::FileMatches};

impl App {
    pub fn toggle_skip_file(&mut self) {
        let sel = self.selected_file();
        let Some(fm) = self.results.get_mut(sel) else {
            return;
        };
        let all_skipped = fm.matches.iter().all(|m| m.skip);
        for m in &mut fm.matches {
            m.skip = !all_skipped;
        }
    }

    pub fn apply_all(&mut self) {
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.options.match_mode);
        let mut to_remove = Vec::with_capacity(self.results.len());
        for (i, fm) in self.results.iter().enumerate() {
            if replace::has_overlapping_matches(&fm.matches) {
                self.status_message = Some(format!(
                    "Overlapping matches in {}, skipping",
                    fm.path.display()
                ));
                continue;
            }
            if let Err(e) = Self::apply_to_file(fm, &replacement, self.options.match_mode) {
                self.status_message = Some(format!("{}: {e}", fm.path.display()));
            } else {
                to_remove.push((i, fm.path.clone()));
            }
        }
        if to_remove.len() == self.results.len() {
            self.drop_results_in_background();
        } else {
            for (i, p) in to_remove.into_iter().rev() {
                self.results.swap_remove(i);
                self.invalidate_preview_for(&p);
            }
        }
        self.clamp_selection();
        self.dispatch_preview();
    }

    pub fn apply_file(&mut self) {
        let sel = self.selected_file();
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.options.match_mode);
        let Some(fm) = self.results.get(sel) else {
            return;
        };
        if replace::has_overlapping_matches(&fm.matches) {
            self.status_message = Some(format!("Overlapping matches in {}", fm.path.display()));
            return;
        }
        let path_to_remove = fm.path.clone();
        if let Err(e) = Self::apply_to_file(fm, &replacement, self.options.match_mode) {
            self.status_message = Some(e.to_string());
        } else {
            self.results.remove(sel);
            self.invalidate_preview_for(&path_to_remove);
            self.clamp_selection();
            self.dispatch_preview();
        }
    }

    pub fn apply_single_match(&mut self) {
        let sel = self.selected_file();
        let replacement =
            replace::effective_replacement(self.replace_input.text(), self.options.match_mode);
        let Some(fm) = self.results.get_mut(sel) else {
            return;
        };
        let Some(m) = fm.matches.get(self.preview.selected_match()) else {
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
        let new_content = replace::apply_replacements(
            content,
            slice::from_ref(m),
            &replacement,
            self.options.match_mode,
        );
        if let Err(e) = replace::write_file(&fm.path, &new_content) {
            self.status_message = Some(format!("{}: {e}", fm.path.display()));
            return;
        }
        let path_to_remove = fm.path.clone();
        // remove this match from the results
        // if no matches left, remove the file
        fm.matches.remove(self.preview.selected_match());
        if fm.matches.is_empty() {
            self.results.remove(sel);
        }
        self.invalidate_preview_for(&path_to_remove);
        self.clamp_selection();
        self.dispatch_preview();
    }

    fn apply_to_file(fm: &FileMatches, replacement: &str, mode: MatchMode) -> anyhow::Result<()> {
        if !fm.hash.matches(&fm.path)? {
            anyhow::bail!("file modified externally, skipping");
        }
        let content = fs::read_to_string(&fm.path)?;
        let new_content = replace::apply_replacements(content, &fm.matches, replacement, mode);
        replace::write_file(&fm.path, &new_content)?;
        Ok(())
    }
}
