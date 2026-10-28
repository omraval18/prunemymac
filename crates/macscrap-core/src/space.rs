//! Space Lens: a size-sorted tree of a folder or the whole Mac.
//!
//! Speed: directories are read with `listing::read_entries` (one
//! `getattrlistbulk` call per batch of entries) and scanned in parallel.
//!
//! Memory: every directory is kept, but only its largest files are kept by
//! name (`TOP_FILES`, plus anything ≥ `NOTABLE_FILE`); the rest are folded
//! into one summary entry. A `node_modules` with 40k files costs one line.

use crate::fsutil::Cancel;
use rayon::prelude::*;
use crate::listing::{read_entries, Entry, Kind};
use std::ffi::CString;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Files always listed by name per directory…
pub const TOP_FILES: usize = 30;
/// …plus any file at least this big, up to `MAX_FILES`.
pub const NOTABLE_FILE: u64 = 10_000_000;
pub const MAX_FILES: usize = 100;

#[derive(Debug, Clone)]
pub struct Leaf {
    pub name: Box<str>,
    pub size: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Dir {
    pub name: Box<str>,
    /// Allocated bytes of everything below.
    pub size: u64,
    /// Number of files below (recursive).
    pub files: u64,
    /// Subdirectories, largest first.
    pub dirs: Vec<Dir>,
    /// Largest files directly in this directory, largest first.
    pub top_files: Vec<Leaf>,
    /// Files directly in this directory not listed in `top_files`.
    pub other_count: u64,
    pub other_size: u64,
    /// Couldn't be read (usually needs Full Disk Access).
    pub unreadable: bool,
}

impl Dir {
    pub fn direct_files(&self) -> u64 {
        self.top_files.len() as u64 + self.other_count
    }

    /// Follow `chain` (indices into `dirs`) from this node.
    pub fn at(&self, chain: &[usize]) -> Option<&Dir> {
        chain.iter().try_fold(self, |d, &i| d.dirs.get(i))
    }

    /// Remove the entry at `rel` (path components below this node) and
    /// subtract its size/files from every ancestor. Returns what was removed.
    pub fn remove(&mut self, rel: &[&str]) -> Option<(u64, u64)> {
        let (first, rest) = rel.split_first()?;
        let removed = if rest.is_empty() {
            if let Some(i) = self.dirs.iter().position(|d| &*d.name == *first) {
                let d = self.dirs.remove(i);
                Some((d.size, d.files))
            } else if let Some(i) = self.top_files.iter().position(|f| &*f.name == *first) {
                let f = self.top_files.remove(i);
                Some((f.size, 1))
            } else {
                None
            }
        } else {
            self.dirs.iter_mut().find(|d| &*d.name == *first)?.remove(rest)
        }?;
        self.size = self.size.saturating_sub(removed.0);
        self.files = self.files.saturating_sub(removed.1);
        Some(removed)
    }

    /// Number of directories in the tree (for diagnostics).
    pub fn dir_count(&self) -> usize {
        1 + self.dirs.iter().map(Dir::dir_count).sum::<usize>()
    }
}

fn scan_dir(path: &Path, name: Box<str>, dev: i32, cancel: &Cancel, files_seen: &AtomicU64) -> Dir {
    let mut dir = Dir { name, ..Default::default() };
    if cancel.is_cancelled() {
        return dir;
    }
    let Ok(listing) = read_entries(path) else {
        dir.unreadable = true;
        return dir;
    };
    let mut files: Vec<&Entry> = Vec::new();
    let mut subdirs: Vec<Box<str>> = Vec::new();
    for e in &listing.entries {
        match e.kind {
            Kind::File => files.push(e),
            // Don't cross into other volumes (and firmlinks to them).
            Kind::Dir if e.dev == dev => subdirs.push(listing.name(e).into()),
            _ => {}
        }
    }
    files_seen.fetch_add(files.len() as u64, Ordering::Relaxed);

    let mut dirs: Vec<Dir> = subdirs
        .into_par_iter()
        .map(|n| {
            let child = path.join(&*n);
            scan_dir(&child, n, dev, cancel, files_seen)
        })
        .collect();
    dirs.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    dirs.shrink_to_fit();

    files.sort_unstable_by(|a, b| b.size.cmp(&a.size));
    let file_total: u64 = files.iter().map(|f| f.size).sum();
    let file_count = files.len() as u64;
    let keep = files.iter().enumerate().take_while(|(i, f)| *i < TOP_FILES || f.size >= NOTABLE_FILE).count().min(MAX_FILES);
    let rest = &files[keep..];
    dir.other_count = rest.len() as u64;
    dir.other_size = rest.iter().map(|f| f.size).sum();
    let files: Vec<Leaf> = files[..keep].iter().map(|e| Leaf { name: listing.name(e).into(), size: e.size }).collect();

    dir.size = file_total + dirs.iter().map(|d| d.size).sum::<u64>();
    dir.files = file_count + dirs.iter().map(|d| d.files).sum::<u64>();
    dir.top_files = files;
    dir.dirs = dirs;
    dir
}

/// Build the size tree for `root`. `files_seen` counts files for progress.
pub fn scan(root: &Path, cancel: &Cancel, files_seen: &AtomicU64) -> Dir {
    let dev = std::fs::metadata(root).map(|m| m.dev() as i32).unwrap_or(0);
    let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
    scan_dir(root, name.into(), dev, cancel, files_seen)
}

/// Where user data lives on APFS. Scanning `/` stops at the read-only
/// system volume; the Data volume holds Users, Applications, Library, etc.
pub fn whole_mac_root() -> PathBuf {
    let data = PathBuf::from("/System/Volumes/Data");
    if data.exists() {
        data
    } else {
        PathBuf::from("/")
    }
}

/// Bytes used on the startup disk's APFS container (all volumes).
pub fn disk_used() -> u64 {
    let Ok(cpath) = CString::new("/") else { return 0 };
    let mut st: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(cpath.as_ptr(), &mut st) } != 0 {
        return 0;
    }
    (st.f_blocks - st.f_bfree) * st.f_bsize as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::listing::{read_entries, read_std, Listing};

    #[test]
    fn bulk_matches_std() {
        let dir = std::env::temp_dir().join(format!("macscrap-space-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a/b")).unwrap();
        std::fs::write(dir.join("one.bin"), vec![1u8; 100_000]).unwrap();
        std::fs::write(dir.join("a/two.bin"), vec![2u8; 300_000]).unwrap();
        std::fs::write(dir.join("a/b/three.bin"), vec![3u8; 50_000]).unwrap();
        std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();

        let bulk = read_entries(&dir).unwrap();
        let std_ = read_std(&dir).unwrap();
        let flat = |l: &Listing| {
            let mut v: Vec<(String, bool, u64, u64)> = l
                .entries
                .iter()
                .map(|e| (l.name(e).to_string(), e.kind == Kind::Dir, if e.kind == Kind::File { e.size } else { 0 }, e.len))
                .collect();
            v.sort();
            v
        };
        assert_eq!(flat(&bulk), flat(&std_));
        assert_eq!(bulk.entries.len(), 3); // one.bin, a, link

        let tree = scan(&dir, &Cancel::new(), &AtomicU64::new(0));
        assert_eq!(tree.files, 3);
        assert_eq!(tree.dirs[0].name.as_ref(), "a");
        assert!(tree.dirs[0].size >= 350_000);

        let mut tree = tree;
        let before = tree.size;
        let (removed, n) = tree.remove(&["a", "two.bin"]).unwrap();
        assert_eq!(n, 1);
        assert_eq!(tree.size, before - removed);
        assert_eq!(tree.files, 2);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
