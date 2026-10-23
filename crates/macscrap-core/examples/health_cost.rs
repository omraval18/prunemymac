//! Average cost of Monitor::sample over 20 calls at the app's cadence.
fn main() {
    let mut m = macscrap_core::health::Monitor::new();
    let mut total = std::time::Duration::ZERO;
    for _ in 0..20 {
        let t = std::time::Instant::now();
        let s = m.sample();
        total += t.elapsed();
        assert!(s.cpu_temp.is_some());
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    println!("avg sample {:?}", total / 20);
}
