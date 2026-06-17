//! The command line surface. Mirrors git so it feels familiar: `init`, `add`,
//! `status`, `commit`, `log`, `branch`, `checkout`, `merge`, `materialize`.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use clap::{Parser, Subcommand};

use crate::repo::{short, Merge, Repo};

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
        /// Category: prompt, note, or decision
        #[arg(short, long, default_value = "prompt")]
        kind: String,
        /// Override the recorded author
        #[arg(short, long)]
        author: Option<String>,
    },
    /// Show the current branch, head, and staged intent
    Status,
    /// Record staged intent as a commit
    Commit {
        #[arg(short, long)]
        message: String,
        #[arg(short, long)]
        author: Option<String>,
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
        Command::Add { text, kind, author } => {
            let repo = Repo::discover(cwd)?;
            let id = repo.add(
                &kind,
                &author.unwrap_or_else(default_author),
                &text.join(" "),
                now(),
            )?;
            println!("staged [{kind}] {}", short(&id));
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
                    println!("  {} [{}] {}", short(&id), e.kind, oneline(&e.text));
                }
            }
        }
        Command::Commit { message, author } => {
            let repo = Repo::discover(cwd)?;
            let id = repo.commit(&author.unwrap_or_else(default_author), &message, now())?;
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
                    c.author,
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
        } => {
            let repo = Repo::discover(cwd)?;
            let message = message.unwrap_or_else(|| format!("Merge branch '{branch}'"));
            match repo.merge(
                &branch,
                &author.unwrap_or_else(default_author),
                &message,
                now(),
            )? {
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

fn default_author() -> String {
    std::env::var("LORE_AUTHOR")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "anonymous".into())
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
    fn parses_add_with_flags() {
        let cli = Cli::try_parse_from(["lore", "add", "use", "sqlite", "-k", "decision"]).unwrap();
        match cli.command {
            Command::Add { text, kind, .. } => {
                assert_eq!(text, vec!["use", "sqlite"]);
                assert_eq!(kind, "decision");
            }
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
    fn oneline_is_untruncated_first_line() {
        assert_eq!(oneline("hi\nthere"), "hi");
        assert_eq!(oneline(&"x".repeat(100)).len(), 100);
    }
}
