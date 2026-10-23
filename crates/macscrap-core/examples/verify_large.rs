//! Cross-check files::large_files against a plain std walk with the same pruning.
use macscrap_core::{files, fsutil, Cancel};
use std::collections::BTreeSet;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

fn std_walk(dir: &Path, dev: u64, min: u64, out: &mut BTreeSet<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(m) = std::fs::symlink_metadata(&p) else { continue };
        if m.dev() != dev { continue; }
        if m.is_dir() {
            if !fsutil::prune_user_noise(&p) { std_walk(&p, dev, min, out); }
        } else if m.is_file() && m.len() >= min {
            out.insert(p);
        }
    }
}

fn main() {
    let home = fsutil::home();
    let min = 100_000_000;
    let fast: BTreeSet<PathBuf> = files::large_files(&home, min, &Cancel::new(), &Default::default()).into_iter().map(|f| f.path).collect();
    let mut slow = BTreeSet::new();
    std_walk(&home, std::fs::metadata(&home).unwrap().dev(), min, &mut slow);
    println!("fast {} std {}", fast.len(), slow.len());
    for p in slow.difference(&fast).take(10) { println!("  missing: {}", p.display()); }
    for p in fast.difference(&slow).take(10) { println!("  extra:   {}", p.display()); }
}
