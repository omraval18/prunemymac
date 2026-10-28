//! The only module that removes anything. Every removal goes through
//! `remove`, which enforces the protected-path guard.

use crate::fsutil;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Unrecoverable; frees space immediately.
    Delete,
    /// Move to the user's Trash via NSFileManager.
    Trash,
}

#[derive(Debug, Default, Clone)]
pub struct CleanReport {
    pub removed: usize,
    pub freed: u64,
    pub failures: Vec<(PathBuf, String)>,
}

pub fn remove(path: &Path, method: Method) -> Result<(), String> {
    remove_tracked(path, method, &Progress::default())
}

/// `remove`, reporting bytes freed and files removed as it goes, so a single
/// huge folder still shows steady progress.
pub fn remove_tracked(path: &Path, method: Method, progress: &Progress) -> Result<(), String> {
    if fsutil::is_protected(path) {
        return Err("protected path".into());
    }
    if std::fs::symlink_metadata(path).is_err() {
        return Ok(()); // already gone
    }
    match method {
        Method::Delete => {
            let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
            if meta.is_dir() {
                delete_tree(path, progress);
                // Anything left (locked or in-use files) makes this a failure.
                if path.exists() {
                    return Err("some files are in use or protected".into());
                }
                #[cfg(target_os = "linux")]
                remove_trash_info(path);
                Ok(())
            } else {
                std::fs::remove_file(path).map_err(|e| e.to_string())?;
                progress.freed.fetch_add(fsutil::allocated(&meta), Ordering::Relaxed);
                progress.files.fetch_add(1, Ordering::Relaxed);
                #[cfg(target_os = "linux")]
                remove_trash_info(path);
                Ok(())
            }
        }
        Method::Trash => {
            #[cfg(target_os = "macos")]
            {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            let mut ctx = trash::TrashContext::default();
            ctx.set_delete_method(DeleteMethod::NsFileManager);
            ctx.delete(path).map_err(|e| e.to_string())
            }
            #[cfg(target_os = "linux")]
            { trash::delete(path).map_err(|e| e.to_string()) }
        }
    }
}

// Keep a Trash entry's restore information until its file has actually gone.
#[cfg(target_os = "linux")]
fn remove_trash_info(path: &Path) {
    let trash = fsutil::home().join(".local/share/Trash");
    if path.parent() != Some(trash.join("files").as_path()) { return; }
    let Some(name) = path.file_name() else { return };
    let mut info_name = name.to_os_string();
    info_name.push(".trashinfo");
    let info = trash.join("info").join(info_name);
    if !fsutil::is_protected(&info) { let _ = std::fs::remove_file(info); }
}

/// Live progress of a `run_tracked` call, readable from the UI thread.
#[derive(Default)]
pub struct Progress {
    /// Top-level items finished.
    pub done: AtomicUsize,
    /// Bytes freed so far (updated per file).
    pub freed: AtomicU64,
    /// Individual files removed so far.
    pub files: AtomicU64,
    pub current: Mutex<String>,
}

/// Delete a directory tree bottom-up, files in parallel, counting each file.
/// Symlinks are unlinked, never followed. Errors are skipped so one locked
/// file doesn't stop the rest; the caller checks what remains.
fn delete_tree(dir: &Path, progress: &Progress) {
    use rayon::prelude::*;
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            dirs.push(entry.path());
        } else {
            files.push((entry.path(), fsutil::allocated(&meta)));
        }
    }
    files.par_iter().for_each(|(p, size)| {
        if std::fs::remove_file(p).is_ok() {
            progress.freed.fetch_add(*size, Ordering::Relaxed);
            progress.files.fetch_add(1, Ordering::Relaxed);
        }
    });
    dirs.par_iter().for_each(|d| delete_tree(d, progress));
    let _ = std::fs::remove_dir(dir);
}

/// Remove `(path, size, method)` entries, collecting failures instead of
/// stopping at the first one.
pub fn run(items: impl IntoIterator<Item = (PathBuf, u64, Method)>) -> CleanReport {
    run_tracked(items, &Progress::default())
}

/// Like `run`, publishing progress after every item.
pub fn run_tracked(items: impl IntoIterator<Item = (PathBuf, u64, Method)>, progress: &Progress) -> CleanReport {
    let mut report = CleanReport::default();
    for (path, size, method) in items {
        if let Ok(mut cur) = progress.current.lock() {
            *cur = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        }
        let before = progress.freed.load(Ordering::Relaxed);
        let result = remove_tracked(&path, method, progress);
        // Trash moves are one step: count the whole item when it succeeds.
        let counted = progress.freed.load(Ordering::Relaxed) - before;
        match result {
            Ok(()) => {
                report.removed += 1;
                report.freed += size.max(counted);
                progress.freed.fetch_add(size.saturating_sub(counted), Ordering::Relaxed);
            }
            Err(e) => {
                report.freed += counted;
                report.failures.push((path, e));
            }
        }
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn emptying_trash_removes_only_the_matching_metadata() {
        let trash = fsutil::home().join(".local/share/Trash");
        let files = trash.join("files");
        let info = trash.join("info");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::create_dir_all(&info).unwrap();
        let name = format!("prunemymac-test-{}", std::process::id());
        let file = files.join(&name);
        let metadata = info.join(format!("{name}.trashinfo"));
        std::fs::write(&file, b"test").unwrap();
        std::fs::write(&metadata, b"test").unwrap();
        remove(&file, Method::Delete).unwrap();
        assert!(!file.exists());
        assert!(!metadata.exists());
    }

    #[test]
    fn tracked_delete_reports_every_file() {
        #[cfg(target_os = "macos")]
        let dir = std::env::temp_dir().join(format!("macscrap-clean-{}", std::process::id()));
        #[cfg(target_os = "linux")]
        let dir = fsutil::home().join(format!(".prunemymac-clean-test-{}", std::process::id()));
        for i in 0..20 {
            let sub = dir.join(format!("d{}", i % 4));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join(format!("f{i}")), vec![0u8; 8192]).unwrap();
        }
        std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
        let progress = Progress::default();
        let report = run_tracked([(dir.clone(), 0, Method::Delete)], &progress);
        assert!(!dir.exists());
        assert!(std::path::Path::new("/etc").exists(), "symlink target must survive");
        assert_eq!(report.removed, 1);
        assert_eq!(progress.files.load(Ordering::Relaxed), 21);
        assert!(progress.freed.load(Ordering::Relaxed) >= 20 * 8192);
    }
}
