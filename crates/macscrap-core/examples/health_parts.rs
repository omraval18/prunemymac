use std::time::Instant;
use sysinfo::*;
fn main() {
    let mut sys = System::new();
    let mut comps = Components::new_with_refreshed_list();
    let mut nets = Networks::new_with_refreshed_list();
    for _ in 0..3 {
        let t = Instant::now(); sys.refresh_cpu_usage(); let a = t.elapsed();
        let t = Instant::now(); sys.refresh_memory(); let b = t.elapsed();
        let t = Instant::now(); comps.refresh(false); let c = t.elapsed();
        let t = Instant::now(); nets.refresh(true); let d = t.elapsed();
        let t = Instant::now(); let _ = std::process::Command::new("/usr/bin/pmset").args(["-g","batt"]).output(); let e = t.elapsed();
        println!("cpu {a:?} mem {b:?} components {c:?} ({} sensors) net {d:?} pmset {e:?}", comps.len());
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    for c in comps.iter().take(80) { if c.label().to_lowercase().contains("tdie") || c.label().contains("CPU") || c.label().contains("MTR") { print!("{}={:?}  ", c.label(), c.temperature()); } }
    println!();
}
