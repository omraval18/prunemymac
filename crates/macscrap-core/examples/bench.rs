//! Read-only timing of each scanner (best of N). Removes nothing.
use macscrap_core::*;
use std::sync::atomic::AtomicU64;
use std::time::{Duration, Instant};

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best = Duration::MAX;
    let mut out = None;
    for _ in 0..n {
        let t = Instant::now();
        let r = f();
        best = best.min(t.elapsed());
        out = Some(r);
    }
    (best, out.unwrap())
}

fn main() {
    let c = Cancel::new();
    let mut mon = health::Monitor::new();
    let (t, _) = best(10, || mon.sample());
    println!("health sample      {:>8.1?}", t);
    let rules = junk::load_rules().unwrap();
    let (t, g) = best(3, || junk::scan(&rules, &c, &|_| {}));
    println!("junk scan          {:>8.1?}  ({})", t, human_bytes(g.iter().map(|g| g.size()).sum()));
    let (t, a) = best(3, || apps::list(&c));
    println!("apps list          {:>8.1?}  ({} apps)", t, a.len());
    let (t, l) = best(2, || files::large_files(&fsutil::home(), 100_000_000, &c, &AtomicU64::new(0)));
    println!("large files (home) {:>8.1?}  ({} files)", t, l.len());
    let (t, d) = best(2, || files::duplicates(&fsutil::home().join("Downloads"), 1_000_000, &c, &AtomicU64::new(0)));
    println!("dupes (Downloads)  {:>8.1?}  ({} sets)", t, d.len());
    let (t, s) = best(2, || space::scan(&fsutil::home(), &c, &AtomicU64::new(0)));
    println!("space lens (home)  {:>8.1?}  ({} files)", t, s.files);
    let (t, _) = best(3, || background::launch_items());
    println!("launch items       {:>8.1?}", t);
}
