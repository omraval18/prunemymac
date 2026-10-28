//! Read-only Linux scanner timing + peak RSS + disk bytes read.
//! Usage: linux_perf [cold|warm]  (default warm). Prints TSV-friendly lines.
use macscrap_core::*;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

fn rss_kb() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("VmRSS:") {
            return v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    0
}
fn hwm_kb() -> u64 {
    let s = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("VmHWM:") {
            return v.split_whitespace().next().and_then(|n| n.parse().ok()).unwrap_or(0);
        }
    }
    0
}
fn io_read_bytes() -> u64 {
    let s = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("read_bytes:") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}
fn io_rchar() -> u64 {
    let s = std::fs::read_to_string("/proc/self/io").unwrap_or_default();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("rchar:") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn mb(b: u64) -> f64 { b as f64 / (1024.0 * 1024.0) }
fn kb_to_mb(kb: u64) -> f64 { kb as f64 / 1024.0 }

struct Sample {
    name: &'static str,
    secs: f64,
    peak_rss_mb: f64,
    delta_rss_mb: f64,
    read_mb: f64,
    rchar_mb: f64,
    detail: String,
}

fn run<T>(name: &'static str, f: impl FnOnce() -> (T, String)) -> Sample {
    // Force a sync point so HWM from prior phases doesn't confuse "delta"
    let before_rss = rss_kb();
    let before_hwm = hwm_kb();
    let before_read = io_read_bytes();
    let before_rchar = io_rchar();
    let t0 = Instant::now();
    let (_out, detail) = f();
    let secs = t0.elapsed().as_secs_f64();
    let after_hwm = hwm_kb();
    let after_rss = rss_kb();
    let peak = after_hwm.max(before_hwm);
    Sample {
        name,
        secs,
        peak_rss_mb: kb_to_mb(peak),
        delta_rss_mb: kb_to_mb(after_rss.saturating_sub(before_rss.min(after_rss)).max(after_rss.saturating_sub(before_rss))),
        read_mb: mb(io_read_bytes().saturating_sub(before_read)),
        rchar_mb: mb(io_rchar().saturating_sub(before_rchar)),
        detail,
    }
}

fn print(s: &Sample) {
    println!(
        "{:<22} {:>8.2}s  peakRSS {:>7.1} MB  ΔRSS {:>+7.1} MB  disk {:>8.1} MB  rchar {:>8.1} MB  {}",
        s.name, s.secs, s.peak_rss_mb, s.delta_rss_mb, s.read_mb, s.rchar_mb, s.detail
    );
}

fn main() {
    let mode = std::env::args().nth(1).unwrap_or_else(|| "warm".into());
    let home = fsutil::home();
    let c = Cancel::new();
    println!("# linux_perf mode={mode} home={} files≈(see space) pid={}", home.display(), std::process::id());
    println!("# start RSS={:.1} MB HWM={:.1} MB", kb_to_mb(rss_kb()), kb_to_mb(hwm_kb()));

    if mode == "cold" {
        // Touch nothing first; OS page cache may still be warm from earlier work.
        println!("# note: not dropping system caches (would need root and hurts others)");
    } else {
        // Warm page cache with a quick home walk via space once
        let _ = space::scan(&home, &c, &AtomicU64::new(0));
        println!("# warmed with one space::scan; RSS={:.1} MB", kb_to_mb(rss_kb()));
    }

    let samples = [
        run("junk/cleanup", || {
            let rules = junk::load_rules().unwrap_or_default();
            let g = junk::scan(&rules, &c, &|_| {});
            let bytes: u64 = g.iter().map(|x| x.size()).sum();
            ((), format!("{} groups, {}", g.len(), human_bytes(bytes)))
        }),
        run("large_files", || {
            let n = AtomicU64::new(0);
            let files = files::large_files(&home, files::LARGE_FILE_MIN, &c, &n);
            ((), format!("{} files ≥100MB, scanned {}", files.len(), n.load(std::sync::atomic::Ordering::Relaxed)))
        }),
        run("duplicates", || {
            let n = AtomicU64::new(0);
            let sets = files::duplicates(&home, 1_000_000, &c, &n);
            let wasted: u64 = sets.iter().map(|s| s.wasted()).sum();
            ((), format!("{} sets, wasted {}, scanned {}", sets.len(), human_bytes(wasted), n.load(std::sync::atomic::Ordering::Relaxed)))
        }),
        run("space_lens", || {
            let n = AtomicU64::new(0);
            let tree = space::scan(&home, &c, &n);
            ((), format!("{} files, {} dirs, {}", tree.files, tree.dir_count(), human_bytes(tree.size)))
        }),
        run("protection", || {
            let n = AtomicU64::new(0);
            match malware::Engine::load() {
                Ok(e) => {
                    let threats = e.scan(&malware::quick_scan_roots(), &c, &n);
                    let extra = malware::audit_persistence();
                    ((), format!("{} threats + {} persistence, scanned {}", threats.len(), extra.len(), n.load(std::sync::atomic::Ordering::Relaxed)))
                }
                Err(err) => ((), format!("engine err: {err}")),
            }
        }),
    ];
    println!("# --- results ---");
    for s in &samples {
        print(s);
    }
    println!("# end RSS={:.1} MB HWM={:.1} MB", kb_to_mb(rss_kb()), kb_to_mb(hwm_kb()));
}
