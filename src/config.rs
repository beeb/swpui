use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use etcetera::{AppStrategy as _, AppStrategyArgs, choose_app_strategy};
use serde::Deserialize;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    pub match_mode: MatchMode,
    pub include_hidden: bool,
    pub include_gitignored: bool,
}

impl Options {
    fn merge(&mut self, other: &OptionsSection) {
        if let Some(v) = other.match_mode {
            self.match_mode = v;
        }
        if let Some(v) = other.include_hidden {
            self.include_hidden = v;
        }
        if let Some(v) = other.include_gitignored {
            self.include_gitignored = v;
        }
    }
}

impl Default for Options {
    fn default() -> Self {
        Self {
            match_mode: MatchMode::default(),
            include_hidden: true,
            include_gitignored: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchMode {
    #[default]
    CaseAware,
    Literal,
    Regex,
    RegexMultiline,
}

impl MatchMode {
    #[must_use]
    pub fn toggle(self) -> Self {
        match self {
            Self::CaseAware => Self::Literal,
            Self::Literal => Self::Regex,
            Self::Regex => Self::RegexMultiline,
            Self::RegexMultiline => Self::CaseAware,
        }
    }

    #[must_use]
    pub fn is_regex(&self) -> bool {
        match self {
            MatchMode::CaseAware | MatchMode::Literal => false,
            MatchMode::Regex | MatchMode::RegexMultiline => true,
        }
    }
}

impl fmt::Display for MatchMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let disp = match self {
            MatchMode::CaseAware => "case-aware",
            MatchMode::Literal => "literal",
            MatchMode::Regex => "regex",
            MatchMode::RegexMultiline => "regex multiline",
        };
        f.write_str(disp)
    }
}

/// Result of parsing config files. If errors were encountered, a human-readable summary is provided.
#[derive(Debug)]
pub struct ConfigResult {
    pub options: Options,
    pub warning: Option<String>,
}

/// Walker for the config files which iteratively merges them into a config object.
#[derive(Debug, Default)]
pub struct Loader {
    options: Options,
    warnings: Vec<String>,
}

impl Loader {
    /// Load the config from files in various locations (workspace and parents, user config).
    #[must_use]
    pub fn load(root: impl AsRef<Path>) -> ConfigResult {
        let root = root.as_ref();
        let user_dir = Self::user_config_dir();
        Self::load_with(root, user_dir.as_deref())
    }

    /// Load the config from files in various locations, while specifying the user directory instead of resolving.
    fn load_with(root: &Path, user_dir: Option<&Path>) -> ConfigResult {
        Self::default()
            .merge_dir(user_dir)
            .merge_workspace(root)
            .into()
    }

    /// Load the config from a user dir, if any.
    fn merge_dir(self, dir: Option<&Path>) -> Self {
        let Some(dir) = dir else { return self };
        let Some(path) = Self::file_in_dir(dir) else {
            return self;
        };
        self.merge_file(&path)
    }

    /// Load the config from the workspace, walking down from the root to the current dir, merging as we descend.
    fn merge_workspace(mut self, root: &Path) -> Self {
        let mut dirs: Vec<&Path> = root.ancestors().collect();
        dirs.reverse();
        for dir in dirs {
            if let Some(path) = Self::file_in_dir(dir) {
                self = self.merge_file(&path);
            }
        }
        self
    }

    /// Merge the config from a file `path` into the current config.
    fn merge_file(mut self, path: &Path) -> Self {
        let text = match fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return self,
            Err(e) => {
                warn!(error = ?e, path = %path.display(), "error while reading config file");
                let msg = format!("{}: {e}", path.display());
                self.warnings.push(msg);
                return self;
            }
        };
        match toml::from_str::<ConfigFile>(&text) {
            Ok(cfg) => self.merge_config(&cfg),
            Err(e) => {
                warn!(error = ?e, path = %path.display(), "error while parsing config file");
                let msg = format!("{}: {}", path.display(), e.message());
                self.warnings.push(msg);
            }
        }
        self
    }

    /// Merge a config file's content with the current config.
    ///
    /// Fields defined in `other` take precedence over previously defined values.
    fn merge_config(&mut self, other: &ConfigFile) {
        self.options.merge(&other.options);
    }

    /// Determine the path to the config dir.
    ///
    /// `~/.config/swpui` on Linux and macOS, and `~\AppData\Roaming\beeb\swpui` on Windows.
    fn user_config_dir() -> Option<PathBuf> {
        let strategy = choose_app_strategy(AppStrategyArgs {
            top_level_domain: "li".to_string(),
            author: "beeb".to_string(),
            app_name: "swpui".to_string(),
        })
        .ok()?;
        Some(strategy.config_dir())
    }

    /// Check for the existence of a config file in the dir.
    ///
    /// Both `swpui.toml` and `.swpui.toml` are supported. This function gives precedence to the filename without a
    /// leading dot.
    fn file_in_dir(dir: impl AsRef<Path>) -> Option<PathBuf> {
        let dir = dir.as_ref();
        let primary = dir.join("swpui.toml");
        if primary.is_file() {
            return Some(primary);
        }
        let hidden = dir.join(".swpui.toml");
        if hidden.is_file() {
            return Some(hidden);
        }
        None
    }
}

/// Finalize the config loading process by merging warnings into a human-readable string.
impl From<Loader> for ConfigResult {
    fn from(loader: Loader) -> Self {
        let mut warnings = loader.warnings.into_iter();
        let warning = match (warnings.next(), warnings.count()) {
            (None, _) => None,
            (Some(first), 0) => Some(first),
            (Some(first), n) => Some(format!("{first} (+{n} more, see log)")),
        };
        Self {
            options: loader.options,
            warning,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ConfigFile {
    #[serde(default)]
    options: OptionsSection,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct OptionsSection {
    match_mode: Option<MatchMode>,
    include_hidden: Option<bool>,
    include_gitignored: Option<bool>,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    fn write(dir: &TempDir, name: &str, body: &str) -> PathBuf {
        let path = dir.path().join(name);
        fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn match_mode_toggle() {
        let mut mode = MatchMode::CaseAware;
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::Literal);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::Regex);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::RegexMultiline);
        mode = mode.toggle();
        assert_eq!(mode, MatchMode::CaseAware);
    }

    #[test]
    fn match_mode_variants() {
        #[derive(Deserialize)]
        struct Wrap {
            v: MatchMode,
        }
        let cases = [
            ("v = \"case-aware\"", MatchMode::CaseAware),
            ("v = \"literal\"", MatchMode::Literal),
            ("v = \"regex\"", MatchMode::Regex),
            ("v = \"regex-multiline\"", MatchMode::RegexMultiline),
        ];
        for (input, expected) in cases {
            let got: Wrap = toml::from_str(input).unwrap();
            assert_eq!(got.v, expected);
        }
    }

    #[test]
    fn parses_full() {
        let dir = TempDir::new().unwrap();
        let path = write(
            &dir,
            "swpui.toml",
            "[options]\nmatch-mode = \"regex\"\ninclude-hidden = false\ninclude-gitignored = true\n",
        );
        let loader = Loader::default().merge_file(&path);
        assert!(loader.warnings.is_empty());
        assert_eq!(loader.options.match_mode, MatchMode::Regex);
        assert!(!loader.options.include_hidden);
        assert!(loader.options.include_gitignored);
    }

    #[test]
    fn partial_merge() {
        let mut opts = Options {
            match_mode: MatchMode::CaseAware,
            include_hidden: false,
            include_gitignored: true,
        };
        let cfg: ConfigFile = toml::from_str("[options]\nmatch-mode = \"literal\"\n").unwrap();
        opts.merge(&cfg.options);
        assert_eq!(opts.match_mode, MatchMode::Literal);
        assert!(!opts.include_hidden);
        assert!(opts.include_gitignored);
    }

    #[test]
    fn unknown_key() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "swpui.toml", "[options]\nmatch-modee = \"regex\"\n");
        let loader = Loader::default().merge_file(&path);
        assert_eq!(loader.warnings.len(), 1);
        assert_eq!(loader.options, Options::default());
    }

    #[test]
    fn wrong_type() {
        let dir = TempDir::new().unwrap();
        let path = write(&dir, "swpui.toml", "[options]\ninclude-hidden = \"yes\"\n");
        let loader = Loader::default().merge_file(&path);
        assert_eq!(loader.warnings.len(), 1);
        assert_eq!(loader.options, Options::default());
    }

    #[test]
    fn missing_file_silent() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.toml");
        let loader = Loader::default().merge_file(&path);
        assert!(loader.warnings.is_empty());
        assert_eq!(loader.options, Options::default());
    }

    #[test]
    fn dotted_vs_non_dotted() {
        let dir = TempDir::new().unwrap();
        write(&dir, "swpui.toml", "[options]\nmatch-mode = \"regex\"\n");
        write(&dir, ".swpui.toml", "[options]\nmatch-mode = \"literal\"\n");
        let picked = Loader::file_in_dir(dir.path()).unwrap();
        assert_eq!(picked.file_name().unwrap(), "swpui.toml");
    }

    #[test]
    fn dotted_only() {
        let dir = TempDir::new().unwrap();
        write(&dir, ".swpui.toml", "[options]\nmatch-mode = \"literal\"\n");
        let picked = Loader::file_in_dir(dir.path()).unwrap();
        assert_eq!(picked.file_name().unwrap(), ".swpui.toml");
    }

    #[test]
    fn no_config_in_dir() {
        let dir = TempDir::new().unwrap();
        assert!(Loader::file_in_dir(dir.path()).is_none());
    }

    #[test]
    fn project_closest_wins() {
        let dir = TempDir::new().unwrap();
        let inner = dir.path().join("inner");
        fs::create_dir_all(&inner).unwrap();
        fs::write(
            dir.path().join("swpui.toml"),
            "[options]\nmatch-mode = \"regex\"\n",
        )
        .unwrap();
        fs::write(
            inner.join("swpui.toml"),
            "[options]\nmatch-mode = \"literal\"\n",
        )
        .unwrap();

        let result = Loader::load_with(&inner, None);
        assert!(result.warning.is_none());
        assert_eq!(result.options.match_mode, MatchMode::Literal);
    }

    #[test]
    fn user_layer_lowest() {
        let user_dir = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        fs::write(
            user_dir.path().join("swpui.toml"),
            "[options]\nmatch-mode = \"regex\"\ninclude-hidden = false\n",
        )
        .unwrap();
        fs::write(
            project.path().join("swpui.toml"),
            "[options]\nmatch-mode = \"literal\"\n",
        )
        .unwrap();

        let result = Loader::load_with(project.path(), Some(user_dir.path()));
        assert!(result.warning.is_none());
        assert_eq!(result.options.match_mode, MatchMode::Literal);
        assert!(!result.options.include_hidden);
    }

    #[test]
    fn warning_summary() {
        let user_dir = TempDir::new().unwrap();
        let project = TempDir::new().unwrap();
        fs::write(user_dir.path().join("swpui.toml"), "[options]\nbogus = 1\n").unwrap();
        fs::write(project.path().join("swpui.toml"), "[options]\nbogus = 2\n").unwrap();

        let result = Loader::load_with(project.path(), Some(user_dir.path()));
        let msg = result.warning.unwrap();
        assert!(msg.contains("+1 more"), "got: {msg}");
    }
}
