//! Cloning, pushing, fetching, and pulling intent between repositories. Sync is
//! git-shaped: move the objects the other side is missing, then move the ref.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{anyhow, bail, Result};

use crate::config::{Config, Identity, Remote};
use crate::object::Object;
use crate::repo::{Merge, Repo};
use crate::transport::{self, Transport};

/// What a push did.
#[derive(Debug, PartialEq, Eq)]
pub enum Push {
    UpToDate,
    Pushed { objects: usize },
}

fn token() -> Option<String> {
    std::env::var("LORE_TOKEN").ok()
}

fn transport_for(repo: &Repo, remote: &str) -> Result<Box<dyn Transport>> {
    let url = repo
        .config()?
        .remotes
        .get(remote)
        .ok_or_else(|| anyhow!("no such remote: {remote}"))?
        .url
        .clone();
    transport::open(&url, token())
}

/// Walk a commit's reachable object graph, fetching anything not held locally.
fn fetch_graph(repo: &Repo, transport: &dyn Transport, tip: &str) -> Result<()> {
    let mut stack = vec![tip.to_string()];
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let bytes = if repo.has_object(&id) {
            repo.object_bytes(&id)?
        } else {
            let bytes = transport.get_object(&id)?;
            repo.write_object_bytes(&id, &bytes)?;
            bytes
        };
        if let Object::Commit(c) = serde_json::from_slice(&bytes)? {
            stack.extend(c.parents);
            stack.extend(c.entries);
        }
    }
    Ok(())
}

/// Fetch every remote branch's objects and update tracking refs. Returns the
/// remote's advertised refs.
fn fetch_into(
    repo: &Repo,
    remote: &str,
    transport: &dyn Transport,
) -> Result<BTreeMap<String, String>> {
    let refs = transport.list_refs()?;
    for (branch, id) in &refs {
        fetch_graph(repo, transport, id)?;
        repo.write_remote_ref(remote, branch, id)?;
    }
    Ok(refs)
}

/// Fetch from a configured remote into tracking refs (no local branch moves).
pub fn fetch(repo: &Repo, remote: &str) -> Result<BTreeMap<String, String>> {
    let transport = transport_for(repo, remote)?;
    fetch_into(repo, remote, transport.as_ref())
}

/// Clone the remote at `url` into a fresh repository at `dir`.
pub fn clone(url: &str, dir: &Path, token: Option<String>) -> Result<Repo> {
    let repo = Repo::scaffold(dir)?;
    let transport = transport::open(url, token)?;
    let refs = fetch_into(&repo, "origin", transport.as_ref())?;

    let mut config = Config::default();
    config
        .remotes
        .insert("origin".into(), Remote { url: url.into() });
    repo.save_config(&config)?;

    // Adopt a default local branch: main if present, else any tracked branch.
    let default = if refs.contains_key("main") {
        Some("main".to_string())
    } else {
        refs.keys().next().cloned()
    };
    if let Some(branch) = default {
        if let Some(id) = repo.read_remote_ref("origin", &branch)? {
            repo.write_ref(&branch, &id)?;
            repo.set_head(&branch)?;
        }
    }
    Ok(repo)
}

/// Push a local branch to a configured remote. Fast-forward only.
pub fn push(repo: &Repo, remote: &str, branch: &str) -> Result<Push> {
    let transport = transport_for(repo, remote)?;
    let head = repo
        .read_ref(branch)?
        .ok_or_else(|| anyhow!("branch '{branch}' has no commits to push"))?;

    // The remote's branch tip tells us what it already has, so we send only the
    // new objects -- no per-object round-trip to check existence.
    let base = match transport.list_refs()?.get(branch) {
        Some(r) if r == &head => return Ok(Push::UpToDate),
        Some(r) if !repo.reachable(&head)?.contains(r) => {
            bail!("non-fast-forward push to {remote}/{branch}; fetch and merge first")
        }
        Some(r) => Some(r.clone()),
        None => None,
    };

    let head_objects = objects_reachable(repo, &head)?;
    let already_there = match &base {
        Some(b) => objects_reachable(repo, b)?,
        None => HashSet::new(),
    };
    let mut sent = 0;
    for id in head_objects.difference(&already_there) {
        transport.put_object(id, &repo.object_bytes(id)?)?;
        sent += 1;
    }
    transport.set_ref(branch, &head)?;
    repo.write_remote_ref(remote, branch, &head)?;
    Ok(Push::Pushed { objects: sent })
}

/// Every object id (commits and the entries they record) reachable from `tip`.
fn objects_reachable(repo: &Repo, tip: &str) -> Result<HashSet<String>> {
    let mut objects = HashSet::new();
    for cid in repo.reachable(tip)? {
        for eid in repo.read_commit(&cid)?.entries {
            objects.insert(eid);
        }
        objects.insert(cid);
    }
    Ok(objects)
}

/// Fetch and merge a remote's matching branch into the current branch.
pub fn pull(repo: &Repo, remote: &str, who: &Identity, now: u64) -> Result<Merge> {
    fetch(repo, remote)?;
    let branch = repo.current_branch()?;
    let tracked = repo
        .read_remote_ref(remote, &branch)?
        .ok_or_else(|| anyhow!("remote '{remote}' has no branch '{branch}'"))?;
    repo.merge_commit(&tracked, who, &format!("Merge {remote}/{branch}"), now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn who(name: &str) -> Identity {
        Identity::new(name, format!("{name}@test"))
    }

    fn with_origin(repo: &Repo, url: &str) {
        let mut c = repo.config().unwrap();
        c.remotes
            .insert("origin".into(), Remote { url: url.into() });
        repo.save_config(&c).unwrap();
    }

    #[test]
    fn push_then_clone_round_trips() {
        let srv = TempDir::new().unwrap();
        let url = srv.path().display().to_string();
        let a_dir = TempDir::new().unwrap();
        let a = Repo::init(a_dir.path()).unwrap();
        a.add(&who("ray"), "use sqlite", 1).unwrap();
        a.commit(&who("ray"), "c1", 10).unwrap();
        a.add(&who("ray"), "links expire", 2).unwrap();
        a.commit(&who("ray"), "c2", 20).unwrap();
        with_origin(&a, &url);

        assert!(matches!(
            push(&a, "origin", "main").unwrap(),
            Push::Pushed { .. }
        ));
        assert_eq!(push(&a, "origin", "main").unwrap(), Push::UpToDate);

        let c_dir = TempDir::new().unwrap();
        let c = clone(&url, &c_dir.path().join("c"), None).unwrap();
        assert_eq!(
            a.materialize("HEAD", 100).unwrap(),
            c.materialize("HEAD", 100).unwrap()
        );
        assert!(c
            .materialize("HEAD", 100)
            .unwrap()
            .contains("ray <ray@test>"));
        assert!(c.config().unwrap().remotes.contains_key("origin"));
    }

    #[test]
    fn incremental_push_and_fetch() {
        let srv = TempDir::new().unwrap();
        let url = srv.path().display().to_string();
        let a_dir = TempDir::new().unwrap();
        let a = Repo::init(a_dir.path()).unwrap();
        a.add(&who("ray"), "one", 1).unwrap();
        a.commit(&who("ray"), "c1", 10).unwrap();
        with_origin(&a, &url);
        push(&a, "origin", "main").unwrap();

        let c_dir = TempDir::new().unwrap();
        let c = clone(&url, &c_dir.path().join("c"), None).unwrap();

        a.add(&who("ray"), "two", 2).unwrap();
        let a2 = a.commit(&who("ray"), "c2", 20).unwrap();
        // only the new commit and its entry travel, not the whole history
        assert_eq!(
            push(&a, "origin", "main").unwrap(),
            Push::Pushed { objects: 2 }
        );

        let refs = fetch(&c, "origin").unwrap();
        assert_eq!(refs.get("main"), Some(&a2));
        assert_eq!(c.read_remote_ref("origin", "main").unwrap(), Some(a2));
        assert!(c.materialize("origin/main", 100).unwrap().contains("two"));
    }

    #[test]
    fn non_fast_forward_push_is_rejected() {
        let srv = TempDir::new().unwrap();
        let url = srv.path().display().to_string();
        let a_dir = TempDir::new().unwrap();
        let a = Repo::init(a_dir.path()).unwrap();
        a.add(&who("ray"), "base", 1).unwrap();
        a.commit(&who("ray"), "base", 10).unwrap();
        with_origin(&a, &url);
        push(&a, "origin", "main").unwrap();

        let c_dir = TempDir::new().unwrap();
        let c = clone(&url, &c_dir.path().join("c"), None).unwrap();
        c.add(&who("cara"), "from c", 2).unwrap();
        c.commit(&who("cara"), "c work", 20).unwrap();
        push(&c, "origin", "main").unwrap();

        a.add(&who("ray"), "from a", 3).unwrap();
        a.commit(&who("ray"), "a work", 30).unwrap();
        let err = push(&a, "origin", "main").unwrap_err().to_string();
        assert!(err.contains("non-fast-forward"), "got: {err}");
    }

    #[test]
    fn pull_fast_forwards_to_remote() {
        let srv = TempDir::new().unwrap();
        let url = srv.path().display().to_string();
        let a_dir = TempDir::new().unwrap();
        let a = Repo::init(a_dir.path()).unwrap();
        a.add(&who("ray"), "base", 1).unwrap();
        a.commit(&who("ray"), "base", 10).unwrap();
        with_origin(&a, &url);
        push(&a, "origin", "main").unwrap();

        let c_dir = TempDir::new().unwrap();
        let c = clone(&url, &c_dir.path().join("c"), None).unwrap();

        a.add(&who("ray"), "more", 2).unwrap();
        let a2 = a.commit(&who("ray"), "c2", 20).unwrap();
        push(&a, "origin", "main").unwrap();

        assert_eq!(
            pull(&c, "origin", &who("cara"), 30).unwrap(),
            Merge::FastForward(a2.clone())
        );
        assert_eq!(c.head_commit().unwrap(), Some(a2));
        assert!(c.materialize("HEAD", 100).unwrap().contains("more"));
    }

    #[test]
    fn push_to_unknown_remote_errors() {
        let a_dir = TempDir::new().unwrap();
        let a = Repo::init(a_dir.path()).unwrap();
        a.add(&who("ray"), "x", 1).unwrap();
        a.commit(&who("ray"), "c", 10).unwrap();
        assert!(push(&a, "origin", "main").is_err());
    }
}
