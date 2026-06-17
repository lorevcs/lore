//! The command line surface. Mirrors git so it feels familiar: `init`, `add`,
//! `status`, `commit`, `log`, `branch`, `checkout`, `merge`, `materialize`, plus
//! remotes (`clone`, `push`, `fetch`, `pull`, `remote`) and `config`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};

use crate::config::{Identity, Remote};
use crate::repo::{short, Merge, Repo};
use crate::sync::Push;

#[derive(Parser)]
#[command(
    name = "lore",
    version,
    about = "The latent repository. Track intent, not code."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Create a new Lore repository in the current directory
    Init,
    /// Stage a unit of intent (a prompt, note, or decision)
    Add {
        /// The intent text
        #[arg(required = true, num_args = 1.., value_name = "TEXT")]
        text: Vec<String>,
        /// Override the recorded author name
        #[arg(short, long)]
        author: Option<String>,
        /// Override the recorded author email
        #[arg(short, long)]
        email: Option<String>,
    },
    /// Show the current branch, head, and staged intent
    Status,
    /// Record staged intent as a commit
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(short, long)]
        author: Option<String>,
        #[arg(short, long)]
        email: Option<String>,
    },
    /// Show commit history, newest first
    Log,
    /// List branches, or create one with a name
    Branch { name: Option<String> },
    /// Switch to another branch
    Checkout {
        name: String,
        /// Create the branch before switching
        #[arg(short = 'b')]
        create: bool,
    },
    /// Merge another branch's intent into the current branch
    Merge {
        branch: String,
        #[arg(short, long)]
        message: Option<String>,
        #[arg(short, long)]
        author: Option<String>,
        #[arg(short, long)]
        email: Option<String>,
    },
    /// Render accumulated intent into a brief an agent can rebuild from
    Materialize {
        /// Branch or commit to materialize
        #[arg(short = 'r', long = "ref", value_name = "REF", default_value = "HEAD")]
        reference: String,
        /// Write to a file instead of stdout
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Copy a remote repository into a new directory
    Clone { url: String, dir: Option<String> },
    /// Send local intent to a remote (default: origin, current branch)
    Push {
        remote: Option<String>,
        branch: Option<String>,
    },
    /// Download remote intent into tracking refs (default: origin)
    Fetch { remote: Option<String> },
    /// Fetch and merge a remote's matching branch (default: origin)
    Pull {
        remote: Option<String>,
        #[arg(short, long)]
        author: Option<String>,
        #[arg(short, long)]
        email: Option<String>,
    },
    /// Manage remotes
    Remote {
        #[command(subcommand)]
        command: Option<RemoteCmd>,
    },
    /// Get or set config: user.name, user.email
    Config {
        key: Option<String>,
        value: Option<String>,
    },
}

#[derive(Subcommand)]
pub enum RemoteCmd {
    /// Add a remote
    Add { name: String, url: String },
    /// Remove a remote
    Remove { name: String },
}

/// Run a parsed command against the repository discovered from `cwd`.
pub fn run(cli: Cli, cwd: &Path) -> Result<()> {
    match cli.command {
        Command::Init => {
            Repo::init(cwd)?;
            println!(
                "Initialized empty Lore repository in {}",
                cwd.join(".lore").display()
            );
            println!("Wrote AGENTS.md so agents record intent on every prompt.");
        }
        Command::Add {
            text,
            author,
            email,
        } => {
            let repo = Repo::discover(cwd)?;
            let who = identity(&repo, author, email)?;
            let id = repo.add(&who, &text.join(" "), now())?;
            println!("staged {}", short(&id));
        }
        Command::Status => {
            let repo = Repo::discover(cwd)?;
            let st = repo.status()?;
            match st.head {
                Some(id) => println!("On branch {} at {}", st.branch, short(&id)),
                None => println!("On branch {} (no commits yet)", st.branch),
            }
            if st.staged.is_empty() {
                println!("Nothing staged.");
            } else {
                println!("Staged intent:");
                for (id, e) in st.staged {
                    println!("  {} {}", short(&id), oneline(&e.text));
                }
            }
        }
        Command::Commit {
            message,
            author,
            email,
        } => {
            let repo = Repo::discover(cwd)?;
            let who = identity(&repo, author, email)?;
            let id = repo.commit(&who, &message, now())?;
            println!("committed {} {message}", short(&id));
        }
        Command::Log => {
            let repo = Repo::discover(cwd)?;
            let commits = repo.log()?;
            if commits.is_empty() {
                println!("No commits yet.");
            }
            for (id, c) in commits {
                let merge = if c.parents.len() > 1 { " (merge)" } else { "" };
                println!(
                    "{} {} {} - {}{merge} [{} intent]",
                    short(&id),
                    crate::time::format_ns(c.timestamp),
                    c.author.label(),
                    c.message,
                    c.entries.len()
                );
            }
        }
        Command::Branch { name } => {
            let repo = Repo::discover(cwd)?;
            match name {
                Some(name) => {
                    repo.create_branch(&name)?;
                    println!("created branch {name}");
                }
                None => {
                    let current = repo.current_branch()?;
                    for b in repo.branches()? {
                        println!("{} {b}", if b == current { "*" } else { " " });
                    }
                }
            }
        }
        Command::Checkout { name, create } => {
            let repo = Repo::discover(cwd)?;
            repo.checkout(&name, create)?;
            println!("switched to branch {name}");
        }
        Command::Merge {
            branch,
            message,
            author,
            email,
        } => {
            let repo = Repo::discover(cwd)?;
            let who = identity(&repo, author, email)?;
            let message = message.unwrap_or_else(|| format!("Merge branch '{branch}'"));
            match repo.merge(&branch, &who, &message, now())? {
                Merge::UpToDate => println!("already up to date"),
                Merge::FastForward(id) => println!("fast-forward to {}", short(&id)),
                Merge::Merged(id) => println!("merged into {}", short(&id)),
            }
        }
        Command::Materialize { reference, out } => {
            let repo = Repo::discover(cwd)?;
            let brief = repo.materialize(&reference, now())?;
            match out {
                Some(path) => {
                    std::fs::write(&path, &brief)?;
                    println!("wrote materialization to {}", path.display());
                }
                None => print!("{brief}"),
            }
        }
        Command::Clone { url, dir } => {
            let name = dir.unwrap_or_else(|| default_clone_dir(&url));
            if name.is_empty() {
                bail!("could not infer a directory from {url}; pass one explicitly");
            }
            let target = cwd.join(&name);
            crate::sync::clone(&url, &target, token())?;
            println!("cloned {url} into {}", target.display());
        }
        Command::Push { remote, branch } => {
            let repo = Repo::discover(cwd)?;
            let remote = remote.unwrap_or_else(|| "origin".into());
            let branch = match branch {
                Some(b) => b,
                None => repo.current_branch()?,
            };
            match crate::sync::push(&repo, &remote, &branch)? {
                Push::UpToDate => println!("everything up to date"),
                Push::Pushed { objects } => {
                    println!("pushed {branch} to {remote} ({objects} objects)")
                }
            }
        }
        Command::Fetch { remote } => {
            let repo = Repo::discover(cwd)?;
            let remote = remote.unwrap_or_else(|| "origin".into());
            for (b, id) in crate::sync::fetch(&repo, &remote)? {
                println!("{remote}/{b} -> {}", short(&id));
            }
        }
        Command::Pull {
            remote,
            author,
            email,
        } => {
            let repo = Repo::discover(cwd)?;
            let remote = remote.unwrap_or_else(|| "origin".into());
            let who = identity(&repo, author, email)?;
            match crate::sync::pull(&repo, &remote, &who, now())? {
                Merge::UpToDate => println!("already up to date"),
                Merge::FastForward(id) => println!("fast-forward to {}", short(&id)),
                Merge::Merged(id) => println!("merged into {}", short(&id)),
            }
        }
        Command::Remote { command } => {
            let repo = Repo::discover(cwd)?;
            let mut cfg = repo.config()?;
            match command {
                None => {
                    for (name, r) in &cfg.remotes {
                        println!("{name}\t{}", r.url);
                    }
                }
                Some(RemoteCmd::Add { name, url }) => {
                    cfg.remotes.insert(name.clone(), Remote { url });
                    repo.save_config(&cfg)?;
                    println!("added remote {name}");
                }
                Some(RemoteCmd::Remove { name }) => {
                    if cfg.remotes.remove(&name).is_none() {
                        bail!("no such remote: {name}");
                    }
                    repo.save_config(&cfg)?;
                    println!("removed remote {name}");
                }
            }
        }
        Command::Config { key, value } => {
            let repo = Repo::discover(cwd)?;
            let mut cfg = repo.config()?;
            match (key.as_deref(), value) {
                (None, _) => {
                    println!("user.name={}", cfg.user.name);
                    println!("user.email={}", cfg.user.email);
                }
                (Some("user.name"), None) => println!("{}", cfg.user.name),
                (Some("user.email"), None) => println!("{}", cfg.user.email),
                (Some("user.name"), Some(v)) => {
                    cfg.user.name = v;
                    repo.save_config(&cfg)?;
                }
                (Some("user.email"), Some(v)) => {
                    cfg.user.email = v;
                    repo.save_config(&cfg)?;
                }
                (Some(k), _) => bail!("unknown config key: {k} (use user.name or user.email)"),
            }
        }
    }
    Ok(())
}

// Unix nanoseconds. u64 holds nanoseconds until well past year 2500, and the
// resolution keeps intent recorded in the same second in true order.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

fn token() -> Option<String> {
    std::env::var("LORE_TOKEN").ok().filter(|s| !s.is_empty())
}

/// Resolve who is recording: flag, then env, then config, then fallback.
fn identity(repo: &Repo, author: Option<String>, email: Option<String>) -> Result<Identity> {
    let cfg = repo.config()?;
    let name = first_nonempty([author, env("LORE_AUTHOR"), Some(cfg.user.name), env("USER")])
        .unwrap_or_else(|| "anonymous".into());
    let email =
        first_nonempty([email, env("LORE_EMAIL"), Some(cfg.user.email)]).unwrap_or_default();
    Ok(Identity { name, email })
}

fn first_nonempty(options: impl IntoIterator<Item = Option<String>>) -> Option<String> {
    options.into_iter().flatten().find(|s| !s.is_empty())
}

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// The directory git-style clone would create: the last path segment of the url.
fn default_clone_dir(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

// First line of an entry, for a one-line-per-entry status listing. Intent text
// has no length limit; this only keeps `lore status` scannable.
fn oneline(text: &str) -> String {
    text.trim().lines().next().unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_add() {
        let cli = Cli::try_parse_from(["lore", "add", "use", "sqlite"]).unwrap();
        match cli.command {
            Command::Add { text, .. } => assert_eq!(text, vec!["use", "sqlite"]),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn add_requires_text() {
        assert!(Cli::try_parse_from(["lore", "add"]).is_err());
    }

    #[test]
    fn commit_requires_message() {
        assert!(Cli::try_parse_from(["lore", "commit"]).is_err());
    }

    #[test]
    fn commit_takes_email() {
        let cli = Cli::try_parse_from(["lore", "commit", "-m", "x", "--email", "a@b.com"]).unwrap();
        match cli.command {
            Command::Commit { email, .. } => assert_eq!(email.as_deref(), Some("a@b.com")),
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn materialize_defaults_to_head() {
        let cli = Cli::try_parse_from(["lore", "materialize"]).unwrap();
        match cli.command {
            Command::Materialize { reference, out } => {
                assert_eq!(reference, "HEAD");
                assert!(out.is_none());
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_clone_with_optional_dir() {
        let cli = Cli::try_parse_from(["lore", "clone", "https://lorehub.com/r"]).unwrap();
        match cli.command {
            Command::Clone { url, dir } => {
                assert_eq!(url, "https://lorehub.com/r");
                assert!(dir.is_none());
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn parses_remote_add_and_list() {
        let add = Cli::try_parse_from(["lore", "remote", "add", "origin", "/srv"]).unwrap();
        match add.command {
            Command::Remote {
                command: Some(RemoteCmd::Add { name, url }),
            } => {
                assert_eq!(name, "origin");
                assert_eq!(url, "/srv");
            }
            _ => panic!("wrong command"),
        }
        let list = Cli::try_parse_from(["lore", "remote"]).unwrap();
        assert!(matches!(list.command, Command::Remote { command: None }));
    }

    #[test]
    fn parses_config_set() {
        let cli = Cli::try_parse_from(["lore", "config", "user.email", "a@b.com"]).unwrap();
        match cli.command {
            Command::Config { key, value } => {
                assert_eq!(key.as_deref(), Some("user.email"));
                assert_eq!(value.as_deref(), Some("a@b.com"));
            }
            _ => panic!("wrong command"),
        }
    }

    #[test]
    fn clone_dir_defaults_to_last_segment() {
        assert_eq!(default_clone_dir("https://lorehub.com/u/repo"), "repo");
        assert_eq!(default_clone_dir("/tmp/srv/"), "srv");
    }

    #[test]
    fn oneline_is_untruncated_first_line() {
        assert_eq!(oneline("hi\nthere"), "hi");
        assert_eq!(oneline(&"x".repeat(100)).len(), 100);
    }
}
