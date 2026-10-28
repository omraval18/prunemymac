//! A sampled view of TCP connections visible to the current user.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Debug, Clone)]
pub struct Connection {
    pub pid: u32,
    pub process: String,
    pub local: String,
    pub remote: String,
    pub executable: Option<PathBuf>,
    pub review_reason: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Port {
    pub pid: u32,
    pub process: String,
    pub protocol: String,
    pub address: String,
    pub number: u16,
    pub executable: Option<PathBuf>,
    pub started: u64,
    pub locked_reason: Option<&'static str>,
}

/// Listening TCP ports and bound UDP sockets visible to the current user.
pub fn ports() -> Result<Vec<Port>, String> {
    let output = Command::new(if cfg!(target_os = "linux") { "lsof" } else { "/usr/sbin/lsof" })
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-iUDP", "-FpcuPn"])
        .output().map_err(|e| format!("Could not read ports: {e}"))?;
    if !output.status.success() && !(output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty()) {
        let detail = String::from_utf8_lossy(&output.stderr);
        return Err(if detail.trim().is_empty() { format!("Could not read ports ({})", output.status) }
            else { format!("Could not read ports: {}", detail.trim()) });
    }
    let mut rows = parse_ports(&String::from_utf8_lossy(&output.stdout));
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing()
        .with_exe(UpdateKind::OnlyIfNotSet).with_user(UpdateKind::Always));
    let uid = unsafe { libc::getuid() };
    let mut signatures = HashMap::<PathBuf, bool>::new();
    for row in &mut rows {
        let Some(process) = sys.process(Pid::from_u32(row.pid)) else { continue };
        row.executable = process.exe().map(Path::to_path_buf);
        row.started = process.start_time();
        let same_user = uid != 0 && process.user_id().is_some_and(|id| **id == uid)
            && process.effective_user_id().is_some_and(|id| **id == uid);
        row.locked_reason = port_lock_reason(row.pid, same_user, row.number, row.started, row.executable.as_deref(), |exe| {
            *signatures.entry(exe.to_path_buf()).or_insert_with(|| cached_verified_third_party_app(exe))
        });
    }
    rows.sort_by(|a, b| a.number.cmp(&b.number).then(a.protocol.cmp(&b.protocol))
        .then(a.pid.cmp(&b.pid)).then(a.address.cmp(&b.address)));
    rows.dedup_by(|a, b| a.pid == b.pid && a.protocol == b.protocol && a.address == b.address);
    Ok(rows)
}

/// Cache display eligibility briefly. Stopping always performs a fresh check.
pub(crate) fn cached_verified_third_party_app(path: &Path) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (Instant, bool)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(cache) = cache.lock() {
        if let Some((at, result)) = cache.get(path) {
            if at.elapsed() < Duration::from_secs(60) { return *result; }
        }
    }
    let result = verified_third_party_app(path);
    if let Ok(mut cache) = cache.lock() {
        cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(60));
        cache.insert(path.to_path_buf(), (Instant::now(), result));
    }
    result
}

fn port_lock_reason(pid: u32, same_user: bool, number: u16, started: u64, executable: Option<&Path>,
    mut verify: impl FnMut(&Path) -> bool) -> Option<&'static str> {
    if pid == std::process::id() { Some("This app is protected") }
    else if !same_user { Some("System or other user") }
    else if number < 1024 { Some("Privileged port") }
    else if started == 0 || executable.is_none() { Some("Process identity unavailable") }
    else if !executable.is_some_and(&mut verify) { Some("System or unverified process") }
    else { None }
}

/// Refuse anything except a verified, non-Apple application executable.
/// A valid signature alone never implies that stopping an app is harmless.
#[cfg(target_os = "macos")]
pub(crate) fn verified_third_party_app(executable: &Path) -> bool {
    let Ok(executable) = executable.canonicalize() else { return false };
    let Some(bundle) = crate::background::app_bundle(&executable) else { return false };
    let user_apps = crate::fsutil::home().join("Applications");
    if !(bundle.starts_with("/Applications") || bundle.starts_with(user_apps)) {
        return false;
    }
    let Ok(assessment) = Command::new("/usr/sbin/spctl").args(["--assess", "--type", "execute"]).arg(&bundle).output() else { return false };
    if !assessment.status.success() { return false; }
    let Ok(verify) = Command::new("/usr/bin/codesign").args(["--verify", "--strict"]).arg(&executable).output() else { return false };
    if !verify.status.success() { return false; }
    let Ok(info) = Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2"]).arg(&executable).output() else { return false };
    if !info.status.success() { return false; }
    let text = String::from_utf8_lossy(&info.stderr);
    text.lines().find_map(|line| line.strip_prefix("Authority="))
        .is_some_and(|authority| authority.starts_with("Developer ID Application:") || authority.starts_with("Apple Distribution:"))
        && text.lines().any(|line| line.starts_with("TeamIdentifier=") && line != "TeamIdentifier=not set")
}

#[cfg(target_os = "linux")]
pub(crate) fn verified_third_party_app(executable: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(executable) = executable.canonicalize() else { return false };
    let Ok(home) = crate::fsutil::home().canonicalize() else { return false };
    let Ok(meta) = std::fs::metadata(&executable) else { return false };
    let uid = unsafe { libc::getuid() };
    if uid == 0 || !meta.is_file() { return false; }
    if executable.starts_with(&home) && meta.uid() == uid { return true; }
    if meta.uid() != 0 || !(executable.starts_with("/usr") || executable.starts_with("/opt")) { return false; }
    let Some(package) = linux_desktop_packages().get(&executable).cloned() else { return false };
    // Confirm the executable belongs to the same installed package as the
    // desktop entry. Essential and required system packages stay protected.
    let Ok(owner) = Command::new("dpkg-query").args(["--search", "--"]).arg(&executable).output() else { return false };
    if !owner.status.success() { return false; }
    let owner_text = String::from_utf8_lossy(&owner.stdout);
    let owns_executable = owner_text.lines().filter_map(|line| line.rsplit_once(": "))
        .any(|(name, path)| name.split(':').next() == Some(package.as_str()) && Path::new(path) == executable);
    if !owns_executable { return false; }
    let Ok(status) = Command::new("dpkg-query").args(["-W", "-f=${db:Status-Status}\t${Essential}\t${Priority}", "--", &package]).output() else { return false };
    if !status.status.success() { return false; }
    let text = String::from_utf8_lossy(&status.stdout);
    let fields: Vec<_> = text.trim_end().split('\t').collect();
    fields.len() == 3 && fields[0] == "installed" && fields[1] != "yes" && matches!(fields[2], "optional" | "extra")
}

#[cfg(target_os = "linux")]
fn linux_desktop_packages() -> HashMap<PathBuf, String> {
    static CACHE: OnceLock<Mutex<Option<(Instant, HashMap<PathBuf, String>)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(cache) = cache.lock() {
        if let Some((at, entries)) = &*cache {
            if at.elapsed() < Duration::from_secs(60) { return entries.clone(); }
        }
    }
    let entries: HashMap<_, _> = crate::apps::list(&crate::Cancel::new()).into_iter()
        .filter_map(|app| Some((app.executable?, app.package?))).collect();
    if let Ok(mut cache) = cache.lock() { *cache = Some((Instant::now(), entries.clone())); }
    entries
}

/// Recheck identity, ownership, signature and port immediately before SIGTERM.
pub fn stop_port_process(target: &Port) -> Result<(), String> {
    if target.locked_reason.is_some() || target.started == 0 || target.executable.is_none() {
        return Err("This process is protected".into());
    }
    let current = ports()?.into_iter().find(|p| p.pid == target.pid && p.protocol == target.protocol
        && p.address == target.address && p.number == target.number && p.started == target.started
        && p.executable == target.executable);
    match current {
        Some(port) if port.locked_reason.is_none() => {},
        _ => return Err("The process or port changed; refresh and try again".into()),
    }
    stop_verified_app(target.pid, target.started, target.executable.as_deref())
}

/// Check again at the point of stopping, including both real and effective UID.
pub fn stop_verified_app(pid: u32, started: u64, expected_exe: Option<&Path>) -> Result<(), String> {
    if pid == std::process::id() || started == 0 || expected_exe.is_none() {
        return Err("This process is protected or its identity is unavailable".into());
    }
    let mut sys = System::new();
    let selected = [Pid::from_u32(pid)];
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&selected), true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always).with_user(UpdateKind::Always));
    let process = sys.process(selected[0]).ok_or("The process has already exited")?;
    let uid = unsafe { libc::getuid() };
    if uid == 0 || process.start_time() != started || process.exe() != expected_exe
        || !process.user_id().is_some_and(|id| **id == uid)
        || !process.effective_user_id().is_some_and(|id| **id == uid)
        || !expected_exe.is_some_and(verified_third_party_app) {
        return Err("This process is protected, unverified, or has changed".into());
    }
    let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
    if rc == 0 { Ok(()) } else { Err(std::io::Error::last_os_error().to_string()) }
}

fn parse_ports(output: &str) -> Vec<Port> {
    let mut rows = Vec::new();
    let mut pid = None;
    let mut process = String::new();
    let mut protocol = "";
    for line in output.lines() {
        let Some((field, value)) = line.split_at_checked(1) else { continue };
        match field {
            "p" => { pid = value.parse::<u32>().ok(); process.clear(); protocol = ""; }
            "c" => process = value.to_owned(),
            "f" => protocol = "",
            "P" => protocol = if value == "TCP" { "TCP" } else if value == "UDP" { "UDP" } else { "" },
            "n" if !protocol.is_empty() => {
                let address = value.split_once("->").map_or(value, |(local, _)| local);
                if let (Some(pid), Some(number)) = (pid, address.rsplit_once(':').and_then(|(_, p)| p.parse::<u16>().ok())) {
                    rows.push(Port { pid, process: process.clone(), protocol: protocol.into(), address: address.into(), number,
                        executable: None, started: 0, locked_reason: Some("Process identity unavailable") });
                }
            }
            _ => {}
        }
    }
    rows
}

/// Snapshot only: short-lived connections and processes owned by other users
/// can be absent. Socket direction and HTTP request details are not available.
pub fn connections() -> Result<Vec<Connection>, String> {
    let output = Command::new(if cfg!(target_os = "linux") { "lsof" } else { "/usr/sbin/lsof" })
        .args(["-nP", "-iTCP", "-sTCP:ESTABLISHED", "-Fpcn"])
        .output()
        .map_err(|e| format!("Could not read connections: {e}"))?;
    // lsof exits 1 when there are no matching sockets.
    if !output.status.success() && !(output.status.code() == Some(1) && output.stdout.is_empty() && output.stderr.is_empty()) {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        return Err(if detail.is_empty() {
            format!("Could not read connections ({})", output.status)
        } else {
            format!("Could not read connections: {detail}")
        });
    }
    let mut rows = parse_lsof(&String::from_utf8_lossy(&output.stdout));
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
    );
    let downloads = crate::fsutil::home().join("Downloads");
    for row in &mut rows {
        row.executable = system.process(Pid::from_u32(row.pid)).and_then(|p| p.exe()).map(Path::to_path_buf);
        row.review_reason = row.executable.as_deref().and_then(|path| review_reason(path, &downloads));
    }
    rows.sort_by(|a, b| b.review_reason.is_some().cmp(&a.review_reason.is_some())
        .then_with(|| a.process.to_lowercase().cmp(&b.process.to_lowercase()))
        .then_with(|| a.remote.cmp(&b.remote)));
    Ok(rows)
}

fn review_reason(path: &Path, downloads: &Path) -> Option<&'static str> {
    if path.starts_with(downloads) {
        Some("Running from Downloads")
    } else if ["/tmp", "/private/tmp", "/var/tmp", "/private/var/tmp", "/private/var/folders"]
        .iter().any(|dir| path.starts_with(dir)) {
        Some("Running from a temporary folder")
    } else {
        None
    }
}

fn parse_lsof(output: &str) -> Vec<Connection> {
    let mut rows = Vec::new();
    let mut pid = None;
    let mut process = String::new();
    for line in output.lines() {
        let Some((field, value)) = line.split_at_checked(1) else { continue };
        match field {
            "p" => { pid = value.parse::<u32>().ok(); process.clear(); }
            "c" => process = value.to_owned(),
            "n" => {
                if let (Some(pid), Some((local, remote))) = (pid, value.split_once("->")) {
                    rows.push(Connection {
                        pid,
                        process: process.clone(),
                        local: local.to_owned(),
                        remote: remote.to_owned(),
                        executable: None,
                        review_reason: None,
                    });
                }
            }
            _ => {}
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_processes_and_ipv6_connections() {
        let rows = parse_lsof("p12\ncBrowser Helper\nf4\nn192.0.2.1:1234->203.0.113.7:443\nf5\nn[::1]:99->[::1]:80\np34\ncOther\nf7\nn*:8080\n");
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].pid, rows[0].process.as_str(), rows[0].remote.as_str()), (12, "Browser Helper", "203.0.113.7:443"));
        assert_eq!(rows[1].remote, "[::1]:80");
    }

    #[test]
    fn only_flags_executables_inside_review_locations() {
        let downloads = Path::new("/Users/example/Downloads");
        assert_eq!(review_reason(Path::new("/Users/example/Downloads/tool"), downloads), Some("Running from Downloads"));
        assert_eq!(review_reason(Path::new("/private/tmp/tool"), downloads), Some("Running from a temporary folder"));
        assert_eq!(review_reason(Path::new("/Applications/Example.app/Contents/MacOS/Example"), downloads), None);
        assert_eq!(review_reason(Path::new("/Users/example/Downloads-old/tool"), downloads), None);
    }

    #[test]
    fn parses_tcp_and_udp_ports_without_treating_remote_as_local() {
        let rows = parse_ports("p12\ncDev Server\nu501\nf4\nPTCP\nn*:8080\nf5\nPUDP\nn[::1]:5353\nf6\nPUDP\nn127.0.0.1:5000->192.0.2.1:53\nf7\nPTCP\nn*:not-a-port\n");
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[0].protocol.as_str(), rows[0].number), ("TCP", 8080));
        assert_eq!((rows[1].protocol.as_str(), rows[1].number), ("UDP", 5353));
        assert_eq!((rows[2].address.as_str(), rows[2].number), ("127.0.0.1:5000", 5000));
        assert!(rows.iter().all(|row| row.locked_reason.is_some()));
    }

    #[test]
    fn protected_or_unidentified_processes_cannot_be_stopped() {
        let port = Port { pid: std::process::id(), process: "self".into(), protocol: "TCP".into(), address: "*:8080".into(),
            number: 8080, executable: None, started: 0, locked_reason: Some("Protected") };
        assert!(stop_port_process(&port).is_err());
        assert!(stop_verified_app(std::process::id(), 0, None).is_err());
        assert!(!verified_third_party_app(Path::new("/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder")));
    }

    #[test]
    fn port_protection_fails_closed_for_unknown_or_system_processes() {
        let exe = Some(Path::new("/Applications/Example.app/Contents/MacOS/Example"));
        let other = std::process::id().wrapping_add(1);
        assert!(port_lock_reason(other, false, 8080, 1, exe, |_| true).is_some());
        assert!(port_lock_reason(other, true, 80, 1, exe, |_| true).is_some());
        assert!(port_lock_reason(other, true, 8080, 0, exe, |_| true).is_some());
        assert!(port_lock_reason(other, true, 8080, 1, None, |_| true).is_some());
        assert!(port_lock_reason(other, true, 8080, 1, exe, |_| false).is_some());
        assert!(port_lock_reason(other, true, 8080, 1, exe, |_| true).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn live_ports_show_own_sockets_as_protected() {
        let tcp = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let udp = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let rows = ports().unwrap();
        for (protocol, number) in [("TCP", tcp.local_addr().unwrap().port()), ("UDP", udp.local_addr().unwrap().port())] {
            let row = rows.iter().find(|row| row.pid == std::process::id() && row.protocol == protocol && row.number == number)
                .expect("own socket should appear in the snapshot");
            assert_eq!(row.locked_reason, Some("This app is protected"));
            assert!(stop_port_process(row).is_err());
        }
    }
}
