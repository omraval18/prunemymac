//! XDG autostart entries and enabled systemd services.

use super::{LaunchItem, Scope};
use crate::fsutil;
use std::path::{Path, PathBuf};
use std::process::Command;

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| fsutil::home().join(".config"))
}

fn autostart(path: PathBuf, scope: Scope) -> Option<LaunchItem> {
    if path.extension().is_none_or(|ext| ext != "desktop") { return None; }
    let content = std::fs::read_to_string(&path).ok()?;
    let mut entry = false;
    let (mut label, mut program, mut hidden) = (None, None, false);
    for line in content.lines().map(str::trim) {
        if line.starts_with('[') { entry = line == "[Desktop Entry]"; continue; }
        if !entry { continue; }
        if let Some(name) = line.strip_prefix("Name=") { label = Some(name.to_string()); }
        if let Some(exec) = line.strip_prefix("Exec=") {
            program = exec.split_whitespace().next().filter(|s| s.starts_with('/')).map(PathBuf::from);
        }
        if line == "Hidden=true" || line == "X-GNOME-Autostart-enabled=false" { hidden = true; }
    }
    if hidden { return None; }
    let broken = program.as_ref().is_some_and(|p| !p.exists());
    Some(LaunchItem { label: label.unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().into_owned()),
        plist: path, scope, program, run_at_load: true, keep_alive: false, broken, signer: None, loaded: false })
}

fn systemd_items(root: &Path, scope: Scope, running: &[String]) -> Vec<LaunchItem> {
    let mut result = Vec::new();
    for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let dir = entry.path();
        if !dir.is_dir() || !dir.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".wants")) { continue; }
        for unit in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = unit.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else { continue };
            if !(name.ends_with(".service") || name.ends_with(".socket") || name.ends_with(".timer")) { continue; }
            let label = name.to_string();
            let broken = !path.exists();
            result.push(LaunchItem { loaded: running.iter().any(|s| s == &label), label, plist: path,
                scope, program: None, run_at_load: true, keep_alive: false, broken, signer: None });
        }
    }
    result
}

pub fn launch_items() -> Vec<LaunchItem> {
    let running = Command::new("systemctl").args(["--user", "list-units", "--state=running", "--no-legend", "--plain"])
        .output().ok().map(|o| String::from_utf8_lossy(&o.stdout).lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
    let mut items = Vec::new();
    for (dir, scope) in [(config_home().join("autostart"), Scope::UserAgent),
        (PathBuf::from("/etc/xdg/autostart"), Scope::GlobalAgent)] {
        items.extend(std::fs::read_dir(dir).into_iter().flatten().flatten().filter_map(|e| autostart(e.path(), scope)));
    }
    items.extend(systemd_items(&config_home().join("systemd/user"), Scope::UserAgent, &running));
    items.extend(systemd_items(Path::new("/etc/systemd/system"), Scope::GlobalDaemon, &[]));
    items.sort_by(|a, b| a.label.to_lowercase().cmp(&b.label.to_lowercase()));
    items
}

pub fn disable(item: &LaunchItem) -> Result<(), String> {
    if item.scope != Scope::UserAgent { return Err("System startup items are protected".into()); }
    let config = config_home().canonicalize().map_err(|_| "User configuration directory is unavailable")?;
    let parent = item.plist.parent().and_then(|p| p.canonicalize().ok()).ok_or("Startup entry is unavailable")?;
    if !parent.starts_with(&config) || !config.starts_with(fsutil::home().canonicalize().map_err(|_| "Home is unavailable")?) {
        return Err("Startup item is outside your home configuration".into());
    }
    if parent == config.join("autostart") && item.plist.extension().is_some_and(|ext| ext == "desktop") {
        if fsutil::is_protected(&item.plist) { return Err("Startup item is protected".into()); }
        return trash::delete(&item.plist).map_err(|e| e.to_string());
    }
    let systemd = config.join("systemd/user");
    let valid_parent = parent.parent().is_some_and(|p| p == systemd)
        && parent.file_name().is_some_and(|n| n.to_string_lossy().ends_with(".wants"));
    let name = item.plist.file_name().and_then(|n| n.to_str()).ok_or("Invalid service name")?;
    if !valid_parent || name.starts_with('-') || !name.chars().all(|c| c.is_ascii_alphanumeric() || "_.@:-\\".contains(c))
        || !(name.ends_with(".service") || name.ends_with(".timer") || name.ends_with(".socket")) {
        return Err("Startup item is protected".into());
    }
    let out = Command::new("systemctl").args(["--user", "disable", "--now", "--", name]).output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&out.stderr).trim().to_string()) }
}
