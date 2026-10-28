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
    #[cfg(target_os = "linux")]
    {
        // Cleanup and Trash actions never remove system files or escape HOME.
        if !path.is_absolute() || !path.starts_with(&home)
            || path.components().any(|part| matches!(part, std::path::Component::ParentDir)) { return true; }
        let Ok(root) = home.canonicalize() else { return true };
        let Ok(resolved) = path.canonicalize() else { return true };
        if !resolved.starts_with(&root) { return true; }
        let protected = [root.clone(), root.join("Documents"), root.join("Desktop"), root.join("Downloads"),
            root.join(".config"), root.join(".local"), root.join(".local/share"), root.join(".cache")];
        return protected.iter().any(|p| *p == resolved);
    }
    #[cfg(target_os = "macos")]
    {
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
        || is_browser_profile_dir(path)
}

/// Browser profile / app-state trees. Duplicate scans should not suggest
/// deleting copies inside these — they are live application data.
pub fn is_browser_profile_dir(path: &Path) -> bool {
    let home = home();
    let known = [
        home.join(".config/google-chrome"),
        home.join(".config/google-chrome-headless"),
        home.join(".config/chromium"),
        home.join(".config/chromium-headless"),
        home.join(".config/BraveSoftware"),
        home.join(".config/microsoft-edge"),
        home.join(".config/opera"),
        home.join(".config/vivaldi"),
        home.join(".mozilla"),
        home.join(".config/mozilla"),
        home.join("Library/Application Support/Google/Chrome"),
        home.join("Library/Application Support/Chromium"),
        home.join("Library/Application Support/BraveSoftware"),
        home.join("Library/Application Support/Microsoft Edge"),
        home.join("Library/Application Support/Firefox"),
    ];
    if known.iter().any(|root| path == root || path.starts_with(root)) {
        return true;
    }
    // Non-standard locations (e.g. ~/chrome-profile/Fork-*, custom --user-data-dir):
    // treat a directory (or anything under it) as a profile when it has Chromium /
    // Firefox marker files, optionally reinforced by a browser-like name.
    path.ancestors().take(12).any(|dir| {
        if dir == Path::new("/") || dir == Path::new("") {
            return false;
        }
        let name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
        let named = browser_like_name(&name);
        let marked = has_browser_profile_markers(dir);
        marked || (named && (dir.join("Default").is_dir() || dir.join("Profile 1").is_dir()))
    })
}

fn browser_like_name(name: &str) -> bool {
    name.contains("chrome")
        || name.contains("chromium")
        || name.contains("brave")
        || name.contains("firefox")
        || name.contains("mozilla")
        || name.contains("vivaldi")
        || name.contains("opera")
        || name.contains("msedge")
        || name.contains("microsoft-edge")
        || name.contains("headless")
        || name.ends_with("-headless")
}

/// Chromium / Firefox user-data markers at the root of a profile tree.
fn has_browser_profile_markers(dir: &Path) -> bool {
    dir.join("Local State").is_file()
        || dir.join("First Run").exists()
        || dir.join("Default").join("Preferences").is_file()
        || dir.join("profiles.ini").is_file()
        || dir.join("prefs.js").is_file()
        || dir.join("Default").join("prefs.js").is_file()
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

#[cfg(all(test, target_os = "linux"))]
mod linux_protection_tests {
    use super::*;

    #[test]
    fn cleanup_is_confined_to_home_and_cannot_follow_escape_links() {
        let home = home();
        let dir = home.join(format!(".prunemymac-protection-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("file");
        std::fs::write(&file, b"test").unwrap();
        let link = dir.join("outside");
        std::os::unix::fs::symlink("/tmp", &link).unwrap();
        assert!(!is_protected(&file));
        assert!(is_protected(&home));
        assert!(is_protected(Path::new("/etc")));
        assert!(is_protected(&link));
        assert!(is_protected(&dir.join("..").join("..").join("etc")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn browser_profile_dirs_are_pruned_from_user_searches() {
        let chrome = home().join(".config/google-chrome");
        let profile = chrome.join("Default/Preferences");
        assert!(is_browser_profile_dir(&chrome));
        assert!(is_browser_profile_dir(&profile));
        assert!(prune_user_noise(&chrome));
        assert!(!is_browser_profile_dir(&home().join("Documents")));
    }

    #[test]
    fn detects_nonstandard_chrome_user_data_dirs_by_markers_and_name() {
        let root = home().join(format!(".prunemymac-chrome-profile-test-{}", std::process::id()));
        let fork = root.join("Fork-1");
        let headless = home().join(format!(".prunemymac-chrome-headless-test-{}", std::process::id()));
        let innocent = home().join(format!(".prunemymac-notes-test-{}", std::process::id()));
        std::fs::create_dir_all(fork.join("Default")).unwrap();
        std::fs::write(fork.join("Local State"), b"{}").unwrap();
        std::fs::write(fork.join("Default/Preferences"), b"{}").unwrap();
        std::fs::create_dir_all(headless.join("Default")).unwrap();
        std::fs::write(headless.join("Local State"), b"{}").unwrap();
        std::fs::create_dir_all(&innocent).unwrap();
        std::fs::write(innocent.join("readme.txt"), b"hi").unwrap();

        assert!(is_browser_profile_dir(&fork));
        assert!(is_browser_profile_dir(&fork.join("Default/Preferences")));
        assert!(prune_user_noise(&fork));
        assert!(is_browser_profile_dir(&headless));
        assert!(prune_user_noise(&headless));
        // A normal folder must not be treated as a browser profile.
        assert!(!is_browser_profile_dir(&innocent));
        assert!(!prune_user_noise(&innocent));
        // Browser-like name alone without markers / Default is not enough.
        let named_only = home().join(format!(".prunemymac-chrome-named-only-{}", std::process::id()));
        std::fs::create_dir_all(&named_only).unwrap();
        assert!(!is_browser_profile_dir(&named_only));

        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&headless).ok();
        std::fs::remove_dir_all(&innocent).ok();
        std::fs::remove_dir_all(&named_only).ok();
    }
}
