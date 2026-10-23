//! Filesystem helpers shared by every scanner: tilde expansion, allocated
//! size, a parallel walker, and the protected-path guard.

use crate::listing::{read_entries, Kind, Listing};
use rayon::prelude::*;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Cooperative cancellation shared between the UI and a running job.
#[derive(Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

pub fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

pub fn expand(path: &str) -> String {
    match path.strip_prefix("~") {
        Some(rest) => format!("{}{}", home().display(), rest),
        None => path.to_string(),
    }
}

/// Bytes actually allocated on disk (what deleting frees), not logical length.
pub fn allocated(meta: &fs::Metadata) -> u64 {
    meta.blocks() * 512
}

/// Total allocated size of a file or directory tree. Does not follow symlinks
/// and does not cross into other volumes.
pub fn size_of(path: &Path, cancel: &Cancel) -> u64 {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return allocated(&meta);
    }
    allocated(&meta) + dir_size(path, meta.dev(), cancel)
}

/// Pure fork-join reduction, so it is safe to call from inside rayon tasks
/// (unlike `walk`, which blocks the caller on a channel).
fn dir_size(dir: &Path, dev: u64, cancel: &Cancel) -> u64 {
    if cancel.is_cancelled() {
        return 0;
    }
    let Ok(listing) = read_entries(dir) else { return 0 };
    let mut total = 0;
    let mut subdirs = Vec::new();
    for e in &listing.entries {
        if e.dev != dev as i32 {
            continue;
        }
        match e.kind {
            Kind::File => total += e.size,
            Kind::Dir => subdirs.push(dir.join(listing.name(e))),
            Kind::Other => {}
        }
    }
    total + subdirs.par_iter().map(|d| dir_size(d, dev, cancel)).sum::<u64>()
}

/// Visit every regular file under `root` (same volume only) as
/// `(parent dir, file name, allocated bytes, logical length)`. Each directory is listed in
/// one bulk call and shipped as a single batch, so there is no per-file
/// `stat` or allocation; callers build a full path only for files they keep.
/// Subdirectories fan out across the rayon pool; the visitor runs on the
/// calling thread so it needs no `Sync` state. Directories for which
/// `prune` returns true are not entered. Must not be called from a rayon
/// worker thread.
pub fn walk(root: &Path, cancel: &Cancel, prune: fn(&Path) -> bool, visit: &mut dyn FnMut(&Path, &str, u64, u64)) {
    let Ok(meta) = fs::metadata(root) else { return };
    let dev = meta.dev() as i32;
    let (tx, rx) = std::sync::mpsc::channel::<(PathBuf, Listing)>();
    let root = root.to_path_buf();
    let cancel2 = cancel.clone();
    let producer = std::thread::spawn(move || {
        crate::sys::background_priority();
        walk_dir(root, dev, &cancel2, prune, &tx)
    });
    for (dir, listing) in rx {
        for e in listing.entries.iter().filter(|e| e.kind == Kind::File) {
            visit(&dir, listing.name(e), e.size, e.len);
        }
    }
    let _ = producer.join();
}

fn walk_dir(dir: PathBuf, dev: i32, cancel: &Cancel, prune: fn(&Path) -> bool, tx: &std::sync::mpsc::Sender<(PathBuf, Listing)>) {
    if cancel.is_cancelled() {
        return;
    }
    let Ok(listing) = read_entries(&dir) else { return };
    let subdirs: Vec<PathBuf> = listing
        .entries
        .iter()
        .filter(|e| e.kind == Kind::Dir && e.dev == dev)
        .map(|e| dir.join(listing.name(e)))
        .filter(|p| !prune(p))
        .collect();
    let _ = tx.send((dir, listing));
    subdirs.into_par_iter().for_each_with(tx.clone(), |tx, d| walk_dir(d, dev, cancel, prune, tx));
}

/// Paths the cleaner must never remove, even if a rule or the user asks.
pub fn is_protected(path: &Path) -> bool {
    let home = home();
    let exact: [PathBuf; 14] = [
        "/".into(),
        "/System".into(),
        "/Library".into(),
        "/Applications".into(),
        "/Users".into(),
        "/usr".into(),
        "/bin".into(),
        "/sbin".into(),
        "/private".into(),
        home.clone(),
        home.join("Library"),
        home.join("Documents"),
        home.join("Desktop"),
        home.join("Library/Caches"),
    ];
    if exact.iter().any(|p| p == path) {
        return true;
    }
    path.starts_with("/System") || path.starts_with("/usr") || path.starts_with("/bin") || path.starts_with("/sbin")
}

/// Prune rule for user-facing searches: caches, VCS internals, dependency
/// trees and opaque libraries that users don't manage file-by-file.
pub fn prune_user_noise(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    matches!(name, ".Trash" | "node_modules" | ".git" | "Caches" | ".cache" | ".npm" | ".cargo" | ".rustup" | "DerivedData")
        || path.ends_with("Library/Containers")
        || path.ends_with("Library/Group Containers")
        || path.ends_with("Library/Mail")
        || path.ends_with("Library/Photos")
        || name.ends_with(".photoslibrary")
        || name.ends_with(".app")
}

pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1000.0 && i < UNITS.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}
