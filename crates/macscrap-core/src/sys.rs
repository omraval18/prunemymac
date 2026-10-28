//! Being a good citizen on the user's Mac: background priority for scans,
//! handing freed memory back to the OS, and never leaving zombie processes.

use std::process::Command;

#[cfg(target_os = "macos")]
extern "C" {
    fn pthread_set_qos_class_self_np(qos_class: u32, relative_priority: i32) -> i32;
    fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
}

/// `QOS_CLASS_UTILITY`: long-running work the user is aware of but not
/// waiting on frame-by-frame. macOS favors efficiency cores and yields to
/// whatever the user is actively doing.
#[cfg(target_os = "macos")]
const QOS_CLASS_UTILITY: u32 = 0x11;

/// Run the calling thread at utility QoS.
pub fn background_priority() {
    #[cfg(target_os = "macos")]
    unsafe { pthread_set_qos_class_self_np(QOS_CLASS_UTILITY, 0) };
    #[cfg(target_os = "linux")]
    unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, 5); }
}

/// Configure the global rayon pool (used by every scanner) to run at
/// utility QoS. Call once at startup, before any scan.
pub fn init_thread_pool() {
    let _ = rayon::ThreadPoolBuilder::new()
        .thread_name(|i| format!("macscrap-worker-{i}"))
        .start_handler(|_| background_priority())
        .build_global();
}

/// Return free pages held by the allocator to macOS. Call after dropping
/// large results (a Space Lens tree, a finished scan).
pub fn release_free_memory() {
    #[cfg(target_os = "macos")]
    unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe { libc::malloc_trim(0); }
}

/// Launch a helper command without blocking, and reap it when it exits so
/// it never lingers as a zombie process.
pub fn spawn_detached(cmd: &mut Command) {
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
