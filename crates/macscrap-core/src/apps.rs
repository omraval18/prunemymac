//! App uninstaller: list installed apps and find every file they left
//! around the system, matched by bundle identifier and app name.

use crate::fsutil::{self, Cancel};
use rayon::prelude::*;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct AppInfo {
    pub name: String,
    pub bundle_id: String,
    pub path: PathBuf,
    pub version: String,
    pub size: u64,
    /// PNG rendering of the app icon, cached on disk.
    pub icon: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct Leftover {
    pub path: PathBuf,
    pub size: u64,
    pub kind: &'static str,
}

struct BundleInfo {
    name: String,
    bundle_id: String,
    version: String,
    icon_file: Option<String>,
}

fn read_info(app: &Path) -> Option<BundleInfo> {
    let v = plist::Value::from_file(app.join("Contents/Info.plist")).ok()?;
    let d = v.as_dictionary()?;
    let s = |k: &str| d.get(k).and_then(|v| v.as_string()).map(str::to_string);
    // Prefer the bundle's file name: it's what users see in Finder.
    let name = app
        .file_stem()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| s("CFBundleDisplayName"))
        .or_else(|| s("CFBundleName"))?;
    Some(BundleInfo {
        name,
        bundle_id: s("CFBundleIdentifier")?,
        version: s("CFBundleShortVersionString").unwrap_or_default(),
        icon_file: s("CFBundleIconFile"),
    })
}

fn icon_cache_dir() -> PathBuf {
    fsutil::home().join("Library/Caches/macscrap/icons")
}

/// Render the app's .icns to a 128px PNG via the built-in `sips`, cached by
/// bundle id + version so it's done once per app version.
fn app_icon(app: &Path, info: &BundleInfo) -> Option<PathBuf> {
    let out = icon_cache_dir().join(format!("{}-{}.png", info.bundle_id, info.version.replace('/', "_")));
    if out.exists() {
        return Some(out);
    }
    let resources = app.join("Contents/Resources");
    let named = info.icon_file.as_ref().map(|f| {
        let f = resources.join(f);
        if f.extension().is_none() { f.with_extension("icns") } else { f }
    });
    let icns = named.filter(|p| p.exists()).or_else(|| {
        std::fs::read_dir(&resources)
            .ok()?
            .flatten()
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "icns"))
    })?;
    std::fs::create_dir_all(icon_cache_dir()).ok()?;
    let ok = std::process::Command::new("/usr/bin/sips")
        .args(["-s", "format", "png", "-Z", "128"])
        .arg(&icns)
        .arg("--out")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    ok.then_some(out)
}

/// Apps in /Applications and ~/Applications (one level of subfolders too).
pub fn list(cancel: &Cancel) -> Vec<AppInfo> {
    let roots = [PathBuf::from("/Applications"), fsutil::home().join("Applications")];
    let mut bundles = Vec::new();
    for root in &roots {
        for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
            let p = e.path();
            if p.extension().is_some_and(|x| x == "app") {
                bundles.push(p);
            } else if p.is_dir() {
                bundles.extend(
                    std::fs::read_dir(&p)
                        .into_iter()
                        .flatten()
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.extension().is_some_and(|x| x == "app")),
                );
            }
        }
    }
    let mut apps: Vec<AppInfo> = bundles
        .par_iter()
        .filter_map(|p| {
            let info = read_info(p)?;
            // Apple's bundled apps live on the sealed system volume.
            if info.bundle_id.starts_with("com.apple.") {
                return None;
            }
            Some(AppInfo {
                icon: app_icon(p, &info),
                size: fsutil::size_of(p, cancel),
                path: p.clone(),
                name: info.name,
                bundle_id: info.bundle_id,
                version: info.version,
            })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}

/// Locations where apps keep data, as (directory, kind label).
fn search_roots() -> Vec<(PathBuf, &'static str)> {
    let h = fsutil::home();
    vec![
        (h.join("Library/Application Support"), "Application Support"),
        (h.join("Library/Caches"), "Cache"),
        (h.join("Library/Preferences"), "Preferences"),
        (h.join("Library/Preferences/ByHost"), "Preferences"),
        (h.join("Library/Logs"), "Logs"),
        (h.join("Library/Saved Application State"), "Saved State"),
        (h.join("Library/Containers"), "Container"),
        (h.join("Library/Group Containers"), "Group Container"),
        (h.join("Library/HTTPStorages"), "Web Data"),
        (h.join("Library/WebKit"), "Web Data"),
        (h.join("Library/Cookies"), "Cookies"),
        (h.join("Library/LaunchAgents"), "Launch Agent"),
        (h.join("Library/Application Scripts"), "Scripts"),
        (h.join("Library/Application Support/CrashReporter"), "Crash Reports"),
        (h.join("Library/Logs/DiagnosticReports"), "Crash Reports"),
        (PathBuf::from("/Library/Application Support"), "Application Support (all users)"),
        (PathBuf::from("/Library/Caches"), "Cache (all users)"),
        (PathBuf::from("/Library/Preferences"), "Preferences (all users)"),
        (PathBuf::from("/Library/LaunchAgents"), "Launch Agent (all users)"),
        (PathBuf::from("/Library/LaunchDaemons"), "Launch Daemon"),
        (PathBuf::from("/Library/PrivilegedHelperTools"), "Privileged Helper"),
        (PathBuf::from("/Library/Logs"), "Logs (all users)"),
    ]
}

/// How strongly an entry name ties to an app. Bundle-ID matches are exact;
/// name matches need the whole name to avoid "Notes" matching "NotesPlus".
fn matches(entry: &str, app: &AppInfo) -> bool {
    let e = entry.to_lowercase();
    let id = app.bundle_id.to_lowercase();
    let stem = e
        .trim_end_matches(".plist")
        .trim_end_matches(".savedstate")
        .trim_end_matches(".binarycookies")
        .trim_end_matches(".ips")
        .trim_end_matches(".crash");
    if stem == id || stem.starts_with(&format!("{id}.")) {
        return true;
    }
    // Group containers are "<TEAMID>.<bundle id or group>".
    if let Some((_, rest)) = stem.split_once('.') {
        if rest == id || rest.starts_with(&format!("{id}.")) {
            return true;
        }
    }
    let name = app.name.to_lowercase();
    name.len() >= 4 && (stem == name || stem.starts_with(&format!("{name}_")) || stem.starts_with(&format!("{name}-20")))
}

pub fn leftovers(app: &AppInfo, cancel: &Cancel) -> Vec<Leftover> {
    let mut found: Vec<Leftover> = search_roots()
        .par_iter()
        .flat_map_iter(|(dir, kind)| {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| matches(&e.file_name().to_string_lossy(), app))
                .map(|e| e.path())
                .filter(|p| !fsutil::is_protected(p))
                .map(|p| Leftover { size: fsutil::size_of(&p, cancel), path: p, kind })
                .collect::<Vec<_>>()
        })
        .collect();
    found.sort_by(|a, b| b.size.cmp(&a.size));
    found
}

pub fn is_running(app: &AppInfo) -> bool {
    let exe_dir = app.path.join("Contents/MacOS");
    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| p.exe().is_some_and(|e| e.starts_with(&exe_dir)))
}
