//! Leak check: run every scanner repeatedly and track RSS and open file
//! descriptors. After a warm-up round, both should stay flat.
use macscrap_core::*;
use std::sync::atomic::AtomicU64;

fn rss_mb() -> f64 {
    let out = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse::<f64>().unwrap_or(0.0) / 1024.0
}
fn fds() -> usize {
    std::fs::read_dir("/dev/fd").map(|d| d.count()).unwrap_or(0)
}

fn phase(name: &str, rounds: usize, mut f: impl FnMut()) {
    f(); // warm-up: caches, thread pool, lazy statics
    let (r0, f0) = (rss_mb(), fds());
    for _ in 0..rounds {
        f();
    }
    let (r1, f1) = (rss_mb(), fds());
    println!("{name:<22} x{rounds:<4} rss {r0:>6.1} -> {r1:>6.1} MB ({:+.1})   fds {f0} -> {f1}", r1 - r0);
}

fn main() {
    let c = Cancel::new();
    let mut mon = health::Monitor::new();
    phase("health sample", 3000, || drop(mon.sample()));
    let rules = junk::load_rules().unwrap();
    phase("junk scan", 30, || drop(junk::scan(&rules, &c, &|_| {})));
    phase("apps list", 15, || drop(apps::list(&c)));
    let apps = apps::list(&c);
    phase("leftovers (all apps)", 5, || for a in &apps { drop(apps::leftovers(a, &c)) });
    phase("launch items", 10, || drop(background::launch_items()));
    phase("heavy processes", 10, || drop(background::heavy_processes(15)));
    phase("large files (home)", 4, || drop(files::large_files(&fsutil::home(), 100_000_000, &c, &AtomicU64::new(0))));
    phase("duplicates (Downloads)", 4, || drop(files::duplicates(&fsutil::home().join("Downloads"), 1_000_000, &c, &AtomicU64::new(0))));
    phase("space lens (home)", 3, || drop(space::scan(&fsutil::home(), &c, &AtomicU64::new(0))));
    phase("malware engine+scan", 3, || {
        let e = malware::Engine::load().unwrap();
        drop(e.scan(&[fsutil::home().join("Downloads")], &c, &AtomicU64::new(0)));
    });
    let tmp = std::path::PathBuf::from(std::env::args().nth(1).expect("scratch dir"));
    phase("clean (create+delete)", 20, || {
        let d = tmp.join("stress-tree");
        for i in 0..200 {
            let sub = d.join(format!("d{}", i % 10));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join(format!("f{i}")), b"x").unwrap();
        }
        let r = clean::run_tracked([(d, 0, clean::Method::Delete)], &clean::Progress::default());
        assert_eq!(r.removed, 1);
    });
}
