//! The remote transport seam. A [`Transport`] moves content-addressed objects
//! and refs to and from somewhere else. Two backends ship today: a local
//! filesystem remote (a path to another repo, like git's file remotes) and an
//! HTTP remote (the protocol lorehub will implement).
//!
//! HTTP wire protocol:
//! ```text
//! GET  /refs            -> { "<branch>": "<commit id>", ... }
//! HEAD /objects/{id}    -> 200 if present, 404 if missing
//! GET  /objects/{id}    -> raw stored object bytes
//! PUT  /objects/{id}    -> store body (idempotent, content-addressed)
//! PUT  /refs/{branch}   -> body is the commit id
//! ```
//! Writes carry `Authorization: Bearer <token>` when a token is configured.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// A place objects and refs can be pushed to and fetched from.
pub trait Transport {
    /// Branch name to commit id for every ref the remote advertises.
    fn list_refs(&self) -> Result<BTreeMap<String, String>>;
    /// Whether the remote already stores this object.
    fn has_object(&self, id: &str) -> Result<bool>;
    /// The raw stored bytes of an object.
    fn get_object(&self, id: &str) -> Result<Vec<u8>>;
    /// Store an object (idempotent; the remote may keep an existing copy).
    fn put_object(&self, id: &str, bytes: &[u8]) -> Result<()>;
    /// Point a branch at a commit id.
    fn set_ref(&self, branch: &str, id: &str) -> Result<()>;
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

    fn has_object(&self, id: &str) -> Result<bool> {
        Ok(self.dir.join("objects").join(id).exists())
    }

    fn get_object(&self, id: &str) -> Result<Vec<u8>> {
        fs::read(self.dir.join("objects").join(id)).with_context(|| format!("no such object: {id}"))
    }

    fn put_object(&self, id: &str, bytes: &[u8]) -> Result<()> {
        let objects = self.dir.join("objects");
        fs::create_dir_all(&objects)?;
        let path = objects.join(id);
        if !path.exists() {
            fs::write(path, bytes)?;
        }
        Ok(())
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
            // One pooled agent so a push/fetch's many small requests reuse a
            // keep-alive connection instead of a fresh tcp+tls handshake each.
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

    fn has_object(&self, id: &str) -> Result<bool> {
        match self
            .request("HEAD", &format!("{}/objects/{id}", self.base))
            .call()
        {
            Ok(_) => Ok(true),
            Err(ureq::Error::Status(404, _)) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    fn get_object(&self, id: &str) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        self.request("GET", &format!("{}/objects/{id}", self.base))
            .call()?
            .into_reader()
            .read_to_end(&mut buf)?;
        Ok(buf)
    }

    fn put_object(&self, id: &str, bytes: &[u8]) -> Result<()> {
        self.request("PUT", &format!("{}/objects/{id}", self.base))
            .send_bytes(bytes)?;
        Ok(())
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
    use crate::object::{Entry, Object};
    use tempfile::TempDir;

    fn sample_entry(text: &str) -> (String, Vec<u8>) {
        let obj = Object::Entry(Entry {
            author: Identity::new("ray", "ray@x"),
            timestamp: 1,
            text: text.into(),
        });
        (obj.id(), obj.to_bytes())
    }

    #[test]
    fn local_transport_round_trip() {
        let dir = TempDir::new().unwrap();
        let t = LocalTransport::new(dir.path());
        assert!(t.list_refs().unwrap().is_empty()); // nothing there yet

        let (id, bytes) = sample_entry("use sqlite");
        assert!(!t.has_object(&id).unwrap());
        t.put_object(&id, &bytes).unwrap();
        assert!(t.has_object(&id).unwrap());
        assert_eq!(t.get_object(&id).unwrap(), bytes);

        t.set_ref("main", "deadbeef").unwrap();
        assert_eq!(
            t.list_refs().unwrap().get("main").map(String::as_str),
            Some("deadbeef")
        );
    }

    /// A tiny HTTP server speaking the lore protocol, backed by a local store.
    /// Returns its base url; the thread runs until the process exits.
    fn serve(dir: PathBuf) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            let backend = LocalTransport::new(&dir);
            for mut req in server.incoming_requests() {
                let method = req.method().as_str().to_string();
                let path = req.url().to_string();
                let resp = if method == "GET" && path == "/refs" {
                    let json = serde_json::to_string(&backend.list_refs().unwrap()).unwrap();
                    tiny_http::Response::from_string(json).boxed()
                } else if let Some(id) = path.strip_prefix("/objects/") {
                    match method.as_str() {
                        "HEAD" if backend.has_object(id).unwrap() => {
                            tiny_http::Response::empty(200u16).boxed()
                        }
                        "GET" if backend.has_object(id).unwrap() => {
                            tiny_http::Response::from_data(backend.get_object(id).unwrap()).boxed()
                        }
                        "PUT" => {
                            let mut b = Vec::new();
                            req.as_reader().read_to_end(&mut b).unwrap();
                            backend.put_object(id, &b).unwrap();
                            tiny_http::Response::empty(200u16).boxed()
                        }
                        _ => tiny_http::Response::empty(404u16).boxed(),
                    }
                } else if let Some(branch) = path.strip_prefix("/refs/") {
                    let mut b = Vec::new();
                    req.as_reader().read_to_end(&mut b).unwrap();
                    backend
                        .set_ref(branch, String::from_utf8(b).unwrap().trim())
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
        let t = HttpTransport::new(&base, None);

        assert!(t.list_refs().unwrap().is_empty());
        let (id, bytes) = sample_entry("over http");
        assert!(!t.has_object(&id).unwrap()); // 404 -> false, not an error
        t.put_object(&id, &bytes).unwrap();
        assert!(t.has_object(&id).unwrap());
        assert_eq!(t.get_object(&id).unwrap(), bytes);

        t.set_ref("main", "cafef00d").unwrap();
        assert_eq!(
            t.list_refs().unwrap().get("main").map(String::as_str),
            Some("cafef00d")
        );
    }

    #[test]
    fn open_dispatches_on_scheme() {
        // Smoke check: scheme routing builds the right backend without panicking.
        assert!(open("https://lorehub.com/r", None).is_ok());
        assert!(open("/tmp/some/repo", None).is_ok());
        assert!(open("file:///tmp/some/repo", None).is_ok());
    }

    #[test]
    fn clone_and_push_over_http() {
        // Drive the real HttpTransport through sync against the mock server.
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
