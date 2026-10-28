//! Large/old file finder and duplicate finder.

/// Shared floor for large-file scans and the default Overview / sidebar threshold.
pub const LARGE_FILE_MIN: u64 = 100_000_000;

use crate::fsutil::{self, Cancel};
use rayon::prelude::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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

const PARTIAL_CHUNK: u64 = 16 * 1024;

/// Fast reject: xxh3 of the first and (when large enough) last 16 KiB.
pub(crate) fn hash_partial(path: &Path, size: u64) -> Option<u64> {
    let mut buf = [0u8; PARTIAL_CHUNK as usize];
    let mut f = File::open(path).ok()?;
    let n = f.read(&mut buf).ok()?;
    let first = xxhash_rust::xxh3::xxh3_64(&buf[..n]);
    if size <= PARTIAL_CHUNK || (n as u64) < PARTIAL_CHUNK {
        return Some(first);
    }
    f.seek(SeekFrom::End(-(PARTIAL_CHUNK as i64))).ok()?;
    let n2 = f.read(&mut buf).ok()?;
    let last = xxhash_rust::xxh3::xxh3_64(&buf[..n2]);
    let mut mix = [0u8; 16];
    mix[..8].copy_from_slice(&first.to_le_bytes());
    mix[8..].copy_from_slice(&last.to_le_bytes());
    Some(xxhash_rust::xxh3::xxh3_64(&mix))
}

fn hash_full(path: &Path) -> Option<[u8; 32]> {
    let mut h = blake3::Hasher::new();
    // Prefer mmap; fall back to a bounded buffered read so huge files never
    // need an unbounded Vec.
    match h.update_mmap(path) {
        Ok(_) => {}
        Err(_) => {
            let mut f = File::open(path).ok()?;
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = f.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
        }
    }
    Some(*h.finalize().as_bytes())
}

/// Exact duplicates: group by size → first/last 16 KB xxh3 → full BLAKE3.
/// Hard links to the same inode are not reported as duplicates.
/// Full hashing runs on a small dedicated pool so we don't thrash the disk.
pub fn duplicates(root: &Path, min_size: u64, cancel: &Cancel, scanned: &AtomicU64) -> Vec<DuplicateSet> {
    let min_size = min_size.max(1);
    let mut by_size: HashMap<u64, Vec<FileEntry>> = HashMap::new();
    let mut seen_inodes = std::collections::HashSet::new();

    // Single walk + one metadata call per candidate (size/inode/dates).
    fsutil::walk(root, cancel, fsutil::prune_user_noise, &mut |dir, name, _allocated, len| {
        scanned.fetch_add(1, Ordering::Relaxed);
        if len < min_size {
            return;
        }
        let path = dir.join(name);
        let Ok(m) = std::fs::symlink_metadata(&path) else { return };
        if !m.is_file() {
            return;
        }
        if !seen_inodes.insert((m.dev(), m.ino())) {
            return;
        }
        by_size.entry(len).or_default().push(FileEntry {
            size: len,
            modified: m.modified().ok(),
            accessed: m.accessed().ok(),
            path,
        });
    });

    let refine = |groups: Vec<Vec<FileEntry>>, key: &(dyn Fn(&FileEntry) -> Option<Vec<u8>> + Sync)| {
        groups
            .into_par_iter()
            .flat_map_iter(|group| {
                let mut buckets: HashMap<Vec<u8>, Vec<FileEntry>> = HashMap::new();
                for f in group {
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Some(k) = key(&f) {
                        buckets.entry(k).or_default().push(f);
                    }
                }
                buckets.into_values().filter(|b| b.len() > 1)
            })
            .collect::<Vec<_>>()
    };

    let candidates: Vec<Vec<FileEntry>> = by_size.into_values().filter(|g| g.len() > 1).collect();
    let candidates = refine(candidates, &|f| hash_partial(&f.path, f.size).map(|h| h.to_le_bytes().to_vec()));

    // Bound concurrent full-file reads: 2 keep SSDs busy without queueing the page cache.
    let hash_threads = std::thread::available_parallelism().map(|n| n.get().clamp(1, 4)).unwrap_or(4);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(hash_threads)
        .thread_name(|i| format!("macscrap-hash-{i}"))
        .build()
        .expect("hash pool");
    let confirmed = pool.install(|| refine(candidates, &|f| hash_full(&f.path).map(|h| h.to_vec())));

    let mut sets: Vec<DuplicateSet> = confirmed
        .into_iter()
        .map(|mut files| {
            // Oldest copy first: it's the one we suggest keeping.
            files.sort_by_key(|f| f.modified);
            let size = files.first().map(|f| f.size).unwrap_or(0);
            DuplicateSet { size, files }
        })
        .collect();
    sets.sort_by(|a, b| b.wasted().cmp(&a.wasted()));
    sets
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn partial_hash_differs_when_only_tail_changes() {
        let dir = std::env::temp_dir().join(format!("prunemymac-dup-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let mut body = vec![0u8; 64 * 1024];
        for (i, x) in body.iter_mut().enumerate() {
            *x = (i % 251) as u8;
        }
        std::fs::write(&a, &body).unwrap();
        let last = body.len() - 1;
        body[last] ^= 0xff;
        std::fs::write(&b, &body).unwrap();
        let ha = hash_partial(&a, 64 * 1024).unwrap();
        let hb = hash_partial(&b, 64 * 1024).unwrap();
        assert_ne!(ha, hb, "tail-only change must change partial hash");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn duplicates_find_identical_copies_and_ignore_unique() {
        let dir = std::env::temp_dir().join(format!("prunemymac-dup-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let payload = vec![0x5au8; 1_500_000];
        std::fs::write(dir.join("one.dat"), &payload).unwrap();
        std::fs::write(dir.join("two.dat"), &payload).unwrap();
        let mut other = payload.clone();
        other[0] = 0x11;
        std::fs::write(dir.join("other.dat"), &other).unwrap();
        let sets = duplicates(&dir, 1_000_000, &Cancel::new(), &AtomicU64::new(0));
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].files.len(), 2);
        assert_eq!(sets[0].size, 1_500_000);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn partial_hash_small_file_is_stable() {
        let dir = std::env::temp_dir().join(format!("prunemymac-dup-small-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tiny.bin");
        let mut f = File::create(&p).unwrap();
        f.write_all(b"hello-prunemymac").unwrap();
        let h1 = hash_partial(&p, 16).unwrap();
        let h2 = hash_partial(&p, 16).unwrap();
        assert_eq!(h1, h2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
