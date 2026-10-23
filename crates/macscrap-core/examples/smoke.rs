//! Read-only end-to-end check of every scanner. Removes nothing.
use macscrap_core::*;
use std::sync::atomic::AtomicU64;
use std::time::Instant;

fn main() {
    let cancel = Cancel::new();
    let t = Instant::now();
    let mut mon = health::Monitor::new();
    std::thread::sleep(std::time::Duration::from_millis(500));
    let s = mon.sample();
    println!("health: {} cpu {:.0}% temp {:?} mem {}/{} pressure {}% disk free {}/{} batt {:?} [{:?}]",
        s.model, s.cpu_load, s.cpu_temp, human_bytes(s.mem_used), human_bytes(s.mem_total), s.mem_pressure,
        human_bytes(s.disk_free), human_bytes(s.disk_total), s.battery, t.elapsed());

    let t = Instant::now();
    let rules = junk::load_rules().unwrap();
    let groups = junk::scan(&rules, &cancel, &|_| {});
    for g in &groups { println!("junk: {:28} {:>10} ({} items)", g.rule.name, human_bytes(g.size()), g.items.len()); }
    println!("junk total {} [{:?}]", human_bytes(groups.iter().map(|g| g.size()).sum()), t.elapsed());

    let t = Instant::now();
    let n = AtomicU64::new(0);
    let large = files::large_files(&fsutil::home(), 200_000_000, &cancel, &n);
    println!("large: {} files >=200MB of {} scanned [{:?}]", large.len(), n.into_inner(), t.elapsed());
    for f in large.iter().take(5) { println!("  {} {} idle {:?}d", human_bytes(f.size), f.path.display(), f.idle_days()); }

    let t = Instant::now();
    let n = AtomicU64::new(0);
    let d = files::duplicates(&fsutil::home().join("Downloads"), 100_000, &cancel, &n);
    println!("dupes in Downloads: {} sets, wasted {} [{:?}]", d.len(), human_bytes(d.iter().map(|s| s.wasted()).sum()), t.elapsed());

    let t = Instant::now();
    let items = background::launch_items();
    println!("launch items: {} ({} broken) [{:?}]", items.len(), items.iter().filter(|i| i.broken).count(), t.elapsed());
    for i in items.iter().take(6) { println!("  {:40} {:14} loaded={} signer={:?}", i.label, i.scope.label(), i.loaded, i.signer); }
    for p in background::heavy_processes(5) { println!("  proc {:30} cpu {:.1}% mem {}", p.name, p.cpu, human_bytes(p.memory)); }

    let t = Instant::now();
    let apps = apps::list(&cancel);
    println!("apps: {} [{:?}]", apps.len(), t.elapsed());
    if let Some(a) = apps.iter().find(|a| a.bundle_id.contains("CleanMyMac")) {
        let t = Instant::now();
        let l = apps::leftovers(a, &cancel);
        println!("leftovers for {} ({}): [{:?}]", a.name, a.bundle_id, t.elapsed());
        for x in &l { println!("  {:24} {:>10} {}", x.kind, human_bytes(x.size), x.path.display()); }
    }

    let t = Instant::now();
    match malware::Engine::load() {
        Ok(e) => {
            println!("yara: {} rules loaded [{:?}] {:?}", e.rule_count, t.elapsed(), e.sources);
            let t = Instant::now();
            let n = AtomicU64::new(0);
            let threats = e.scan(&malware::quick_scan_roots(), &cancel, &n);
            println!("quick scan: {} files, {} threats [{:?}]", n.into_inner(), threats.len(), t.elapsed());
            for th in threats.iter().take(10) { println!("  {:?} {} {}", th.severity, th.name, th.path.display()); }
        }
        Err(e) => println!("yara load failed: {e}"),
    }
    for th in malware::audit_persistence() { println!("  persistence: {:?} {}", th.severity, th.name); }
}
