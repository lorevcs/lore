//! The repository: an append-only object store plus refs, modeled on git but
//! holding intent instead of code.
//!
//! Layout under `.lore/`:
//! ```text
//! HEAD                    "ref: refs/heads/<branch>"
//! config                  JSON: local identity and named remotes
//! index                   newline-delimited staged entry ids
//! objects/<id>            content-addressed entries and commits (JSON)
//! refs/heads/<b>          a commit id
//! refs/remotes/<r>/<b>    last-known commit id on remote <r>
//! ```

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};

use crate::config::{Config, Identity};
use crate::object::{Commit, Entry, Object};

/// The instructions dropped into a fresh repository for AI agents. This is the
/// project README verbatim: the same plain-text document serves humans and
/// agents, so there is one source of truth and no drift.
pub const AGENTS_TEMPLATE: &str = include_str!("../README");

/// A handle to a Lore repository rooted at the directory containing `.lore`.
pub struct Repo {
    root: PathBuf,
}

/// What a snapshot of the repository looks like right now.
pub struct Status {
    pub branch: String,
    pub head: Option<String>,
    pub staged: Vec<(String, Entry)>,
}

/// A commit with the entries it recorded, returned by [`Repo::show`].
pub struct CommitView {
    pub id: String,
    pub commit: Commit,
    pub entries: Vec<(String, Entry)>,
}

/// The result of a merge.
#[derive(Debug, PartialEq, Eq)]
pub enum Merge {
    /// Already contains the other branch's intent.
    UpToDate,
    /// The ref advanced directly to the given commit.
    FastForward(String),
    /// A new merge commit was created.
    Merged(String),
}

impl Repo {
    /// Lay out an empty `.lore/`. Shared by `init` and `clone`.
    pub(crate) fn scaffold(root: &Path) -> Result<Repo> {
        let dir = root.join(".lore");
        if dir.exists() {
            bail!("a Lore repository already exists at {}", dir.display());
        }
        fs::create_dir_all(dir.join("objects"))?;
        fs::create_dir_all(dir.join("refs/heads"))?;
        fs::create_dir_all(dir.join("refs/remotes"))?;
        fs::write(dir.join("HEAD"), "ref: refs/heads/main\n")?;
        fs::write(dir.join("index"), "")?;
        Ok(Repo { root: root.into() })
    }

    /// Create a new repository under `root`. Also writes `AGENTS.md` unless one
    /// already exists.
    pub fn init(root: &Path) -> Result<Repo> {
        let repo = Repo::scaffold(root)?;
        let agents = root.join("AGENTS.md");
        if !agents.exists() {
            fs::write(&agents, AGENTS_TEMPLATE)?;
        }
        Ok(repo)
    }

    /// Open the repository rooted exactly at `root`.
    pub fn open(root: &Path) -> Result<Repo> {
        if !root.join(".lore").is_dir() {
            bail!("not a Lore repository: {}", root.display());
        }
        Ok(Repo { root: root.into() })
    }

    /// Find the repository containing `start`, walking up to the filesystem root.
    pub fn discover(start: &Path) -> Result<Repo> {
        let mut cur = start;
        loop {
            if cur.join(".lore").is_dir() {
                return Ok(Repo { root: cur.into() });
            }
            cur = cur.parent().ok_or_else(|| {
                anyhow!(
                    "not a Lore repository (or any parent of {})",
                    start.display()
                )
            })?;
        }
    }

    // --- paths ---

    fn dir(&self) -> PathBuf {
        self.root.join(".lore")
    }
    fn object_path(&self, id: &str) -> PathBuf {
        self.dir().join("objects").join(id)
    }
    fn ref_path(&self, branch: &str) -> PathBuf {
        self.dir().join("refs/heads").join(branch)
    }
    fn remote_ref_path(&self, remote: &str, branch: &str) -> PathBuf {
        self.dir().join("refs/remotes").join(remote).join(branch)
    }
    fn index_path(&self) -> PathBuf {
        self.dir().join("index")
    }
    fn config_path(&self) -> PathBuf {
        self.dir().join("config")
    }

    // --- config ---

    /// The repository config (identity and remotes); default when absent.
    pub fn config(&self) -> Result<Config> {
        match fs::read(self.config_path()) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e.into()),
        }
    }

    /// Persist the repository config.
    pub fn save_config(&self, config: &Config) -> Result<()> {
        fs::write(self.config_path(), serde_json::to_vec_pretty(config)?)?;
        Ok(())
    }

    // --- object store ---

    /// Write an object, returning its id. Existing objects are left untouched,
    /// which is what gives identical intent a single stored copy.
    pub fn write_object(&self, obj: &Object) -> Result<String> {
        let id = obj.id();
        let path = self.object_path(&id);
        if !path.exists() {
            fs::write(&path, obj.to_bytes())?;
        }
        Ok(id)
    }

    /// Read an object by its full id.
    pub fn read_object(&self, id: &str) -> Result<Object> {
        let bytes =
            fs::read(self.object_path(id)).with_context(|| format!("no such object: {id}"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    /// Whether an object is already stored locally.
    pub(crate) fn has_object(&self, id: &str) -> bool {
        self.object_path(id).exists()
    }

    /// The raw stored bytes of an object, for handing to a remote.
    pub(crate) fn object_bytes(&self, id: &str) -> Result<Vec<u8>> {
        fs::read(self.object_path(id)).with_context(|| format!("no such object: {id}"))
    }

    /// Store object bytes received from a remote, after checking the bytes
    /// actually hash to `id`, so a remote cannot poison the store.
    pub(crate) fn write_object_bytes(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let obj: Object =
            serde_json::from_slice(bytes).with_context(|| format!("invalid object {id}"))?;
        if obj.id() != id {
            bail!("object id mismatch: bytes for {id} hash to {}", obj.id());
        }
        let path = self.object_path(id);
        if !path.exists() {
            fs::write(path, bytes)?;
        }
        Ok(())
    }

    pub(crate) fn read_commit(&self, id: &str) -> Result<Commit> {
        match self.read_object(id)? {
            Object::Commit(c) => Ok(c),
            Object::Entry(_) => bail!("{id} is not a commit"),
        }
    }

    fn read_entry(&self, id: &str) -> Result<Entry> {
        match self.read_object(id)? {
            Object::Entry(e) => Ok(e),
            Object::Commit(_) => bail!("{id} is not an entry"),
        }
    }

    // --- refs and HEAD ---

    /// The branch HEAD currently points at.
    pub fn current_branch(&self) -> Result<String> {
        let head = fs::read_to_string(self.dir().join("HEAD"))?;
        head.trim()
            .strip_prefix("ref: refs/heads/")
            .map(str::to_string)
            .ok_or_else(|| anyhow!("malformed HEAD: {head:?}"))
    }

    /// The commit id a branch points at, if it has one.
    pub fn read_ref(&self, branch: &str) -> Result<Option<String>> {
        read_ref_file(&self.ref_path(branch))
    }

    pub(crate) fn write_ref(&self, branch: &str, id: &str) -> Result<()> {
        fs::write(self.ref_path(branch), format!("{id}\n"))?;
        Ok(())
    }

    /// The last-known commit a remote's branch pointed at, if tracked.
    pub(crate) fn read_remote_ref(&self, remote: &str, branch: &str) -> Result<Option<String>> {
        read_ref_file(&self.remote_ref_path(remote, branch))
    }

    pub(crate) fn write_remote_ref(&self, remote: &str, branch: &str, id: &str) -> Result<()> {
        let path = self.remote_ref_path(remote, branch);
        fs::create_dir_all(path.parent().expect("ref path has a parent"))?;
        fs::write(path, format!("{id}\n"))?;
        Ok(())
    }

    /// Point HEAD at a branch (without touching the branch ref).
    pub(crate) fn set_head(&self, branch: &str) -> Result<()> {
        fs::write(
            self.dir().join("HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )?;
        Ok(())
    }

    /// The commit at the tip of the current branch, if any.
    pub fn head_commit(&self) -> Result<Option<String>> {
        self.read_ref(&self.current_branch()?)
    }

    // --- staging ---

    fn read_index(&self) -> Result<Vec<String>> {
        let raw = fs::read_to_string(self.index_path())?;
        Ok(dedup(
            raw.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(String::from),
        ))
    }

    // --- commands ---

    /// Stage one unit of intent. Fast by design: one object write and one
    /// append to the index.
    pub fn add(&self, who: &Identity, text: &str, now: u64) -> Result<String> {
        if text.trim().is_empty() {
            bail!("refusing to record empty intent");
        }
        let id = self.write_object(&Object::Entry(Entry {
            author: who.clone(),
            timestamp: now,
            text: text.into(),
        }))?;
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(self.index_path())?;
        use std::io::Write;
        writeln!(f, "{id}")?;
        Ok(id)
    }

    /// A snapshot of branch, head, and staged intent.
    pub fn status(&self) -> Result<Status> {
        let staged = self
            .read_index()?
            .into_iter()
            .map(|id| Ok((id.clone(), self.read_entry(&id)?)))
            .collect::<Result<_>>()?;
        Ok(Status {
            branch: self.current_branch()?,
            head: self.head_commit()?,
            staged,
        })
    }

    /// Record staged intent as a commit on the current branch.
    pub fn commit(&self, who: &Identity, message: &str, now: u64) -> Result<String> {
        let entries = self.read_index()?;
        if entries.is_empty() {
            bail!("nothing staged to commit");
        }
        let parents = self.head_commit()?.into_iter().collect();
        let id = self.write_object(&Object::Commit(Commit {
            parents,
            author: who.clone(),
            timestamp: now,
            message: message.into(),
            entries,
        }))?;
        self.write_ref(&self.current_branch()?, &id)?;
        fs::write(self.index_path(), "")?;
        Ok(id)
    }

    /// Every commit reachable from `start` by following parents.
    pub(crate) fn reachable(&self, start: &str) -> Result<HashSet<String>> {
        let mut seen = HashSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(id) = stack.pop() {
            if seen.insert(id.clone()) {
                stack.extend(self.read_commit(&id)?.parents);
            }
        }
        Ok(seen)
    }

    fn is_ancestor(&self, ancestor: &str, descendant: &str) -> Result<bool> {
        Ok(self.reachable(descendant)?.contains(ancestor))
    }

    /// History reachable from HEAD, newest first.
    pub fn log(&self) -> Result<Vec<(String, Commit)>> {
        let Some(head) = self.head_commit()? else {
            return Ok(vec![]);
        };
        let mut commits = self
            .reachable(&head)?
            .into_iter()
            .map(|id| Ok((id.clone(), self.read_commit(&id)?)))
            .collect::<Result<Vec<_>>>()?;
        commits.sort_by(|a, b| b.1.timestamp.cmp(&a.1.timestamp).then(b.0.cmp(&a.0)));
        Ok(commits)
    }

    /// A specific commit and the entries it recorded.
    pub fn show(&self, reference: &str) -> Result<CommitView> {
        let id = self.resolve(reference)?;
        let commit = self.read_commit(&id)?;
        let entries = commit
            .entries
            .iter()
            .map(|e| Ok((e.clone(), self.read_entry(e)?)))
            .collect::<Result<_>>()?;
        Ok(CommitView {
            id,
            commit,
            entries,
        })
    }

    /// Branch names, with the current branch always present, sorted.
    pub fn branches(&self) -> Result<Vec<String>> {
        let mut names: HashSet<String> = HashSet::new();
        names.insert(self.current_branch()?);
        for entry in fs::read_dir(self.dir().join("refs/heads"))? {
            if let Ok(name) = entry?.file_name().into_string() {
                names.insert(name);
            }
        }
        let mut names: Vec<_> = names.into_iter().collect();
        names.sort();
        Ok(names)
    }

    /// Create a branch at the current head.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        if self.ref_path(name).exists() {
            bail!("branch '{name}' already exists");
        }
        let head = self
            .head_commit()?
            .ok_or_else(|| anyhow!("cannot branch with no commits yet"))?;
        self.write_ref(name, &head)
    }

    /// Point HEAD at an existing branch, optionally creating it first.
    pub fn checkout(&self, name: &str, create: bool) -> Result<()> {
        if create {
            self.create_branch(name)?;
        }
        if name != self.current_branch()? && !self.ref_path(name).exists() {
            bail!("branch '{name}' does not exist");
        }
        self.set_head(name)
    }

    /// Merge another branch into the current branch.
    pub fn merge(&self, other: &str, who: &Identity, message: &str, now: u64) -> Result<Merge> {
        let other_id = self
            .read_ref(other)?
            .ok_or_else(|| anyhow!("branch '{other}' has no commits to merge"))?;
        self.merge_commit(&other_id, who, message, now)
    }

    /// Merge a specific commit into the current branch. Because materialization
    /// unions the whole commit graph, a merge only joins the two histories.
    pub(crate) fn merge_commit(
        &self,
        other_id: &str,
        who: &Identity,
        message: &str,
        now: u64,
    ) -> Result<Merge> {
        let branch = self.current_branch()?;
        let Some(head) = self.head_commit()? else {
            self.write_ref(&branch, other_id)?;
            return Ok(Merge::FastForward(other_id.to_string()));
        };
        if head == other_id || self.is_ancestor(other_id, &head)? {
            return Ok(Merge::UpToDate);
        }
        if self.is_ancestor(&head, other_id)? {
            self.write_ref(&branch, other_id)?;
            return Ok(Merge::FastForward(other_id.to_string()));
        }
        let id = self.write_object(&Object::Commit(Commit {
            parents: vec![head, other_id.to_string()],
            author: who.clone(),
            timestamp: now,
            message: message.into(),
            entries: self.read_index()?,
        }))?;
        self.write_ref(&branch, &id)?;
        fs::write(self.index_path(), "")?;
        Ok(Merge::Merged(id))
    }

    /// Resolve a ref to a commit id: `HEAD`, a branch, a `<remote>/<branch>`
    /// tracking ref, or a commit id prefix.
    pub fn resolve(&self, reference: &str) -> Result<String> {
        if reference == "HEAD" {
            return self
                .head_commit()?
                .ok_or_else(|| anyhow!("HEAD has no commits yet"));
        }
        if let Some(id) = self.read_ref(reference)? {
            return Ok(id);
        }
        if let Some((remote, branch)) = reference.split_once('/') {
            if let Some(id) = self.read_remote_ref(remote, branch)? {
                return Ok(id);
            }
        }
        let mut hits = vec![];
        for entry in fs::read_dir(self.dir().join("objects"))? {
            let name = entry?
                .file_name()
                .into_string()
                .map_err(|_| anyhow!("non-utf8 object name"))?;
            if name.starts_with(reference) && matches!(self.read_object(&name)?, Object::Commit(_))
            {
                hits.push(name);
            }
        }
        match hits.len() {
            1 => Ok(hits.remove(0)),
            0 => bail!("unknown ref: {reference}"),
            _ => bail!("ambiguous ref '{reference}' matches {} commits", hits.len()),
        }
    }

    /// Collect the deduplicated, chronological intent reachable from `reference`
    /// and render it as a brief an agent can act on.
    pub fn materialize(&self, reference: &str, now: u64) -> Result<String> {
        let commit_id = self.resolve(reference)?;
        let commit_ids = self.reachable(&commit_id)?;

        let mut entry_ids = vec![];
        let mut seen = HashSet::new();
        for cid in &commit_ids {
            for eid in self.read_commit(cid)?.entries {
                if seen.insert(eid.clone()) {
                    entry_ids.push(eid);
                }
            }
        }
        let mut entries = entry_ids
            .into_iter()
            .map(|id| Ok((id.clone(), self.read_entry(&id)?)))
            .collect::<Result<Vec<_>>>()?;
        entries.sort_by(|a, b| a.1.timestamp.cmp(&b.1.timestamp).then(a.0.cmp(&b.0)));

        Ok(render_brief(
            reference,
            &commit_id,
            commit_ids.len(),
            &entries,
            now,
        ))
    }
}

/// Read a ref file, returning the trimmed commit id or `None` if absent/empty.
fn read_ref_file(path: &Path) -> Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => Ok(Some(s.trim().to_string())),
        Ok(_) => Ok(None),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Drop duplicate strings while preserving first-seen order.
fn dedup(items: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    items
        .into_iter()
        .filter(|x| seen.insert(x.clone()))
        .collect()
}

/// Short, git-style id prefix.
pub fn short(id: &str) -> &str {
    &id[..id.len().min(10)]
}

fn render_brief(
    reference: &str,
    commit_id: &str,
    commits: usize,
    entries: &[(String, Entry)],
    now: u64,
) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "# Lore Materialization\n");
    let _ = writeln!(s, "Target: {reference} ({})", short(commit_id));
    let _ = writeln!(s, "Commits: {commits}   Intent entries: {}", entries.len());
    let _ = writeln!(s, "Generated: {}\n", crate::time::format_ns(now));
    let _ = writeln!(
        s,
        "## How to use this brief\n\n\
         The entries below are the complete, deduplicated, chronological record of\n\
         what everyone on this project asked for and decided. Reconcile the working\n\
         directory to satisfy this intent: keep what is already correct, add what is\n\
         missing, and remove what now contradicts it. Do not commit this file.\n"
    );
    let _ = writeln!(s, "## Intent\n");
    if entries.is_empty() {
        let _ = writeln!(s, "_No intent recorded yet._");
    }
    for (_, e) in entries {
        let _ = writeln!(
            s,
            "### {} - {}",
            e.author.label(),
            crate::time::format_ns(e.timestamp)
        );
        let _ = writeln!(s, "{}\n", e.text.trim());
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repo() -> (TempDir, Repo) {
        let dir = TempDir::new().unwrap();
        let repo = Repo::init(dir.path()).unwrap();
        (dir, repo)
    }

    fn who(name: &str) -> Identity {
        Identity::new(name, format!("{name}@test"))
    }

    #[test]
    fn init_lays_out_the_store() {
        let (dir, _) = repo();
        let lore = dir.path().join(".lore");
        assert!(lore.join("objects").is_dir());
        assert!(lore.join("refs/heads").is_dir());
        assert!(lore.join("refs/remotes").is_dir());
        assert_eq!(
            fs::read_to_string(lore.join("HEAD")).unwrap(),
            "ref: refs/heads/main\n"
        );
        assert!(dir.path().join("AGENTS.md").is_file());
    }

    #[test]
    fn init_twice_is_an_error() {
        let (dir, _) = repo();
        assert!(Repo::init(dir.path()).is_err());
    }

    #[test]
    fn init_keeps_an_existing_agents_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("AGENTS.md"), "mine").unwrap();
        Repo::init(dir.path()).unwrap();
        assert_eq!(
            fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn discover_walks_up() {
        let (dir, _) = repo();
        let nested = dir.path().join("a/b");
        fs::create_dir_all(&nested).unwrap();
        assert!(Repo::discover(&nested).is_ok());
        assert!(Repo::discover(Path::new("/")).is_err());
    }

    #[test]
    fn config_round_trips_on_disk() {
        let (_d, r) = repo();
        assert_eq!(r.config().unwrap(), Config::default());
        let c = Config {
            user: who("ray"),
            remotes: std::collections::BTreeMap::from([(
                "origin".to_string(),
                crate::config::Remote { url: "/srv".into() },
            )]),
        };
        r.save_config(&c).unwrap();
        assert_eq!(r.config().unwrap(), c);
    }

    #[test]
    fn add_stages_and_dedupes() {
        let (_d, r) = repo();
        r.add(&who("alice"), "use sqlite", 1).unwrap();
        r.add(&who("bob"), "use sqlite", 2).unwrap(); // identical intent
        r.add(&who("alice"), "ship it", 3).unwrap();
        let staged = r.status().unwrap().staged;
        assert_eq!(staged.len(), 2);
    }

    #[test]
    fn empty_intent_is_rejected() {
        let (_d, r) = repo();
        assert!(r.add(&who("alice"), "   ", 1).is_err());
    }

    #[test]
    fn commit_advances_head_and_clears_index() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        let c = r.commit(&who("alice"), "first", 10).unwrap();
        assert_eq!(r.head_commit().unwrap(), Some(c.clone()));
        assert!(r.status().unwrap().staged.is_empty());
        assert_eq!(r.read_commit(&c).unwrap().parents, Vec::<String>::new());
    }

    #[test]
    fn identity_is_recorded_in_the_brief() {
        let (_d, r) = repo();
        r.add(&Identity::new("Ray", "ray@x.com"), "a", 1).unwrap();
        r.commit(&who("alice"), "c", 10).unwrap();
        assert!(r
            .materialize("HEAD", 100)
            .unwrap()
            .contains("Ray <ray@x.com>"));
    }

    #[test]
    fn second_commit_links_to_first() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        let c1 = r.commit(&who("alice"), "first", 10).unwrap();
        r.add(&who("alice"), "b", 2).unwrap();
        let c2 = r.commit(&who("alice"), "second", 20).unwrap();
        assert_eq!(r.read_commit(&c2).unwrap().parents, vec![c1]);
    }

    #[test]
    fn commit_with_nothing_staged_errors() {
        let (_d, r) = repo();
        assert!(r.commit(&who("alice"), "x", 1).is_err());
    }

    #[test]
    fn log_is_newest_first() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        r.commit(&who("alice"), "first", 10).unwrap();
        r.add(&who("alice"), "b", 2).unwrap();
        r.commit(&who("alice"), "second", 20).unwrap();
        let msgs: Vec<_> = r
            .log()
            .unwrap()
            .into_iter()
            .map(|(_, c)| c.message)
            .collect();
        assert_eq!(msgs, vec!["second", "first"]);
    }

    #[test]
    fn log_is_empty_before_any_commit() {
        let (_d, r) = repo();
        assert!(r.log().unwrap().is_empty());
    }

    #[test]
    fn branch_and_checkout() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        r.commit(&who("alice"), "first", 10).unwrap();
        r.create_branch("feature").unwrap();
        assert!(r.create_branch("feature").is_err());
        assert_eq!(r.branches().unwrap(), vec!["feature", "main"]);
        r.checkout("feature", false).unwrap();
        assert_eq!(r.current_branch().unwrap(), "feature");
        assert!(r.checkout("ghost", false).is_err());
    }

    #[test]
    fn checkout_dash_b_creates() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        r.commit(&who("alice"), "first", 10).unwrap();
        r.checkout("feature", true).unwrap();
        assert_eq!(r.current_branch().unwrap(), "feature");
    }

    #[test]
    fn branch_without_commits_errors() {
        let (_d, r) = repo();
        assert!(r.create_branch("x").is_err());
    }

    #[test]
    fn merge_fast_forwards() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        let base = r.commit(&who("alice"), "base", 10).unwrap();
        r.checkout("feature", true).unwrap();
        r.add(&who("alice"), "b", 2).unwrap();
        let tip = r.commit(&who("alice"), "feat", 20).unwrap();
        r.checkout("main", false).unwrap();
        assert_eq!(r.head_commit().unwrap(), Some(base));
        assert_eq!(
            r.merge("feature", &who("alice"), "m", 30).unwrap(),
            Merge::FastForward(tip.clone())
        );
        assert_eq!(r.head_commit().unwrap(), Some(tip));
    }

    #[test]
    fn merge_up_to_date() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        r.commit(&who("alice"), "base", 10).unwrap();
        r.create_branch("feature").unwrap();
        assert_eq!(
            r.merge("feature", &who("alice"), "m", 30).unwrap(),
            Merge::UpToDate
        );
    }

    #[test]
    fn merge_creates_a_merge_commit_and_unions_intent() {
        let (_d, r) = repo();
        r.add(&who("alice"), "base", 1).unwrap();
        r.commit(&who("alice"), "base", 10).unwrap();
        r.checkout("feature", true).unwrap();
        r.add(&who("alice"), "from-feature", 2).unwrap();
        r.commit(&who("alice"), "feat", 20).unwrap();
        r.checkout("main", false).unwrap();
        r.add(&who("alice"), "from-main", 3).unwrap();
        r.commit(&who("alice"), "main work", 25).unwrap();

        let outcome = r
            .merge("feature", &who("alice"), "merge feature", 30)
            .unwrap();
        let Merge::Merged(mid) = outcome else {
            panic!("expected a merge commit")
        };
        assert_eq!(r.read_commit(&mid).unwrap().parents.len(), 2);

        // Materializing the merge sees intent from both branches.
        let brief = r.materialize("HEAD", 99).unwrap();
        assert!(brief.contains("base"));
        assert!(brief.contains("from-feature"));
        assert!(brief.contains("from-main"));
    }

    #[test]
    fn materialize_is_chronological_and_deduped() {
        let (_d, r) = repo();
        r.add(&who("alice"), "second", 20).unwrap();
        r.add(&who("bob"), "first", 10).unwrap();
        r.add(&who("carol"), "first", 11).unwrap(); // duplicate intent text
        r.commit(&who("alice"), "c", 30).unwrap();
        let brief = r.materialize("HEAD", 100).unwrap();
        assert_eq!(brief.matches("first").count(), 1);
        assert!(brief.find("first").unwrap() < brief.find("second").unwrap());
        assert!(brief.contains("Intent entries: 2"));
    }

    #[test]
    fn resolve_by_head_branch_and_prefix() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        let c = r.commit(&who("alice"), "first", 10).unwrap();
        assert_eq!(r.resolve("HEAD").unwrap(), c);
        assert_eq!(r.resolve("main").unwrap(), c);
        assert_eq!(r.resolve(&c[..8]).unwrap(), c);
        assert!(r.resolve("nope").is_err());
    }

    #[test]
    fn resolve_handles_remote_tracking_ref() {
        let (_d, r) = repo();
        r.add(&who("alice"), "a", 1).unwrap();
        let c = r.commit(&who("alice"), "first", 10).unwrap();
        r.write_remote_ref("origin", "main", &c).unwrap();
        assert_eq!(
            r.read_remote_ref("origin", "main").unwrap(),
            Some(c.clone())
        );
        assert_eq!(r.resolve("origin/main").unwrap(), c);
    }

    #[test]
    fn materialize_specific_commit_excludes_later_intent() {
        let (_d, r) = repo();
        r.add(&who("alice"), "early", 1).unwrap();
        let first = r.commit(&who("alice"), "first", 10).unwrap();
        r.add(&who("alice"), "late", 2).unwrap();
        r.commit(&who("alice"), "second", 20).unwrap();
        let brief = r.materialize(&first, 100).unwrap();
        assert!(brief.contains("early"));
        assert!(!brief.contains("late"));
    }

    #[test]
    fn materialize_head_follows_the_current_branch() {
        let (_d, r) = repo();
        r.add(&who("alice"), "on main", 1).unwrap();
        r.commit(&who("alice"), "main", 10).unwrap();
        r.checkout("feature", true).unwrap();
        r.add(&who("alice"), "on feature", 2).unwrap();
        r.commit(&who("alice"), "feat", 20).unwrap();

        // HEAD with no ref resolves to the current branch tip.
        assert!(r.materialize("HEAD", 100).unwrap().contains("on feature"));
        r.checkout("main", false).unwrap();
        let on_main = r.materialize("HEAD", 100).unwrap();
        assert!(on_main.contains("on main"));
        assert!(!on_main.contains("on feature"));
    }

    #[test]
    fn long_intent_round_trips_in_full() {
        let (_d, r) = repo();
        let long = "x".repeat(5000);
        r.add(&who("alice"), &long, 1).unwrap();
        r.commit(&who("alice"), "c", 10).unwrap();
        assert!(r.materialize("HEAD", 100).unwrap().contains(&long));
    }

    #[test]
    fn materialize_orders_intent_within_the_same_second() {
        // Same wall-clock second, a few nanoseconds apart: the brief must still
        // read in the order intent was recorded, not by hash.
        let (_d, r) = repo();
        let s = 1_700_000_000 * 1_000_000_000;
        r.add(&who("alice"), "make-red", s + 10).unwrap();
        r.add(&who("alice"), "swap", s + 20).unwrap();
        r.commit(&who("alice"), "c", s + 30).unwrap();
        let brief = r.materialize("HEAD", s + 40).unwrap();
        assert!(brief.find("make-red").unwrap() < brief.find("swap").unwrap());
    }

    #[test]
    fn show_returns_commit_and_entries() {
        let (_d, r) = repo();
        r.add(&who("ray"), "use sqlite", 1).unwrap();
        let c = r
            .commit(&who("ray"), "a long message show keeps in full", 10)
            .unwrap();
        let v = r.show("HEAD").unwrap();
        assert_eq!(v.id, c);
        assert_eq!(v.commit.message, "a long message show keeps in full");
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.entries[0].1.text, "use sqlite");
    }
}
