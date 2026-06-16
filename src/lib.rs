//! Lore: the latent repository. A version control system that tracks intent,
//! not code. See [`repo::Repo`] for the storage model and commands, and
//! [`cli`] for the command line surface.

pub mod cli;
pub mod object;
pub mod repo;
pub mod time;

pub use repo::Repo;
