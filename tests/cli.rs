//! End-to-end tests that drive the compiled `lore` binary in a temp directory.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn lore(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lore"))
        .args(args)
        .current_dir(dir)
        .env("LORE_AUTHOR", "tester")
        .output()
        .expect("run lore")
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = lore(dir, args);
    assert!(
        out.status.success(),
        "command {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn full_intent_lifecycle() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();

    ok(p, &["init"]);
    assert!(p.join(".lore").is_dir());
    assert!(p.join("AGENTS.md").is_file());

    ok(p, &["add", "use sqlite for storage", "-k", "decision"]);
    ok(p, &["add", "the cli should mirror git"]);

    let status = ok(p, &["status"]);
    assert!(status.contains("no commits yet"));
    assert!(status.contains("use sqlite for storage"));

    ok(p, &["commit", "-m", "initial intent"]);
    assert!(ok(p, &["log"]).contains("initial intent"));

    let brief = ok(p, &["materialize"]);
    assert!(brief.contains("use sqlite for storage"));
    assert!(brief.contains("the cli should mirror git"));
    assert!(brief.contains("# Lore Materialization"));
}

#[test]
fn branch_merge_and_materialize_union() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();

    ok(p, &["init"]);
    ok(p, &["add", "base intent"]);
    ok(p, &["commit", "-m", "base"]);

    ok(p, &["checkout", "-b", "feature"]);
    ok(p, &["add", "feature intent"]);
    ok(p, &["commit", "-m", "feature work"]);

    ok(p, &["checkout", "main"]);
    ok(p, &["add", "main intent"]);
    ok(p, &["commit", "-m", "main work"]);

    assert!(ok(p, &["merge", "feature"]).contains("merged into"));

    let brief = ok(p, &["materialize"]);
    assert!(brief.contains("base intent"));
    assert!(brief.contains("feature intent"));
    assert!(brief.contains("main intent"));
}

#[test]
fn materialize_to_file() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    ok(p, &["init"]);
    ok(p, &["add", "remember this"]);
    ok(p, &["commit", "-m", "c"]);
    ok(p, &["materialize", "-o", "brief.md"]);
    let written = std::fs::read_to_string(p.join("brief.md")).unwrap();
    assert!(written.contains("remember this"));
}

#[test]
fn commands_fail_outside_a_repo() {
    let dir = TempDir::new().unwrap();
    let out = lore(dir.path(), &["status"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("not a Lore repository"));
}

#[test]
fn empty_commit_fails() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    ok(p, &["init"]);
    let out = lore(p, &["commit", "-m", "nothing"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("nothing staged"));
}
