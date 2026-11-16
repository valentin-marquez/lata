//! What lata knows about Project Zomboid's own files, apart from Steam: the game version of a
//! build and the catalog of builds seen. Shared by the app and the job that keeps the catalog.

pub mod catalog;
pub mod version;

pub use version::Version;
