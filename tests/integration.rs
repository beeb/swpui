#![expect(clippy::unwrap_used)]

use std::{
    fs,
    io::Write as _,
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use swpui::{
    app::App,
    config::{ConfigResult, MatchMode, Options},
    glob::GlobFilters,
    hash::FileHash,
    replace::{apply_replacements, effective_replacement, write_file},
    search::{Pattern, find_matches_in_content},
    types::Pane,
};

fn create_test_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    for (name, content) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
    }
    dir
}

#[test]
fn full_search_and_replace_workflow() {
    let dir = create_test_dir(&[("hello.txt", "hello world\nhello rust\n")]);
    let path = dir.path().join("hello.txt");
    let content = fs::read_to_string(&path).unwrap();

    let matches = find_matches_in_content(
        &content,
        &Pattern::new("hello", MatchMode::Literal).unwrap(),
        &AtomicUsize::new(0),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(matches.len(), 2);

    let new_content = apply_replacements(content, &matches, "hi", MatchMode::Literal);
    assert_eq!(new_content, "hi world\nhi rust\n");

    write_file(&path, &new_content).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "hi world\nhi rust\n");
}

#[test]
fn stale_file_prevents_write() {
    let dir = create_test_dir(&[("test.txt", "original content\n")]);
    let path = dir.path().join("test.txt");
    let hash = FileHash::new(&path).unwrap();

    // Modify externally
    fs::write(&path, "someone else changed this\n").unwrap();

    assert!(!hash.matches(&path).unwrap());
}

#[test]
fn regex_search_and_replace() {
    let content = "foo123 bar456 foo789\n";
    let matches = find_matches_in_content(
        content,
        &Pattern::new(r"foo\d+", MatchMode::Regex).unwrap(),
        &AtomicUsize::new(0),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(matches.len(), 2);
    let new_content = apply_replacements(content, &matches, "replaced", MatchMode::Regex);
    assert_eq!(new_content, "replaced bar456 replaced\n");
}

#[test]
fn skip_preserves_matches() {
    let content = "aaa bbb aaa\n";
    let mut matches = find_matches_in_content(
        content,
        &Pattern::new("aaa", MatchMode::Literal).unwrap(),
        &AtomicUsize::new(0),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(matches.len(), 2);

    // skip the first match
    matches[0].skip = true;
    let new_content = apply_replacements(content, &matches, "ccc", MatchMode::Literal);
    assert_eq!(new_content, "aaa bbb ccc\n");
}

#[test]
fn multiline_search_and_replace_workflow() {
    let dir = create_test_dir(&[("multi.txt", "hello\nfoo bar\nbaz qux\nend\n")]);
    let path = dir.path().join("multi.txt");
    let content = fs::read_to_string(&path).unwrap();

    let matches = find_matches_in_content(
        &content,
        &Pattern::new(r"bar\nbaz", MatchMode::RegexMultiline).unwrap(),
        &AtomicUsize::new(0),
        usize::MAX,
    )
    .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(&content[matches[0].byte_range.as_range()], "bar\nbaz");

    let replacement = effective_replacement(r"BAR\nBAZ", MatchMode::RegexMultiline);
    let new_content =
        apply_replacements(content, &matches, &replacement, MatchMode::RegexMultiline);
    assert_eq!(new_content, "hello\nfoo BAR\nBAZ qux\nend\n");

    write_file(&path, &new_content).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "hello\nfoo BAR\nBAZ qux\nend\n"
    );
}

fn test_app(dir: &tempfile::TempDir) -> App {
    let config = ConfigResult {
        options: Options::default(),
        warning: None,
    };
    App::new(dir.path().to_path_buf(), config).unwrap()
}

/// Backdate the last keystroke so the debounce window has elapsed.
fn force_debounce(app: &mut App) {
    app.last_keystroke = Instant::now().checked_sub(Duration::from_millis(200));
    app.debounce_search();
}

#[test]
fn glob_edit_updates_options() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    app.include_input.set_text("*.rs, src/**");
    app.exclude_input.set_text("*_test.rs");
    app.schedule_rebuild();
    force_debounce(&mut app);
    assert_eq!(
        app.options.globs.include,
        vec!["*.rs".to_string(), "src/**".to_string()]
    );
    assert_eq!(app.options.globs.exclude, vec!["*_test.rs".to_string()]);
    assert!(!app.pending_rebuild);
}

#[test]
fn invalid_glob_marks_input() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    app.exclude_input.set_text("foo[");
    app.schedule_rebuild();
    force_debounce(&mut app);
    assert!(app.exclude_input.invalid);
    assert!(app.status_message.is_some());
    assert!(app.options.globs.is_empty());
}

#[test]
fn startup_invalid_config_glob() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let options = Options {
        globs: GlobFilters::parse("foo[", ""),
        ..Options::default()
    };
    let app = App::new(
        dir.path().to_path_buf(),
        ConfigResult {
            options,
            warning: None,
        },
    )
    .unwrap();
    assert!(app.include_input.invalid);
    assert!(app.status_message.is_some());
    assert_eq!(app.include_input.text(), "foo[");
}

#[test]
fn startup_populates_glob_inputs() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let options = Options {
        globs: GlobFilters::parse("src/**, *.rs", "*_test.rs"),
        ..Options::default()
    };
    let app = App::new(
        dir.path().to_path_buf(),
        ConfigResult {
            options,
            warning: None,
        },
    )
    .unwrap();
    assert_eq!(app.include_input.text(), "src/**, *.rs");
    assert_eq!(app.exclude_input.text(), "*_test.rs");
    assert!(!app.include_input.invalid);
}

fn key(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, mods)
}

#[test]
fn glob_toggle_view_focus() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    assert!(!app.filter_view);
    assert_eq!(app.focused_pane, Pane::SearchInput);

    app.handle_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(app.filter_view);
    assert_eq!(app.focused_pane, Pane::IncludeInput);

    app.handle_key(key(KeyCode::Char('g'), KeyModifiers::ALT));
    assert!(!app.filter_view);
    assert_eq!(app.focused_pane, Pane::SearchInput);
}

#[test]
fn glob_toggle_noninput_focus() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    app.focused_pane = Pane::FileList;
    app.handle_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert!(app.filter_view);
    assert_eq!(app.focused_pane, Pane::FileList);
}

#[test]
fn switch_to_hidden_input() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    app.focused_pane = Pane::FileList;

    app.handle_key(key(KeyCode::Char('5'), KeyModifiers::NONE));
    assert!(app.filter_view);
    assert_eq!(app.focused_pane, Pane::IncludeInput);

    app.focused_pane = Pane::FileList;
    app.handle_key(key(KeyCode::Char('2'), KeyModifiers::NONE));
    assert!(!app.filter_view);
    assert_eq!(app.focused_pane, Pane::ReplaceInput);
}

#[test]
fn glob_typing_rebuild() {
    let dir = create_test_dir(&[("a.txt", "x\n")]);
    let mut app = test_app(&dir);
    app.handle_key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    app.handle_key(key(KeyCode::Char('*'), KeyModifiers::NONE));
    assert_eq!(app.include_input.text(), "*");
    assert!(app.pending_rebuild);
}
