#![expect(clippy::unwrap_used)]

use std::{fs, io::Write as _, sync::atomic::AtomicUsize};

use swpui::{
    hash::FileHash,
    replace::{apply_replacements, effective_replacement, write_file},
    search::{Pattern, find_matches_in_content},
    types::MatchMode,
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
