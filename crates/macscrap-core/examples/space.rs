//! Benchmark Space Lens on a path: `cargo run --release --example space -- ~`
use macscrap_core::{human_bytes, space, Cancel};
use std::sync::atomic::AtomicU64;

fn main() {
    let root = std::env::args().nth(1).map(std::path::PathBuf::from).unwrap_or_else(space::whole_mac_root);
    let t = std::time::Instant::now();
    let n = AtomicU64::new(0);
    let tree = space::scan(&root, &Cancel::new(), &n);
    println!("{}: {} in {} files, {} dirs [{:?}]", root.display(), human_bytes(tree.size), tree.files, tree.dir_count(), t.elapsed());
    for d in tree.dirs.iter().take(8) {
        println!("  {:>10}  {:>9} files  {}{}", human_bytes(d.size), d.files, d.name, if d.unreadable { " (unreadable)" } else { "" });
    }
    let rss = std::process::Command::new("ps").args(["-o", "rss=", "-p", &std::process::id().to_string()]).output().unwrap();
    println!("rss: {} MB", String::from_utf8_lossy(&rss.stdout).trim().parse::<u64>().unwrap_or(0) / 1024);
}
