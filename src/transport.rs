//! The remote transport seam. A [`Transport`] moves content-addressed objects
//! and refs in bulk. Two backends ship today: a local filesystem remote (a path
//! to another repo) and an HTTP remote (the protocol lorehub will implement).
//!
//! Transfers are batched. A push uploads all new objects in one request; a fetch
//! asks for everything reachable from the wanted tips that the client does not
//! already have, and the remote returns the whole closure in one response.
//!
//! HTTP wire protocol:
//! ```text
//! GET  /refs            -> { "<branch>": "<commit id>", ... }
//! POST /objects         <- a JSON array of objects to store (idempotent)
//! POST /fetch           <- { "want": [ids], "have": [ids] }
//!                       -> a JSON array of every object reachable from want
//!                          that is not already reachable from have
//! PUT  /refs/{branch}   <- the commit id
//! ```
//! Writes carry `Authorization: Bearer <token>` when a token is configured.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::object::Object;

/// A place objects and refs can be pushed to and fetched from, in bulk.
pub trait Transport {
    /// Branch name to commit id for every ref the remote advertises.
    fn list_refs(&self) -> Result<BTreeMap<String, String>>;
    /// Store objects (idempotent; the remote keeps any it already has).
    fn upload(&self, objects: &[Object]) -> Result<()>;
    /// Every object reachable from `want` that is not reachable from `have`.
    fn download(&self, want: &[String], have: &[String]) -> Result<Vec<Object>>;
    /// Point a branch at a commit id.
    fn set_ref(&self, branch: &str, id: &str) -> Result<()>;
}

#[derive(Serialize, Deserialize)]
struct FetchRequest {
    want: Vec<String>,
    have: Vec<String>,
}

/// Build a transport for a remote url. `http(s)://` is HTTP; anything else
/// (including a `file://` prefix) is a local filesystem path.
pub fn open(url: &str, token: Option<String>) -> Result<Box<dyn Transport>> {
    if url.starts_with("http://") || url.starts_with("https://") {
        Ok(Box::new(HttpTransport::new(url, token)))
    } else {
        let path = url.strip_prefix("file://").unwrap_or(url);
        Ok(Box::new(LocalTransport::new(Path::new(path))))
    }
}

/// A remote that is another repository on the local filesystem.
pub struct LocalTransport {
    dir: PathBuf,
}

impl LocalTransport {
    /// `root` is the repository directory; its `.lore/` is the remote store.
    pub fn new(root: &Path) -> LocalTransport {
        LocalTransport {
            dir: root.join(".lore"),
        }
    }

    fn read_object(&self, id: &str) -> Result<Option<Object>> {
        match fs::read(self.dir.join("objects").join(id)) {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Commit ids, and all object ids (commits + entries), reachable from `tips`.
    fn closure(&self, tips: &[String]) -> Result<(HashSet<String>, HashSet<String>)> {
        let mut commits = HashSet::new();
        let mut all = HashSet::new();
        let mut stack = tips.to_vec();
        while let Some(id) = stack.pop() {
            if !commits.insert(id.clone()) {
                continue;
            }
            if let Some(Object::Commit(c)) = self.read_object(&id)? {
                all.insert(id);
                for e in &c.entries {
                    all.insert(e.clone());
                }
                stack.extend(c.parents);
            }
        }
        Ok((commits, all))
    }
}

impl Transport for LocalTransport {
    fn list_refs(&self) -> Result<BTreeMap<String, String>> {
        let heads = self.dir.join("refs/heads");
        let mut refs = BTreeMap::new();
        let entries = match fs::read_dir(&heads) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(refs),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let entry = entry?;
            if let Ok(name) = entry.file_name().into_string() {
                let id = fs::read_to_string(entry.path())?;
                let id = id.trim();
                if !id.is_empty() {
                    refs.insert(name, id.to_string());
                }
            }
        }
        Ok(refs)
    }

    fn upload(&self, objects: &[Object]) -> Result<()> {
        let dir = self.dir.join("objects");
        fs::create_dir_all(&dir)?;
        for obj in objects {
            let path = dir.join(obj.id());
            if !path.exists() {
                fs::write(path, obj.to_bytes())?;
            }
        }
        Ok(())
    }

    fn download(&self, want: &[String], have: &[String]) -> Result<Vec<Object>> {
        let (have_commits, have_all) = self.closure(have)?;
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        let mut emitted = HashSet::new();
        let mut stack = want.to_vec();
        while let Some(id) = stack.pop() {
            if have_commits.contains(&id) || !visited.insert(id.clone()) {
                continue;
            }
            let obj = match self.read_object(&id)? {
                Some(o) => o,
                None => continue,
            };
            let (entries, parents) = match &obj {
                Object::Commit(c) => (c.entries.clone(), c.parents.clone()),
                Object::Entry(_) => continue,
            };
            if emitted.insert(id.clone()) {
                out.push(obj);
            }
            for e in &entries {
                if !have_all.contains(e) && emitted.insert(e.clone()) {
                    if let Some(entry) = self.read_object(e)? {
                        out.push(entry);
                    }
                }
            }
            stack.extend(parents);
        }
        Ok(out)
    }

    fn set_ref(&self, branch: &str, id: &str) -> Result<()> {
        let heads = self.dir.join("refs/heads");
        fs::create_dir_all(&heads)?;
        fs::write(heads.join(branch), format!("{id}\n"))?;
        Ok(())
    }
}

/// A remote reached over HTTP (lorehub, or any server speaking the protocol).
pub struct HttpTransport {
    base: String,
    token: Option<String>,
    agent: ureq::Agent,
}

impl HttpTransport {
    pub fn new(base: &str, token: Option<String>) -> HttpTransport {
        HttpTransport {
            base: base.trim_end_matches('/').to_string(),
            token,
            // One pooled agent so a push/fetch's requests reuse a keep-alive
            // connection instead of a fresh tcp+tls handshake each.
            agent: ureq::AgentBuilder::new().build(),
        }
    }

    fn request(&self, method: &str, url: &str) -> ureq::Request {
        let req = self.agent.request(method, url);
        match &self.token {
            Some(t) => req.set("Authorization", &format!("Bearer {t}")),
            None => req,
        }
    }
}

impl Transport for HttpTransport {
    fn list_refs(&self) -> Result<BTreeMap<String, String>> {
        let body = self
            .request("GET", &format!("{}/refs", self.base))
            .call()?
            .into_string()?;
        Ok(serde_json::from_str(&body)?)
    }

    fn upload(&self, objects: &[Object]) -> Result<()> {
        self.request("POST", &format!("{}/objects", self.base))
            .send_bytes(&serde_json::to_vec(objects)?)?;
        Ok(())
    }

    fn download(&self, want: &[String], have: &[String]) -> Result<Vec<Object>> {
        let body = serde_json::to_vec(&FetchRequest {
            want: want.to_vec(),
            have: have.to_vec(),
        })?;
        let reader = self
            .request("POST", &format!("{}/fetch", self.base))
            .send_bytes(&body)?
            .into_reader();
        Ok(serde_json::from_reader(reader)?)
    }

    fn set_ref(&self, branch: &str, id: &str) -> Result<()> {
        self.request("PUT", &format!("{}/refs/{branch}", self.base))
            .send_string(id)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Identity;
    use crate::object::{Commit, Entry};
    use tempfile::TempDir;

    fn who() -> Identity {
        Identity::new("ray", "ray@x")
    }

    /// A two-commit chain: [entry one, commit c1, entry two, commit c2->c1].
    fn chain() -> Vec<Object> {
        let e1 = Object::Entry(Entry {
            author: who(),
            timestamp: 1,
            text: "one".into(),
        });
        let c1 = Object::Commit(Commit {
            parents: vec![],
            author: who(),
            timestamp: 10,
            message: "c1".into(),
            entries: vec![e1.id()],
        });
        let e2 = Object::Entry(Entry {
            author: who(),
            timestamp: 2,
            text: "two".into(),
        });
        let c2 = Object::Commit(Commit {
            parents: vec![c1.id()],
            author: who(),
            timestamp: 20,
            message: "c2".into(),
            entries: vec![e2.id()],
        });
        vec![e1, c1, e2, c2]
    }

    fn check_round_trip(t: &dyn Transport) {
        assert!(t.list_refs().unwrap().is_empty());
        let objs = chain();
        let (c1, e2, c2) = (objs[1].id(), objs[2].id(), objs[3].id());
        t.upload(&objs).unwrap();
        t.set_ref("main", &c2).unwrap();
        assert_eq!(t.list_refs().unwrap().get("main"), Some(&c2));

        // Full closure from the tip.
        assert_eq!(t.download(std::slice::from_ref(&c2), &[]).unwrap().len(), 4);

        // Incremental: a client that already has c1 gets only c2 and its entry.
        let delta = t.download(std::slice::from_ref(&c2), &[c1]).unwrap();
        let ids: HashSet<String> = delta.iter().map(|o| o.id()).collect();
        assert_eq!(ids, HashSet::from([c2, e2]));

        // Nothing new when the client already has the tip.
        let tip = objs[3].id();
        assert!(t
            .download(std::slice::from_ref(&tip), std::slice::from_ref(&tip))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn local_transport_round_trip() {
        let dir = TempDir::new().unwrap();
        check_round_trip(&LocalTransport::new(dir.path()));
    }

    /// A tiny HTTP server speaking the lore protocol, backed by a local store.
    fn serve(dir: PathBuf) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            let backend = LocalTransport::new(&dir);
            for mut req in server.incoming_requests() {
                let method = req.method().as_str().to_string();
                let path = req.url().to_string();
                let mut body = Vec::new();
                req.as_reader().read_to_end(&mut body).unwrap();
                let resp = if method == "GET" && path == "/refs" {
                    let json = serde_json::to_string(&backend.list_refs().unwrap()).unwrap();
                    tiny_http::Response::from_string(json).boxed()
                } else if method == "POST" && path == "/objects" {
                    let objs: Vec<Object> = serde_json::from_slice(&body).unwrap();
                    backend.upload(&objs).unwrap();
                    tiny_http::Response::empty(200u16).boxed()
                } else if method == "POST" && path == "/fetch" {
                    let r: FetchRequest = serde_json::from_slice(&body).unwrap();
                    let objs = backend.download(&r.want, &r.have).unwrap();
                    tiny_http::Response::from_string(serde_json::to_string(&objs).unwrap()).boxed()
                } else if method == "PUT" {
                    let branch = path.strip_prefix("/refs/").unwrap();
                    backend
                        .set_ref(branch, String::from_utf8(body).unwrap().trim())
                        .unwrap();
                    tiny_http::Response::empty(200u16).boxed()
                } else {
                    tiny_http::Response::empty(404u16).boxed()
                };
                let _ = req.respond(resp);
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[test]
    fn http_transport_round_trip() {
        let dir = TempDir::new().unwrap();
        let base = serve(dir.path().to_path_buf());
        check_round_trip(&HttpTransport::new(&base, None));
    }

    #[test]
    fn open_dispatches_on_scheme() {
        assert!(open("https://lorehub.com/r", None).is_ok());
        assert!(open("/tmp/some/repo", None).is_ok());
        assert!(open("file:///tmp/some/repo", None).is_ok());
    }

    #[test]
    fn clone_and_push_over_http() {
        let remote = TempDir::new().unwrap();
        let base = serve(remote.path().to_path_buf());

        let a_dir = TempDir::new().unwrap();
        let a = crate::repo::Repo::init(a_dir.path()).unwrap();
        let ray = Identity::new("ray", "ray@x");
        a.add(&ray, "use sqlite", 1).unwrap();
        a.commit(&ray, "c1", 10).unwrap();
        let mut cfg = a.config().unwrap();
        cfg.remotes
            .insert("origin".into(), crate::config::Remote { url: base.clone() });
        a.save_config(&cfg).unwrap();

        crate::sync::push(&a, "origin", "main").unwrap();

        let c_dir = TempDir::new().unwrap();
        let c = crate::sync::clone(&base, &c_dir.path().join("c"), None).unwrap();
        assert_eq!(
            a.materialize("HEAD", 100).unwrap(),
            c.materialize("HEAD", 100).unwrap()
        );
        assert!(c.materialize("HEAD", 100).unwrap().contains("use sqlite"));
    }
}
