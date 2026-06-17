//! Lore: the latent repository. A version control system that tracks intent,
//! not code. See [`repo::Repo`] for the storage model and commands, and
//! [`cli`] for the command line surface.

pub mod cli;
pub mod config;
pub mod object;
pub mod repo;
pub mod sync;
pub mod time;
pub mod transport;

pub use repo::Repo;
