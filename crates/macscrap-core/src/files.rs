//! Large/old file finder and duplicate finder.

use crate::fsutil::{self, Cancel};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
}

impl FileEntry {
    /// Days since the file was last opened or modified, whichever is newer.
    pub fn idle_days(&self) -> Option<u64> {
        let last = self.modified.max(self.accessed)?;
        Some(SystemTime::now().duration_since(last).ok()?.as_secs() / 86_400)
    }
}

/// Collect every regular file under `root` of at least `min_size` bytes.
/// `scanned` is bumped per file so the UI can show live progress.
pub fn collect(root: &Path, min_size: u64, cancel: &Cancel, scanned: &AtomicU64) -> Vec<FileEntry> {
    let mut out = Vec::new();
    fsutil::walk(root, cancel, fsutil::prune_user_noise, &mut |dir, name, _allocated, len| {
        scanned.fetch_add(1, Ordering::Relaxed);
        if len < min_size {
            return;
        }
        // Only files that pass the size filter pay for a stat (for dates).
        let path = dir.join(name);
        let Ok(m) = std::fs::symlink_metadata(&path) else { return };
        out.push(FileEntry { size: len, modified: m.modified().ok(), accessed: m.accessed().ok(), path });
    });
    out.sort_by(|a, b| b.size.cmp(&a.size));
    out
}

pub fn large_files(root: &Path, min_size: u64, cancel: &Cancel, scanned: &AtomicU64) -> Vec<FileEntry> {
    collect(root, min_size, cancel, scanned)
}

#[derive(Debug, Clone)]
pub struct DuplicateSet {
    pub size: u64,
    pub files: Vec<FileEntry>,
}

impl DuplicateSet {
    /// Space recovered by keeping one copy.
    pub fn wasted(&self) -> u64 {
        self.size * (self.files.len() as u64 - 1)
    }
}

fn hash_prefix(path: &Path) -> Option<u64> {
    let mut buf = [0u8; 16 * 1024];
    let mut f = File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    Some(xxhash_rust::xxh3::xxh3_64(&buf[..n]))
}

fn hash_full(path: &Path) -> Option<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    h.update_mmap(path).ok()?;
    Some(*h.finalize().as_bytes())
}

/// Exact duplicates: group by size → 16 KB prefix hash → full BLAKE3.
/// Hard links to the same inode are not reported as duplicates.
pub fn duplicates(root: &Path, min_size: u64, cancel: &Cancel, scanned: &AtomicU64) -> Vec<DuplicateSet> {
    let files = collect(root, min_size.max(1), cancel, scanned);

    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    let mut seen_inodes = std::collections::HashSet::new();
    for f in files {
        let Ok(m) = std::fs::metadata(&f.path) else { continue };
        if !seen_inodes.insert((m.dev(), m.ino())) {
            continue;
        }
        by_size.entry(m.len()).or_default().push(f);
    }

    let refine = |groups: Vec<Vec<FileEntry>>, key: &(dyn Fn(&Path) -> Option<Vec<u8>> + Sync)| {
        groups
            .into_par_iter()
            .flat_map_iter(|group| {
                let mut buckets: HashMap<Vec<u8>, Vec<FileEntry>> = HashMap::new();
                for f in group {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Some(k) = key(&f.path) {
                        buckets.entry(k).or_default().push(f);
                    }
                }
                buckets.into_values().filter(|b| b.len() > 1)
            })
            .collect::<Vec<_>>()
    };

    let candidates: Vec<Vec<FileEntry>> = by_size.into_values().filter(|g| g.len() > 1).collect();
    let candidates = refine(candidates, &|p| hash_prefix(p).map(|h| h.to_le_bytes().to_vec()));
    let confirmed = refine(candidates, &|p| hash_full(p).map(|h| h.to_vec()));

    let mut sets: Vec<DuplicateSet> = confirmed
        .into_iter()
        .map(|mut files| {
            // Oldest copy first: it's the one we suggest keeping.
            files.sort_by_key(|f| f.modified);
            let size = std::fs::metadata(&files[0].path).map(|m| m.len()).unwrap_or(0);
            DuplicateSet { size, files }
        })
        .collect();
    sets.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    sets
}
