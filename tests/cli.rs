//! End-to-end tests that drive the compiled `lore` binary in a temp directory.

use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn lore(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_lore"))
        .args(args)
        .current_dir(dir)
        .env("LORE_AUTHOR", "tester")
        .env("LORE_EMAIL", "") // let config supply the email, deterministically
        .env("LORE_TOKEN", "")
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

    ok(p, &["add", "use sqlite for storage"]);
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
fn show_prints_full_message_log_abbreviates() {
    let dir = TempDir::new().unwrap();
    let p = dir.path();
    ok(p, &["init"]);
    ok(p, &["add", "use sqlite for storage"]);
    let long = "a very long commit message that the log view abbreviates but show keeps in full so nothing is lost";
    ok(p, &["commit", "-m", long]);

    let log = ok(p, &["log"]);
    assert!(log.contains("..."), "log should abbreviate: {log}");
    assert!(!log.contains("nothing is lost"), "log should drop the tail");

    let show = ok(p, &["show"]);
    assert!(show.starts_with("commit "));
    assert!(show.contains(long), "show should keep the full message");
    assert!(
        show.contains("use sqlite for storage"),
        "show should list the intent"
    );
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

/// Everything but the volatile "Generated:" line, for comparing two briefs.
fn intent_body(brief: &str) -> String {
    brief
        .lines()
        .filter(|l| !l.starts_with("Generated:"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn remote_push_and_clone_round_trip() {
    let w = TempDir::new().unwrap();
    let a = w.path().join("a");
    let srv = w.path().join("srv");
    std::fs::create_dir_all(&a).unwrap();

    ok(&a, &["init"]);
    ok(&a, &["config", "user.email", "tester@lore.test"]);
    ok(&a, &["add", "use sqlite for storage"]);
    ok(&a, &["commit", "-m", "initial intent"]);

    // identity = name from env, email from config
    assert!(ok(&a, &["log"]).contains("tester <tester@lore.test>"));

    // wire up a local-path remote and push to it
    ok(&a, &["remote", "add", "origin", srv.to_str().unwrap()]);
    assert!(ok(&a, &["remote"]).contains("origin"));
    assert!(ok(&a, &["push"]).contains("pushed main to origin"));

    // clone it into a sibling directory
    ok(w.path(), &["clone", srv.to_str().unwrap(), "b"]);
    let b = w.path().join("b");
    assert!(b.join(".lore").is_dir());

    // the clone reproduces the same intent and the same authorship
    assert_eq!(
        intent_body(&ok(&a, &["materialize"])),
        intent_body(&ok(&b, &["materialize"]))
    );
    assert!(ok(&b, &["materialize"]).contains("use sqlite for storage"));
    assert!(ok(&b, &["log"]).contains("tester <tester@lore.test>"));
}
