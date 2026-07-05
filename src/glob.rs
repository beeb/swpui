use std::{fmt, path::Path};

use ignore::overrides::{Override, OverrideBuilder};

/// Include/exclude glob patterns restricting the searched file set.
///
/// Patterns use ripgrep `-g` / gitignore matching semantics. When any include
/// pattern is set, only matching files are searched. Exclude patterns are
/// applied second so they remove files matched by the include rules.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlobFilters {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Which input field an invalid glob came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlobErrorOrigin {
    Include,
    Exclude,
    Build,
}

/// An invalid glob pattern, attributed to the input field it came from.
#[derive(Debug)]
pub struct GlobError {
    pub origin: GlobErrorOrigin,
    pub source: ignore::Error,
}

impl std::error::Error for GlobError {}

impl fmt::Display for GlobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl GlobFilters {
    /// Parse the two comma-separated input strings into pattern lists.
    #[must_use]
    pub fn parse(include: &str, exclude: &str) -> Self {
        Self {
            include: Self::split_globs(include),
            exclude: Self::split_globs(exclude),
        }
    }

    /// Validate patterns and build the walker overrides.
    pub fn overrides(&self, root: impl AsRef<Path>) -> Result<Override, GlobError> {
        let mut builder = OverrideBuilder::new(root);
        for pat in &self.include {
            builder.add(pat).map_err(|source| GlobError {
                origin: GlobErrorOrigin::Include,
                source,
            })?;
        }
        for pat in &self.exclude {
            builder
                .add(&format!("!{pat}"))
                .map_err(|source| GlobError {
                    origin: GlobErrorOrigin::Exclude,
                    source,
                })?;
        }
        builder.build().map_err(|source| GlobError {
            origin: GlobErrorOrigin::Build,
            source,
        })
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.include.is_empty() && self.exclude.is_empty()
    }

    /// Split a comma-separated glob list, ignoring commas inside braces.
    fn split_globs(input: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut start = 0;
        for (idx, c) in input.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => depth = depth.saturating_sub(1),
                ',' if depth == 0 => {
                    parts.push(&input[start..idx]);
                    start = idx + 1;
                }
                _ => {}
            }
        }
        parts.push(&input[start..]);
        parts
            .into_iter()
            .filter_map(|s| {
                let s = s.trim();
                (!s.is_empty()).then(|| s.to_string())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn pats(v: &[&str]) -> Vec<String> {
        v.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parse_plain() {
        let f = GlobFilters::parse("src/**, *.rs", "*_test.rs");
        assert_eq!(f.include, pats(&["src/**", "*.rs"]));
        assert_eq!(f.exclude, pats(&["*_test.rs"]));
    }

    #[test]
    fn parse_brace_aware() {
        let f = GlobFilters::parse("*.{rs,toml}, docs/**", "");
        assert_eq!(f.include, pats(&["*.{rs,toml}", "docs/**"]));
        assert!(f.exclude.is_empty());
    }

    #[test]
    fn parse_nested_braces() {
        let f = GlobFilters::parse("{a,{b,c}}, d", "");
        assert_eq!(f.include, pats(&["{a,{b,c}}", "d"]));
    }

    #[test]
    fn parse_messy_separators() {
        let f = GlobFilters::parse(" a ,, b, ", "");
        assert_eq!(f.include, pats(&["a", "b"]));
    }

    #[test]
    fn parse_empty() {
        let f = GlobFilters::parse("", "  ");
        assert!(f.is_empty());
    }

    #[test]
    fn invalid_include() {
        let f = GlobFilters::parse("foo[", "");
        let err = f.overrides(Path::new(".")).unwrap_err();
        assert_eq!(err.origin, GlobErrorOrigin::Include);
    }

    #[test]
    fn invalid_exclude() {
        let f = GlobFilters::parse("*.rs", "foo[");
        let err = f.overrides(Path::new(".")).unwrap_err();
        assert_eq!(err.origin, GlobErrorOrigin::Exclude);
    }

    #[test]
    fn valid_overrides() {
        let f = GlobFilters::parse("*.{rs,toml}", "target/**");
        assert!(f.overrides(Path::new(".")).is_ok());
    }
}
