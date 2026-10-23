//! Create a big tree under the given dir, then delete it with progress sampling.
use macscrap_core::clean::{run_tracked, Method, Progress};
use std::sync::atomic::Ordering;
use std::sync::Arc;

fn main() {
    let root = std::path::PathBuf::from(std::env::args().nth(1).expect("dir")).join("bigtree");
    for d in 0..400 {
        let sub = root.join(format!("pkg{d}/node/lib"));
        std::fs::create_dir_all(&sub).unwrap();
        for f in 0..500 {
            std::fs::write(sub.join(format!("f{f}.js")), b"module.exports = 1;\n").unwrap();
        }
    }
    let size = macscrap_core::fsutil::size_of(&root, &macscrap_core::Cancel::new());
    println!("created 200,000 files, {}", macscrap_core::human_bytes(size));
    let progress = Arc::new(Progress::default());
    let p2 = progress.clone();
    let t = std::time::Instant::now();
    let worker = std::thread::spawn(move || run_tracked([(root, size, Method::Delete)], &p2));
    while !worker.is_finished() {
        std::thread::sleep(std::time::Duration::from_millis(100));
        println!("  {:>5}ms  files {:>7}  freed {:>5.1}%", t.elapsed().as_millis(), progress.files.load(Ordering::Relaxed), progress.freed.load(Ordering::Relaxed) as f64 * 100.0 / size as f64);
    }
    let r = worker.join().unwrap();
    println!("done in {:?}: removed {} freed {} failures {}", t.elapsed(), r.removed, macscrap_core::human_bytes(r.freed), r.failures.len());
}
