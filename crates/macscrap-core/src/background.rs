//! Things running in the background: launchd agents/daemons (persistent) and
//! the heaviest live processes (right now).

#[cfg(target_os = "macos")]
use crate::fsutil;
use std::path::{Path, PathBuf};
#[cfg(target_os = "macos")]
use std::process::Command;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[cfg(target_os = "linux")]
#[path = "background_linux.rs"]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{disable, launch_items};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    UserAgent,
    GlobalAgent,
    GlobalDaemon,
}

impl Scope {
    pub fn label(self) -> &'static str {
        #[cfg(target_os = "linux")]
        { return match self { Scope::UserAgent => "User Startup", Scope::GlobalAgent => "System Startup", Scope::GlobalDaemon => "System Service" }; }
        #[cfg(target_os = "macos")]
        match self {
            Scope::UserAgent => "User Agent",
            Scope::GlobalAgent => "Global Agent",
            Scope::GlobalDaemon => "System Daemon",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LaunchItem {
    pub label: String,
    pub plist: PathBuf,
    pub scope: Scope,
    pub program: Option<PathBuf>,
    pub run_at_load: bool,
    pub keep_alive: bool,
    /// The program it launches no longer exists.
    pub broken: bool,
    pub signer: Option<String>,
    pub loaded: bool,
}

impl LaunchItem {
    pub fn is_apple(&self) -> bool {
        self.label.starts_with("com.apple.")
    }
}

#[cfg(target_os = "macos")]
pub fn launch_items() -> Vec<LaunchItem> {
    let loaded = loaded_labels();
    let dirs = [
        (fsutil::home().join("Library/LaunchAgents"), Scope::UserAgent),
        (PathBuf::from("/Library/LaunchAgents"), Scope::GlobalAgent),
        (PathBuf::from("/Library/LaunchDaemons"), Scope::GlobalDaemon),
    ];
    let mut items: Vec<LaunchItem> = dirs
        .iter()
        .flat_map(|(dir, scope)| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "plist"))
                .filter_map(|e| parse_launch_plist(&e.path(), *scope))
                .collect::<Vec<_>>()
        })
        .collect();
    for item in &mut items {
        item.loaded = loaded.iter().any(|l| l == &item.label);
        item.signer = item.program.as_deref().and_then(code_signer);
    }
    items.sort_by(|a, b| b.broken.cmp(&a.broken).then(a.label.cmp(&b.label)));
    items
}

#[cfg(target_os = "macos")]
fn parse_launch_plist(path: &Path, scope: Scope) -> Option<LaunchItem> {
    let value = plist::Value::from_file(path).ok()?;
    let dict = value.as_dictionary()?;
    let label = dict.get("Label")?.as_string()?.to_string();
    let program = dict
        .get("Program")
        .and_then(|v| v.as_string())
        .or_else(|| dict.get("ProgramArguments")?.as_array()?.first()?.as_string())
        .map(PathBuf::from);
    let keep_alive = match dict.get("KeepAlive") {
        Some(plist::Value::Boolean(b)) => *b,
        Some(plist::Value::Dictionary(_)) => true,
        _ => false,
    };
    // Only absolute paths can be checked; bare names resolve via PATH.
    let broken = program.as_ref().is_some_and(|p| p.is_absolute() && !p.exists());
    Some(LaunchItem {
        label,
        plist: path.to_path_buf(),
        scope,
        program,
        run_at_load: dict.get("RunAtLoad").and_then(|v| v.as_boolean()).unwrap_or(false),
        keep_alive,
        broken,
        signer: None,
        loaded: false,
    })
}

#[cfg(target_os = "macos")]
fn loaded_labels() -> Vec<String> {
    let Ok(out) = Command::new("/bin/launchctl").arg("list").output() else {
        return vec![];
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .skip(1)
        .filter_map(|l| l.split('\t').nth(2).map(str::to_string))
        .collect()
}

/// Leaf signing authority ("Developer ID Application: Foo (TEAMID)"), or None
/// when unsigned / ad-hoc / missing.
#[cfg(target_os = "macos")]
pub fn code_signer(path: &Path) -> Option<String> {
    if !path.exists() {
        return None;
    }
    let out = Command::new("/usr/bin/codesign").arg("-dvv").arg(path).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .find_map(|l| l.strip_prefix("Authority="))
        .map(str::to_string)
}

/// Stop a user agent now and keep it from starting again by moving its plist
/// to the Trash. System-wide items need admin rights and are left to the user.
#[cfg(target_os = "macos")]
pub fn disable(item: &LaunchItem) -> Result<(), String> {
    if item.scope != Scope::UserAgent {
        return Err("System-wide items need administrator rights. Remove the plist from Finder.".into());
    }
    let uid = unsafe { libc::getuid() };
    let _ = Command::new("/bin/launchctl")
        .args(["bootout", &format!("gui/{uid}")])
        .arg(&item.plist)
        .output();
    crate::clean::remove(&item.plist, crate::clean::Method::Trash)
}

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub memory: u64,
    pub app_bundle: Option<PathBuf>,
    pub app_name: Option<String>,
    pub executable: Option<PathBuf>,
    pub started: u64,
    pub can_quit: bool,
}

/// The outermost application bundle owns nested helper apps too.
#[cfg(target_os = "macos")]
pub fn app_bundle(executable: &Path) -> Option<PathBuf> {
    executable.ancestors().filter(|path| path.extension().is_some_and(|ext| ext == "app")).last().map(Path::to_path_buf)
}

/// Heaviest processes by CPU, then memory. Takes ~250ms because CPU usage
/// needs two samples.
pub fn heavy_processes(limit: usize) -> Vec<ProcessInfo> {
    let mut sys = System::new();
    let kind = ProcessRefreshKind::nothing().with_cpu().with_memory().with_exe(UpdateKind::OnlyIfNotSet).with_user(UpdateKind::Always);
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(std::time::Duration::from_millis(250)));
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, kind);
    let mut signatures = std::collections::HashMap::<PathBuf, bool>::new();
    let uid = unsafe { libc::getuid() };
    #[cfg(target_os = "linux")]
    let app_executables: std::collections::HashMap<PathBuf, (PathBuf, String)> = crate::apps::list(&crate::Cancel::new())
        .into_iter().filter_map(|app| app.executable.map(|exe| (exe, (app.path, app.name)))).collect();
    let mut procs: Vec<ProcessInfo> = sys
        .processes()
        .values()
        .map(|p| {
            let executable = p.exe().map(Path::to_path_buf);
            let same_user = uid != 0 && p.user_id().is_some_and(|id| **id == uid)
                && p.effective_user_id().is_some_and(|id| **id == uid);
            let can_quit = p.pid().as_u32() != std::process::id() && p.start_time() != 0 && same_user
                && executable.as_ref().is_some_and(|exe| *signatures.entry(exe.clone())
                    .or_insert_with(|| crate::network::cached_verified_third_party_app(exe)));
            #[cfg(target_os = "macos")]
            let (app_bundle, app_name) = {
                let bundle = p.exe().and_then(app_bundle);
                let name = bundle.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned());
                (bundle, name)
            };
            #[cfg(target_os = "linux")]
            let (app_bundle, app_name) = executable.as_ref().and_then(|p| p.canonicalize().ok())
                .and_then(|exe| app_executables.get(&exe).cloned())
                .map(|(path, name)| (Some(path), Some(name))).unwrap_or((None, None));
            ProcessInfo {
                pid: p.pid().as_u32(),
                name: p.name().to_string_lossy().into_owned(),
                cpu: p.cpu_usage(),
                memory: p.memory(),
                app_bundle,
                app_name,
                executable,
                started: p.start_time(),
                can_quit,
            }
        })
        .collect();
    procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.memory.cmp(&a.memory)));
    procs.truncate(limit);
    procs
}

pub fn quit_process(process: &ProcessInfo) -> Result<(), String> {
    crate::network::stop_verified_app(process.pid, process.started, process.executable.as_deref())
}

#[cfg(all(test, target_os = "macos"))]
mod process_group_tests {
    use super::*;

    #[test]
    fn groups_nested_helpers_under_the_outer_app() {
        let exe = Path::new("/Applications/Browser.app/Contents/Helpers/Browser Helper.app/Contents/MacOS/Helper");
        assert_eq!(app_bundle(exe), Some(PathBuf::from("/Applications/Browser.app")));
        assert_eq!(app_bundle(Path::new("/usr/bin/python3")), None);
    }
}
