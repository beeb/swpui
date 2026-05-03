use std::{
    cell::RefCell,
    collections::VecDeque,
    fmt,
    path::{Component, MAIN_SEPARATOR, MAIN_SEPARATOR_STR, Path},
};

use unicode_width::UnicodeWidthStr as _;

use crate::utils::{trim_end_to_width, trim_start_to_width};

const CACHE_SIZE: usize = 4;

/// A path that can be abbreviated depending on the available space.
#[derive(Debug, Clone)]
pub struct ResponsivePath {
    dirs: Vec<String>,
    last: Option<String>,
    cache: Cache,
}

impl fmt::Display for ResponsivePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.dirs.join(MAIN_SEPARATOR_STR))?;
        if let Some(filename) = &self.last {
            write!(f, "{MAIN_SEPARATOR_STR}{filename}")?;
        }
        Ok(())
    }
}

impl ResponsivePath {
    /// Create a new responsive path (fluid width display), optionally making it relative to some root.
    ///
    /// The paths are canonicalized in the constructor for good measure.
    pub fn new(path: impl AsRef<Path>, root: Option<impl AsRef<Path>>) -> anyhow::Result<Self> {
        let mut path = dunce::canonicalize(path)?;
        if let Some(root) = root.map(|p| dunce::canonicalize(p)).transpose()?
            && let Ok(rel) = path.strip_prefix(root)
        {
            path = rel.to_path_buf();
        }
        let mut dirs: Vec<_> = path
            .components()
            .flat_map(|comp| match comp {
                Component::Prefix(c) => c
                    .as_os_str()
                    .to_string_lossy()
                    .split([MAIN_SEPARATOR])
                    .map(ToString::to_string)
                    .collect(),
                Component::Normal(c) => vec![c.to_string_lossy().into()],
                Component::RootDir | Component::CurDir | Component::ParentDir => vec![],
            })
            .collect();
        let last = dirs.pop();
        Ok(Self {
            dirs,
            last,
            cache: Cache::default(),
        })
    }

    /// Format the path, progressively abbreviating to fit `width` display columns.
    ///
    /// 1. Full path
    /// 2. Directory segments abbreviated to 3 characters
    /// 3. Directory segments abbreviated to 2 characters
    /// 4. Directory segments abbreviated to 1 character
    /// 5. Ellipsis in the file stem
    /// 6. Right-aligned (left-truncated) compact form
    #[must_use]
    pub fn to_width(&self, width: usize) -> String {
        if let Some(out) = self.cache.get(width) {
            return out;
        }
        if self.fits(None, width) {
            let res = self.join_path(None);
            self.cache.insert(width, res.clone());
            return res;
        }

        for n in (1..=3).rev() {
            if self.fits(Some(n), width) {
                let res = self.join_path(Some(n));
                self.cache.insert(width, res.clone());
                return res;
            }
        }

        let dir_prefix = self.dir_prefix_string(Some(1));
        let compact = format!("{dir_prefix}{}", self.short_last(dir_prefix.width(), width));
        let compact_w = compact.width();

        if compact_w <= width {
            self.cache.insert(width, compact.clone());
            return compact;
        }

        let res = trim_start_to_width(&compact, width, false).0.to_string();
        self.cache.insert(width, res.clone());
        res
    }

    /// Checks whether the path would fit with each segment being `truncate` chars (and filename not abbreviated)
    fn fits(&self, truncate: Option<usize>, width: usize) -> bool {
        let last_w = self.last.as_ref().map(|s| s.width()).unwrap_or_default();
        self.dir_prefix_width(truncate) + last_w <= width
    }

    /// Directories prefix (including trailing separator)
    fn dir_prefix_string(&self, truncate: Option<usize>) -> String {
        if self.dirs.is_empty() {
            String::new()
        } else {
            format!("{}{MAIN_SEPARATOR_STR}", self.format_dirs(truncate))
        }
    }

    /// Width of the directories (including trailing separator)
    fn dir_prefix_width(&self, truncate: Option<usize>) -> usize {
        self.dir_prefix_string(truncate).width()
    }

    /// Join the path segments and filename, truncating each dir segment to `truncate` chars.
    fn join_path(&self, truncate: Option<usize>) -> String {
        let mut s = self.format_dirs(truncate);
        if let Some(last) = &self.last {
            if !s.is_empty() {
                s.push(MAIN_SEPARATOR);
            }
            s.push_str(last);
        }
        s
    }

    /// Join the path segments (without filename), truncating each dir segment to `truncate` chars.
    fn format_dirs(&self, truncate: Option<usize>) -> String {
        self.dirs
            .iter()
            .map(|dir| {
                truncate.map_or_else(|| dir.clone(), |n| dir.chars().take(n).collect::<String>())
            })
            .collect::<Vec<_>>()
            .join(MAIN_SEPARATOR_STR)
    }

    /// Compute an abbreviated form of the last segment, sized so the full path targets `width` total display columns.
    fn short_last(&self, dir_prefix_w: usize, width: usize) -> String {
        let Some(filename) = self.last.as_deref() else {
            return String::new();
        };
        let filename_w = filename.width();
        if dir_prefix_w + filename_w <= width {
            return filename.to_string();
        }
        let (stem, ext) = Self::split_stem_ext(filename);
        let stem_chars: Vec<char> = stem.chars().collect();
        if stem_chars.len() <= 3 {
            // can't abbreviate `foo.bar` while keeping the extension and first/last character of the basename
            return filename.to_string();
        }
        // overhead = dir_prefix + ellipsis + period + ext
        let dot_w = usize::from(!ext.is_empty());
        let overhead = dir_prefix_w + 1 + dot_w + ext.width();
        let budget = width.saturating_sub(overhead);
        let left = budget.div_ceil(2).max(1);
        let right = budget.saturating_sub(left).max(1);

        let (start, _) = trim_end_to_width(stem, left, false);
        let (end, _) = trim_start_to_width(stem, right, false);
        if ext.is_empty() {
            format!("{start}\u{2026}{end}")
        } else {
            format!("{start}\u{2026}{end}.{ext}")
        }
    }

    /// Split the filename into stem and extension.
    ///
    /// Dotfiles are considered as having no extension.
    fn split_stem_ext(filename: &str) -> (&str, &str) {
        match filename.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => (stem, ext),
            _ => (filename, ""),
        }
    }
}

#[derive(Debug, Default, Clone)]
struct Cache(RefCell<VecDeque<(usize, String)>>);

impl Cache {
    fn get(&self, width: usize) -> Option<String> {
        self.0
            .borrow()
            .iter()
            .find_map(|(w, s)| if w == &width { Some(s.clone()) } else { None })
    }

    fn insert(&self, width: usize, s: impl Into<String>) {
        let s = s.into();
        {
            let mut this = self.0.borrow_mut();
            if let Some(pos) = this.iter().position(|(w, _)| w == &width) {
                this.remove(pos);
            }
            this.push_front((width, s));
        }
        self.evict();
    }

    fn evict(&self) {
        let mut this = self.0.borrow_mut();
        while this.len() > CACHE_SIZE {
            this.pop_back();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make(dirs: &[&str], last: Option<&str>) -> ResponsivePath {
        ResponsivePath {
            dirs: dirs.iter().map(|s| (*s).to_string()).collect(),
            last: last.map(ToString::to_string),
            cache: Cache::default(),
        }
    }

    #[test]
    fn full_path_when_it_fits() {
        let p = make(&["src"], Some("main.rs"));
        assert_eq!(p.to_width(50), format!("src{MAIN_SEPARATOR_STR}main.rs"));
    }

    #[test]
    fn dirs_abbreviated_to_three_chars() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(25),
            format!(
                "src{MAIN_SEPARATOR_STR}com{MAIN_SEPARATOR_STR}wid{MAIN_SEPARATOR_STR}MyFile.rs"
            )
        );
    }

    #[test]
    fn dirs_abbreviated_to_two_chars() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(20),
            format!("sr{MAIN_SEPARATOR_STR}co{MAIN_SEPARATOR_STR}wi{MAIN_SEPARATOR_STR}MyFile.rs")
        );
    }

    #[test]
    fn dirs_abbreviated_to_one_char() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(17),
            format!("s{MAIN_SEPARATOR_STR}c{MAIN_SEPARATOR_STR}w{MAIN_SEPARATOR_STR}MyFile.rs")
        );
    }

    #[test]
    fn ellipsis_in_stem_minimal() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(12),
            format!("s{MAIN_SEPARATOR_STR}c{MAIN_SEPARATOR_STR}w{MAIN_SEPARATOR_STR}M\u{2026}e.rs")
        );
    }

    #[test]
    fn ellipsis_in_stem_with_more_chars() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(14),
            format!(
                "s{MAIN_SEPARATOR_STR}c{MAIN_SEPARATOR_STR}w{MAIN_SEPARATOR_STR}My\u{2026}le.rs"
            )
        );
    }

    #[test]
    fn right_aligned_truncation() {
        let p = make(&["src", "components", "widgets"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(9),
            format!("{MAIN_SEPARATOR_STR}w{MAIN_SEPARATOR_STR}M\u{2026}e.rs")
        );
    }

    #[test]
    fn no_directories() {
        let p = make(&[], Some("README.md"));
        assert_eq!(p.to_width(20), "README.md");
    }

    #[test]
    fn no_directories_with_ellipsis() {
        let p = make(&[], Some("README.md"));
        assert_eq!(p.to_width(7), "RE\u{2026}E.md");
    }

    #[test]
    fn no_extension() {
        let p = make(&["src"], Some("Makefile"));
        assert_eq!(p.to_width(11), format!("sr{MAIN_SEPARATOR_STR}Makefile"));
    }

    #[test]
    fn no_extension_with_ellipsis() {
        let p = make(&["src"], Some("Makefile"));
        assert_eq!(p.to_width(7), format!("s{MAIN_SEPARATOR_STR}Ma\u{2026}le"));
    }

    #[test]
    fn short_stem_skips_ellipsis() {
        let p = make(&["src"], Some("a.rs"));
        assert_eq!(p.to_width(4), "a.rs");
    }

    #[test]
    fn short_dirs_not_over_abbreviated() {
        let p = make(&["a", "b"], Some("MyFile.rs"));
        assert_eq!(
            p.to_width(20),
            format!("a{MAIN_SEPARATOR_STR}b{MAIN_SEPARATOR_STR}MyFile.rs")
        );
    }

    #[test]
    fn very_narrow_truncation() {
        let p = make(&["src", "components"], Some("MyFile.rs"));
        assert_eq!(p.to_width(2), "rs");
    }

    #[test]
    fn empty_returns_empty() {
        let p = make(&[], None);
        assert_eq!(p.to_width(20), "");
    }

    #[test]
    fn dotfile_treated_as_no_ext() {
        let p = make(&[], Some(".gitignore"));
        assert_eq!(p.to_width(5), ".g\u{2026}re");
    }
}
