//! macscrap engine: scanners and cleaners with no UI dependency.

#[cfg(target_os = "macos")]
pub mod apps;
#[cfg(target_os = "linux")]
#[path = "apps_linux.rs"]
pub mod apps;
pub mod background;
pub mod clean;
pub mod files;
pub mod fsutil;
pub mod health;
pub mod junk;
mod listing;
pub mod malware;
pub mod network;
pub mod space;
pub mod sys;

pub use fsutil::{human_bytes, Cancel};
