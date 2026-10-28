//! Linux desktop application discovery and conservative user-data matching.

use crate::fsutil::{self, Cancel};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    pub path: PathBuf,
    pub version: String,
    pub size: u64,
    pub icon: Option<PathBuf>,
    pub executable: Option<PathBuf>,
    pub package: Option<String>,
    pub user_flatpak: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Leftover {
    pub path: PathBuf,
    pub size: u64,
    pub kind: &'static str,
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| fsutil::home().join(".local/share"))
}

fn config_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| fsutil::home().join(".config"))
}

fn cache_home() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME").map(PathBuf::from).filter(|p| p.is_absolute())
        .unwrap_or_else(|| fsutil::home().join(".cache"))
}

pub fn desktop_roots() -> Vec<PathBuf> {
    let mut dirs = vec![data_home()];
    let shared = std::env::var_os("XDG_DATA_DIRS").unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    dirs.extend(std::env::split_paths(&shared).filter(|p| p.is_absolute()));
    dirs.extend([fsutil::home().join(".local/share/flatpak/exports/share"),
        PathBuf::from("/var/lib/flatpak/exports/share"), PathBuf::from("/var/lib/snapd/desktop")]);
    dirs.into_iter().map(|p| p.join("applications")).collect()
}

fn desktop_fields(path: &Path) -> Option<HashMap<String, String>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut fields = HashMap::new();
    let mut in_entry = false;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with('[') { in_entry = line == "[Desktop Entry]"; continue; }
        if in_entry && !line.starts_with('#') {
            if let Some((key, value)) = line.split_once('=') {
                fields.entry(key.to_string()).or_insert_with(|| value.to_string());
            }
        }
    }
    (fields.get("Type").map(String::as_str) == Some("Application")
        && fields.get("Hidden").map(String::as_str) != Some("true")
        && fields.get("NoDisplay").map(String::as_str) != Some("true")).then_some(fields)
}

fn executable(command: &str) -> Option<PathBuf> {
    // Read the executable name only. Desktop Exec lines are never executed here.
    let command = command.trim_start();
    let first = if let Some(rest) = command.strip_prefix('"') {
        rest.split_once('"')?.0
    } else { command.split_whitespace().next()? };
    if first.is_empty() || first == "env" || first == "flatpak" || first == "snap" { return None; }
    let path = Path::new(first);
    if path.is_absolute() { return path.canonicalize().ok(); }
    if path.components().count() != 1 { return None; }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(path)).find(|p| p.is_file()).and_then(|p| p.canonicalize().ok())
}

fn icon_path(value: &str) -> Option<PathBuf> {
    let path = Path::new(value);
    if path.is_absolute() && path.extension().is_some_and(|e| e == "png") && path.is_file() { return Some(path.to_owned()); }
    if value.contains('/') || value.contains("..") { return None; }
    let mut roots = vec![data_home().join("icons"), fsutil::home().join(".icons"), PathBuf::from("/usr/share/icons")];
    for root in roots.drain(..) {
        for size in ["128x128", "64x64", "48x48", "256x256"] {
            let candidate = root.join("hicolor").join(size).join("apps").join(format!("{value}.png"));
            if candidate.is_file() { return Some(candidate); }
        }
    }
    let candidate = PathBuf::from("/usr/share/pixmaps").join(format!("{value}.png"));
    candidate.is_file().then_some(candidate)
}

fn package_metadata() -> (HashMap<PathBuf, String>, HashMap<String, (String, u64)>) {
    let mut owners = HashMap::new();
    if let Ok(out) = Command::new("dpkg-query").args(["--search", "/usr/share/applications/*.desktop"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some((owner, path)) = line.rsplit_once(": ") {
                if !owner.contains(", ") { owners.insert(PathBuf::from(path), owner.split(':').next().unwrap_or(owner).to_owned()); }
            }
        }
    }
    let mut packages = HashMap::new();
    if let Ok(out) = Command::new("dpkg-query").args(["-W", "-f=${Package}\t${Version}\t${Installed-Size}\n"]).output() {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            let mut cols = line.split('\t');
            if let (Some(name), Some(version), Some(size)) = (cols.next(), cols.next(), cols.next()) {
                packages.insert(name.to_owned(), (version.to_owned(), size.parse::<u64>().unwrap_or(0).saturating_mul(1024)));
            }
        }
    }
    (owners, packages)
}

fn user_flatpaks() -> HashMap<String, String> {
    Command::new("flatpak").args(["list", "--user", "--app", "--columns=application,version"]).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines()
            .filter_map(|line| line.split_once('\t').map(|(id, version)| (id.to_string(), version.to_string())))
            .collect()).unwrap_or_default()
}

/// Desktop launchers, deduplicated using the XDG desktop-file ID.
pub fn list(cancel: &Cancel) -> Vec<AppInfo> {
    let (owners, packages) = package_metadata();
    let flatpaks = user_flatpaks();
    let mut seen = HashSet::new();
    let mut seen_apps = HashSet::new();
    let own_exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok());
    let mut apps = Vec::new();
    for root in desktop_roots() {
        let mut pending = vec![(root.clone(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            if cancel.is_cancelled() { break; }
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() && depth < 3 { pending.push((path, depth + 1)); continue; }
                if path.extension().is_none_or(|ext| ext != "desktop") { continue; }
                let Some(relative) = path.strip_prefix(&root).ok().and_then(|p| p.to_str()) else { continue };
                let id = relative.replace('/', "-").trim_end_matches(".desktop").to_string();
                if !seen.insert(id.clone()) { continue; }
                let Some(fields) = desktop_fields(&path) else { continue };
                let Some(name) = fields.get("Name").filter(|n| !n.is_empty()) else { continue };
                let app_exe = fields.get("Exec").and_then(|s| executable(s));
                if app_exe.is_some() && app_exe == own_exe { continue; }
                if let Some(exe) = &app_exe {
                    if !seen_apps.insert((name.to_lowercase(), exe.clone())) { continue; }
                }
                let package = owners.get(&path).cloned();
                let user_flatpak = fields.get("X-Flatpak").filter(|id| flatpaks.contains_key(*id)).cloned();
                let (version, size) = package.as_ref().and_then(|p| packages.get(p)).cloned().unwrap_or_else(||
                    (user_flatpak.as_ref().and_then(|id| flatpaks.get(id)).cloned().unwrap_or_default(), 0));
                apps.push(AppInfo {
                    name: name.clone(), bundle_id: id, path,
                    icon: fields.get("Icon").and_then(|s| icon_path(s)),
                    executable: app_exe, package, user_flatpak, version, size,
                });
            }
        }
    }
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

pub fn leftovers(app: &AppInfo, cancel: &Cancel) -> Vec<Leftover> {
    let mut identifiers = vec![app.bundle_id.to_lowercase()];
    if let Some(package) = &app.package { identifiers.push(package.to_lowercase()); }
    if let Some(id) = &app.user_flatpak { identifiers.push(id.to_lowercase()); }
    let roots = [(cache_home(), "Cache"), (config_home(), "Preferences"), (data_home(), "Application Data"), (fsutil::home().join(".var/app"), "Container")];
    let mut found = Vec::new();
    for (root, kind) in roots {
        if cancel.is_cancelled() { break; }
        for entry in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            // Exact names only: loose matching risks selecting unrelated files.
            if identifiers.iter().any(|id| id.len() >= 4 && *id == name) {
                let path = entry.path();
                if !fsutil::is_protected(&path) {
                    found.push(Leftover { size: fsutil::size_of(&path, cancel), path, kind });
                }
            }
        }
    }
    found.sort_by(|a, b| b.size.cmp(&a.size));
    found
}

pub fn is_running(app: &AppInfo) -> bool {
    let Some(exe) = &app.executable else { return false };
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| p.exe().and_then(|p| p.canonicalize().ok()).as_ref() == Some(exe))
}

/// Ask the desktop's software manager to show the app. It owns the uninstall
/// transaction and displays any package dependencies and authorization UI.
pub fn open_software_manager(app: &AppInfo) -> Result<(), String> {
    let id = format!("{}.desktop", app.bundle_id);
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')) {
        return Err("Invalid application identifier".into());
    }
    for (command, flag) in [("gnome-software", "--details"), ("plasma-discover", "--application")] {
        if let Ok(mut child) = Command::new(command).args([flag, &id]).spawn() {
            std::thread::spawn(move || { let _ = child.wait(); });
            return Ok(());
        }
    }
    Err("Install GNOME Software or KDE Discover to manage this app, or use your distribution's package manager.".into())
}

/// Remove exactly one per-user Flatpak app, preserving its data and runtimes.
pub fn uninstall_user_flatpak(app: &AppInfo) -> Result<(), String> {
    let id = app.user_flatpak.as_deref().ok_or("This is not a per-user Flatpak app")?;
    if id.starts_with('-') || !id.contains('.') || !id.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
        return Err("Invalid Flatpak application identifier".into());
    }
    let running = Command::new("flatpak").args(["ps", "--columns=application"]).output().map_err(|e| e.to_string())?;
    if !running.status.success() { return Err("Could not check whether the app is running".into()); }
    if String::from_utf8_lossy(&running.stdout).lines().any(|line| line.trim() == id) {
        return Err("Quit the app before uninstalling it".into());
    }
    let info = Command::new("flatpak").args(["info", "--user", "--show-ref", "--", id]).output().map_err(|e| e.to_string())?;
    if !info.status.success() { return Err("The user Flatpak app is no longer installed".into()); }
    let text = String::from_utf8_lossy(&info.stdout);
    let reference = text.trim();
    let parts: Vec<_> = reference.split('/').collect();
    if parts.len() != 4 || parts[0] != "app" || parts[1] != id
        || !reference.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
        return Err("Could not verify the installed Flatpak reference".into());
    }
    let result = Command::new("flatpak").args(["uninstall", "--user", "--app", "--no-related", "--noninteractive", "--", reference])
        .output().map_err(|e| e.to_string())?;
    if result.status.success() { Ok(()) } else { Err(String::from_utf8_lossy(&result.stderr).trim().to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_parser_ignores_hidden_and_non_application_entries() {
        let dir = std::env::temp_dir().join(format!("prunemymac-desktop-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("example.desktop");
        std::fs::write(&path, "[Desktop Entry]\nType=Application\nName=Example\nExec=/usr/bin/example %U\n[Desktop Action Other]\nName=Wrong\n").unwrap();
        assert_eq!(desktop_fields(&path).unwrap().get("Name").map(String::as_str), Some("Example"));
        std::fs::write(&path, "[Desktop Entry]\nType=Application\nName=Hidden\nHidden=true\n").unwrap();
        assert!(desktop_fields(&path).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
