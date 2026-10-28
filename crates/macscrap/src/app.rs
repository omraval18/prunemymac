//! Application state, background jobs, and the window shell (sidebar,
//! confirmation sheet, toast). Page bodies live in `pages/`.

use crate::theme::*;
use gpui::prelude::*;
use gpui::{
    div, img, px, ClipboardItem, Context, FocusHandle, FontWeight, KeyDownEvent, MouseButton, SharedString, Task,
    Window, WindowAppearance,
};
use macscrap_core::apps::{AppInfo, Leftover};
use macscrap_core::background::{LaunchItem, ProcessInfo};
use macscrap_core::clean::{CleanReport, Method};
use macscrap_core::files::{DuplicateSet, FileEntry};
use macscrap_core::health::{Level, Monitor, Snapshot};
use macscrap_core::junk::{JunkGroup, Safety};
use macscrap_core::malware::{Severity, Threat};
use macscrap_core::network::{Connection, Port};
use macscrap_core::{human_bytes, Cancel};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Overview,
    Cleanup,
    SpaceLens,
    LargeFiles,
    Duplicates,
    Uninstaller,
    Performance,
    Network,
    Protection,
}

impl Page {
    pub fn title(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::Cleanup => "Cleanup",
            Page::SpaceLens => "Space Lens",
            Page::LargeFiles => "Large & Old Files",
            Page::Duplicates => "Duplicates",
            Page::Uninstaller => "Uninstaller",
            Page::Performance => "Performance",
            Page::Network => "Network",
            Page::Protection => "Protection",
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Page::Overview => "overview",
            Page::Cleanup => "sparkles",
            Page::SpaceLens => "pie",
            Page::LargeFiles => "file",
            Page::Duplicates => "copy",
            Page::Uninstaller => "grid",
            Page::Performance => "activity",
            Page::Network => "wifi",
            Page::Protection => "shield",
        }
    }
}

const NAV: [(&str, &[Page]); 4] = [
    ("", &[Page::Overview]),
    ("Clean", &[Page::Cleanup, Page::SpaceLens, Page::LargeFiles, Page::Duplicates]),
    ("Apps", &[Page::Uninstaller, Page::Performance]),
    ("Security", &[Page::Protection, Page::Network]),
];

static GENERATION: AtomicU64 = AtomicU64::new(1);

/// A background job's lifecycle. `Done` carries a generation number so
/// result lists can replay their entrance animation only for new results.
#[derive(Default)]
pub enum Job<T> {
    #[default]
    Idle,
    Running { started: Instant, cancel: Cancel, progress: Arc<AtomicU64> },
    Done(T, u64),
}

impl<T> Job<T> {
    pub fn is_running(&self) -> bool {
        matches!(self, Job::Running { .. })
    }
    pub fn done(&self) -> Option<&T> {
        match self {
            Job::Done(t, _) => Some(t),
            _ => None,
        }
    }
    pub fn done_mut(&mut self) -> Option<&mut T> {
        match self {
            Job::Done(t, _) => Some(t),
            _ => None,
        }
    }
    pub fn generation(&self) -> u64 {
        match self {
            Job::Done(_, g) => *g,
            _ => 0,
        }
    }
    pub fn progress(&self) -> Option<(u64, u64)> {
        match self {
            Job::Running { started, progress, .. } => Some((progress.load(Ordering::Relaxed), started.elapsed().as_secs())),
            _ => None,
        }
    }
    pub fn cancel(&self) {
        if let Job::Running { cancel, .. } = self {
            cancel.cancel();
        }
    }
}

/// A destructive action waiting for the user's go-ahead.
pub struct Confirm {
    pub title: String,
    pub verb: String,
    pub items: Vec<(PathBuf, u64, Method)>,
    pub on_done: fn(&mut MacScrap, &CleanReport),
    pub opened: Instant,
}

/// A clean in progress (or just finished), shown in the sheet.
pub struct Cleaning {
    pub title: String,
    pub has_trash: bool,
    pub total_items: usize,
    pub total_bytes: u64,
    pub progress: Arc<macscrap_core::clean::Progress>,
    pub started: Instant,
    /// Displayed ring fraction; eases toward real progress every frame.
    pub shown: f32,
    pub report: Option<CleanReport>,
    pub completed_at: Option<Instant>,
}

#[derive(Default)]
pub struct Threats {
    pub rule_count: usize,
    pub threats: Vec<Threat>,
    pub error: Option<String>,
    pub clamav: bool,
    pub scanned: u64,
}

impl Threats {
    pub fn malicious(&self) -> usize {
        self.threats.iter().filter(|t| t.severity == Severity::Malicious).count()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LargeSort {
    Size,
    Unused,
}

/// What Space Lens maps.
#[derive(Clone, PartialEq, Eq)]
pub enum LensScope {
    Home,
    Mac,
    Folder(PathBuf),
}

impl LensScope {
    pub fn root(&self) -> PathBuf {
        match self {
            LensScope::Home => macscrap_core::fsutil::home(),
            LensScope::Mac => macscrap_core::space::whole_mac_root(),
            LensScope::Folder(p) => p.clone(),
        }
    }
}

pub struct LensTree {
    pub scope: LensScope,
    pub root: PathBuf,
    pub tree: macscrap_core::space::Dir,
    /// For the whole-Mac scope: bytes used on disk that the scan can't see
    /// (system volume, snapshots, other APFS volumes).
    pub hidden: u64,
}

pub struct Toast {
    pub message: String,
    pub ok: bool,
    pub at: Instant,
}

pub struct MacScrap {
    pub page: Page,
    pub focus: FocusHandle,
    pub search_focus: FocusHandle,

    pub health: Option<Snapshot>,
    pub cpu_history: VecDeque<f32>,
    hot_since: Option<Instant>,
    last_hot_notice: Option<Instant>,

    pub smart_scan_started: bool,

    pub junk: Job<Vec<JunkGroup>>,
    pub junk_items: HashSet<PathBuf>,
    pub junk_expanded: HashSet<String>,
    pub junk_phase: Arc<Mutex<String>>,

    pub lens_scope: LensScope,
    pub lens: Job<LensTree>,
    pub lens_chain: Vec<usize>,
    pub lens_selected: HashSet<PathBuf>,
    pub lens_scroll: gpui::UniformListScrollHandle,

    pub large_min: u64,
    pub large_sort: LargeSort,
    pub large_unused_only: bool,
    pub large: Job<Vec<FileEntry>>,
    pub large_selected: HashSet<PathBuf>,

    pub dupes: Job<Vec<DuplicateSet>>,
    pub dupes_selected: HashSet<PathBuf>,

    pub apps: Job<Vec<AppInfo>>,
    pub app_query: String,
    pub selected_app: Option<String>,
    pub leftovers: Job<Vec<Leftover>>,
    pub leftovers_selected: HashSet<PathBuf>,

    pub perf_tab: usize,
    pub processes: Job<Vec<ProcessInfo>>,
    pub process_groups_expanded: HashSet<String>,
    pub launch: Job<Vec<LaunchItem>>,
    pub maintenance_running: Option<&'static str>,

    pub network: Option<Result<Vec<Connection>, String>>,
    pub network_running: bool,
    pub network_review_only: bool,
    network_last_refresh: Option<Instant>,
    pub network_tab: usize,
    pub network_ports: Option<Result<Vec<Port>, String>>,
    pub network_ports_running: bool,
    network_ports_last_refresh: Option<Instant>,
    pub port_confirm: Option<Port>,
    pub port_stop_pending: bool,
    #[cfg(target_os = "linux")]
    pub flatpak_confirm: Option<AppInfo>,
    #[cfg(target_os = "linux")]
    pub flatpak_uninstall_pending: bool,

    pub protection: Job<Threats>,
    pub last_protection_scan: Option<Instant>,

    pub confirm: Option<Confirm>,
    pub cleaning: Option<Cleaning>,
    pub toast: Option<Toast>,
    _ticker: Task<()>,
}

impl MacScrap {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        set_dark(is_dark_appearance(window.appearance()));
        cx.observe_window_appearance(window, |_, window, cx| {
            set_dark(is_dark_appearance(window.appearance()));
            cx.notify();
        })
        .detach();

        let monitor = Arc::new(Mutex::new(Monitor::new()));
        let ticker = cx.spawn(async move |this, cx| loop {
            let m = monitor.clone();
            let snap = cx.background_executor().spawn(async move { m.lock().unwrap().sample() }).await;
            if this.update(cx, |this, cx| this.on_health(snap, cx)).is_err() {
                break;
            }
            cx.background_executor().timer(Duration::from_millis(1500)).await;
        });

        let focus = cx.focus_handle();
        window.focus(&focus);
        let mut app = Self {
            page: Page::Overview,
            focus,
            search_focus: cx.focus_handle(),
            health: None,
            cpu_history: VecDeque::with_capacity(48),
            hot_since: None,
            last_hot_notice: None,
            smart_scan_started: false,
            junk: Job::Idle,
            junk_items: HashSet::new(),
            junk_expanded: HashSet::new(),
            junk_phase: Arc::default(),
            lens_scope: LensScope::Home,
            lens: Job::Idle,
            lens_chain: vec![],
            lens_selected: HashSet::new(),
            lens_scroll: gpui::UniformListScrollHandle::new(),
            large_min: 500_000_000,
            large_sort: LargeSort::Size,
            large_unused_only: false,
            large: Job::Idle,
            large_selected: HashSet::new(),
            dupes: Job::Idle,
            dupes_selected: HashSet::new(),
            apps: Job::Idle,
            app_query: String::new(),
            selected_app: None,
            leftovers: Job::Idle,
            leftovers_selected: HashSet::new(),
            perf_tab: 0,
            processes: Job::Idle,
            process_groups_expanded: HashSet::new(),
            launch: Job::Idle,
            maintenance_running: None,
            network: None,
            network_running: false,
            network_review_only: false,
            network_last_refresh: None,
            network_tab: 0,
            network_ports: None,
            network_ports_running: false,
            network_ports_last_refresh: None,
            port_confirm: None,
            port_stop_pending: false,
            #[cfg(target_os = "linux")]
            flatpak_confirm: None,
            #[cfg(target_os = "linux")]
            flatpak_uninstall_pending: false,
            protection: Job::Idle,
            last_protection_scan: None,
            confirm: None,
            cleaning: None,
            toast: None,
            _ticker: ticker,
        };
        app.load_performance(cx);
        app
    }

    fn on_health(&mut self, snap: Snapshot, cx: &mut Context<Self>) {
        // Notify only for sustained heat, so a short compile burst isn't an alert.
        if snap.temp_level() == Level::Critical {
            let since = *self.hot_since.get_or_insert_with(Instant::now);
            let quiet = self.last_hot_notice.is_none_or(|t| t.elapsed() > Duration::from_secs(600));
            if since.elapsed() > Duration::from_secs(30) && quiet {
                self.last_hot_notice = Some(Instant::now());
                notify(&format!(
                    "Your computer is running hot ({:.0}°C). Open prunemymac → Performance to see what's using the CPU.",
                    snap.cpu_temp.unwrap_or_default()
                ));
            }
        } else {
            self.hot_since = None;
        }
        if self.cpu_history.len() == 48 {
            self.cpu_history.pop_front();
        }
        self.cpu_history.push_back(snap.cpu_load);
        self.health = Some(snap);
        if self.page == Page::Network && self.network_tab == 0 && !self.network_running
            && self.network_last_refresh.is_none_or(|last| last.elapsed() >= Duration::from_secs(5)) {
            self.refresh_network(cx);
        }
        if self.page == Page::Network && self.network_tab == 1 && !self.network_ports_running && !self.port_stop_pending
            && self.network_ports_last_refresh.is_none_or(|last| last.elapsed() >= Duration::from_secs(5)) {
            self.refresh_ports(cx);
        }
        if self.toast.as_ref().is_some_and(|t| t.at.elapsed() > Duration::from_secs(4)) {
            self.toast = None;
        }
        cx.notify();
    }

    pub fn alerts(&self) -> Vec<(String, u32)> {
        let Some(s) = &self.health else { return vec![] };
        let pl = p();
        let mut out = vec![];
        if let (Some(t), Level::Critical) = (s.cpu_temp, s.temp_level()) {
            out.push((format!("Your computer is running hot at {t:.0}°C. Check Performance for apps using a lot of CPU."), pl.red));
        }
        if s.mem_level() == Level::Critical {
            out.push((format!("Memory pressure is high ({}%). Quitting unused apps will help.", s.mem_pressure.unwrap_or(0)), pl.orange));
        }
        if s.disk_level() != Level::Good {
            out.push((format!("Only {} free on your startup disk. Run Cleanup to free space.", human_bytes(s.disk_free)), pl.orange));
        }
        out
    }

    /// Run `work` on the background pool; store the result in `slot` and then
    /// call `after` on the UI thread (e.g. to apply a default selection).
    pub fn spawn_job<T: Send + 'static>(
        &mut self,
        cx: &mut Context<Self>,
        slot: fn(&mut Self) -> &mut Job<T>,
        work: impl FnOnce(Cancel, Arc<AtomicU64>) -> T + Send + 'static,
        after: fn(&mut Self),
    ) {
        if slot(self).is_running() {
            return;
        }
        let cancel = Cancel::new();
        let progress = Arc::new(AtomicU64::new(0));
        *slot(self) = Job::Running { started: Instant::now(), cancel: cancel.clone(), progress: progress.clone() };
        let task = cx.background_executor().spawn(async move {
            macscrap_core::sys::background_priority();
            work(cancel, progress)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                // The previous result is dropped here; give its pages back.
                *slot(this) = Job::Done(result, GENERATION.fetch_add(1, Ordering::Relaxed));
                after(this);
                cx.notify();
            })
            .ok();
            cx.background_executor().spawn(async { macscrap_core::sys::release_free_memory() }).detach();
        })
        .detach();
        cx.notify();
    }

    // ───────────────────────────── Jobs ─────────────────────────────

    pub fn refresh_network(&mut self, cx: &mut Context<Self>) {
        if self.network_running { return; }
        self.network_running = true;
        self.network_last_refresh = Some(Instant::now());
        let task = cx.background_executor().spawn(async { macscrap_core::network::connections() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.network = Some(result);
                this.network_running = false;
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    pub fn refresh_ports(&mut self, cx: &mut Context<Self>) {
        if self.network_ports_running { return; }
        self.network_ports_running = true;
        self.network_ports_last_refresh = Some(Instant::now());
        let task = cx.background_executor().spawn(async { macscrap_core::network::ports() });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.network_ports = Some(result);
                this.network_ports_running = false;
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    pub fn stop_port_process(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.port_confirm.take() else { return };
        if self.port_stop_pending { return; }
        self.port_stop_pending = true;
        let name = target.process.clone();
        let task = cx.background_executor().spawn(async move { macscrap_core::network::stop_port_process(&target) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.port_stop_pending = false;
                match result {
                    Ok(()) => this.show_toast(format!("Asked {name} to stop"), true),
                    Err(error) => this.show_toast(error, false),
                }
                this.refresh_ports(cx);
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    #[cfg(target_os = "linux")]
    pub fn uninstall_flatpak(&mut self, cx: &mut Context<Self>) {
        let Some(app) = self.flatpak_confirm.take() else { return };
        if self.flatpak_uninstall_pending { return; }
        self.flatpak_uninstall_pending = true;
        let name = app.name.clone();
        let task = cx.background_executor().spawn(async move { macscrap_core::apps::uninstall_user_flatpak(&app) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                this.flatpak_uninstall_pending = false;
                match result {
                    Ok(()) => {
                        this.show_toast(format!("Uninstalled {name}; user data was kept"), true);
                        this.selected_app = None;
                        this.leftovers = Job::Idle;
                        this.leftovers_selected.clear();
                        this.load_apps(cx);
                    }
                    Err(error) => this.show_toast(format!("Could not uninstall {name}: {error}"), false),
                }
                cx.notify();
            }).ok();
        }).detach();
        cx.notify();
    }

    pub fn start_junk_scan(&mut self, cx: &mut Context<Self>) {
        let phase = self.junk_phase.clone();
        self.junk_expanded.clear();
        self.spawn_job(
            cx,
            |s| &mut s.junk,
            move |cancel, _| {
                let rules = macscrap_core::junk::load_rules().unwrap_or_default();
                macscrap_core::junk::scan(&rules, &cancel, &|name| *phase.lock().unwrap() = name.to_string())
            },
            |s| {
                // Pre-select only what is safe to remove without thinking.
                s.junk_items = s
                    .junk
                    .done()
                    .map(|g| {
                        g.iter().filter(|g| g.rule.safety == Safety::Safe).flat_map(|g| g.items.iter().map(|i| i.path.clone())).collect()
                    })
                    .unwrap_or_default();
            },
        );
    }

    pub fn start_lens_scan(&mut self, scope: LensScope, cx: &mut Context<Self>) {
        self.lens_scope = scope.clone();
        self.lens_chain.clear();
        self.lens_selected.clear();
        self.spawn_job(
            cx,
            |s| &mut s.lens,
            move |cancel, n| {
                let root = scope.root();
                let tree = macscrap_core::space::scan(&root, &cancel, &n);
                let hidden = if scope == LensScope::Mac { macscrap_core::space::disk_used().saturating_sub(tree.size) } else { 0 };
                LensTree { scope, root, tree, hidden }
            },
            |_| {},
        );
    }

    pub fn start_large_scan(&mut self, cx: &mut Context<Self>) {
        // Always collect ≥100 MB so threshold changes filter instantly.
        self.spawn_job(
            cx,
            |s| &mut s.large,
            |cancel, n| macscrap_core::files::large_files(&macscrap_core::fsutil::home(), 100_000_000, &cancel, &n),
            |s| s.large_selected.clear(),
        );
    }

    pub fn start_dupes_scan(&mut self, cx: &mut Context<Self>) {
        self.spawn_job(
            cx,
            |s| &mut s.dupes,
            |cancel, n| macscrap_core::files::duplicates(&macscrap_core::fsutil::home(), 1_000_000, &cancel, &n),
            |s| s.dupes_selected.clear(),
        );
    }

    pub fn start_protection_scan(&mut self, cx: &mut Context<Self>) {
        self.spawn_job(
            cx,
            |s| &mut s.protection,
            |cancel, n| {
                // YARA runs in a helper process: its memory is returned on
                // exit, and a hostile file can't crash the app.
                let exe = std::env::current_exe().unwrap_or_default();
                let r = macscrap_core::malware::scan_in_subprocess(&exe, &cancel, &n);
                let mut out = Threats { rule_count: r.rule_count, threats: r.threats, error: r.error, scanned: r.scanned, clamav: false };
                out.threats.extend(macscrap_core::malware::audit_persistence());
                out.threats.sort_by(|a, b| b.severity.cmp(&a.severity));
                out
            },
            |s| s.last_protection_scan = Some(Instant::now()),
        );
    }

    pub fn start_clamav_scan(&mut self, cx: &mut Context<Self>) {
        self.spawn_job(
            cx,
            |s| &mut s.protection,
            |_, _| match macscrap_core::malware::clamav_scan(&macscrap_core::malware::quick_scan_roots()) {
                Ok(threats) => Threats { threats, clamav: true, ..Default::default() },
                Err(e) => Threats { error: Some(e), clamav: true, ..Default::default() },
            },
            |s| s.last_protection_scan = Some(Instant::now()),
        );
    }

    pub fn load_performance(&mut self, cx: &mut Context<Self>) {
        self.spawn_job(cx, |s| &mut s.processes, |_, _| macscrap_core::background::heavy_processes(usize::MAX), |_| {});
        self.spawn_job(cx, |s| &mut s.launch, |_, _| macscrap_core::background::launch_items(), |_| {});
    }

    pub fn load_apps(&mut self, cx: &mut Context<Self>) {
        self.spawn_job(cx, |s| &mut s.apps, |cancel, _| macscrap_core::apps::list(&cancel), |_| {});
    }

    /// One click: everything the Overview summarizes.
    pub fn smart_scan(&mut self, cx: &mut Context<Self>) {
        self.smart_scan_started = true;
        self.start_junk_scan(cx);
        self.start_protection_scan(cx);
        self.start_large_scan(cx);
        self.load_performance(cx);
    }

    pub fn smart_scan_running(&self) -> bool {
        self.junk.is_running() || self.protection.is_running() || self.large.is_running()
    }

    // ───────────────────────────── Confirm / toast ─────────────────────────────

    pub fn ask(
        &mut self,
        title: impl Into<String>,
        verb: impl Into<String>,
        items: Vec<(PathBuf, u64, Method)>,
        on_done: fn(&mut MacScrap, &CleanReport),
        cx: &mut Context<Self>,
    ) {
        if items.is_empty() {
            return;
        }
        self.confirm = Some(Confirm { title: title.into(), verb: verb.into(), items, on_done, opened: Instant::now() });
        cx.notify();
    }

    pub fn run_confirmed(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.confirm.take() else { return };
        let progress = Arc::new(macscrap_core::clean::Progress::default());
        self.cleaning = Some(Cleaning {
            title: confirm.verb.clone(),
            has_trash: confirm.items.iter().any(|item| item.2 == Method::Trash),
            total_items: confirm.items.len(),
            total_bytes: confirm.items.iter().map(|i| i.1).sum(),
            progress: progress.clone(),
            started: Instant::now(),
            shown: 0.0,
            report: None,
            completed_at: None,
        });
        let on_done = confirm.on_done;
        let items = confirm.items;
        let task = cx.background_executor().spawn(async move {
            macscrap_core::sys::background_priority();
            macscrap_core::clean::run_tracked(items, &progress)
        });
        cx.spawn(async move |this, cx| {
            let report = task.await;
            this.update(cx, |this, cx| {
                on_done(this, &report);
                // Other pages may have cached entries for the same paths.
                this.prune_removed_junk();
                if let Some(files) = this.large.done_mut() { files.retain(|f| f.path.exists()); }
                if let Some(sets) = this.dupes.done_mut() {
                    for set in sets.iter_mut() { set.files.retain(|f| f.path.exists()); }
                    sets.retain(|set| set.files.len() > 1);
                }
                if let Some(result) = this.protection.done_mut() { result.threats.retain(|t| t.path.exists()); }
                if this.page != Page::SpaceLens && !this.lens.is_running() { this.lens = Job::Idle; }
                if let Some(c) = this.cleaning.as_mut() {
                    c.report = Some(report);
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }

    pub fn show_toast(&mut self, message: impl Into<String>, ok: bool) {
        self.toast = Some(Toast { message: message.into(), ok, at: Instant::now() });
    }

    pub fn go(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        self.page = page;
        match page {
            Page::Uninstaller => {
                if matches!(self.apps, Job::Idle) {
                    self.load_apps(cx);
                }
                window.focus(&self.search_focus);
            }
            Page::Performance => {
                if !self.processes.is_running() {
                    self.load_performance(cx);
                }
                window.focus(&self.focus);
            }
            Page::Network => {
                if self.network_tab == 0 { self.refresh_network(cx); } else { self.refresh_ports(cx); }
                window.focus(&self.focus);
            }
            _ => window.focus(&self.focus),
        }
        cx.notify();
    }

    // ───────────────────────────── Shell ─────────────────────────────

    /// Live status shown at the right edge of a sidebar item.
    fn nav_meta(&self, page: Page) -> Option<(String, u32)> {
        let pl = p();
        let size = |b: u64| (human_bytes(b), pl.text3);
        match page {
            Page::Cleanup => self.junk.done().map(|g| size(g.iter().map(|g| g.size()).sum())),
            Page::SpaceLens => self.lens.done().map(|t| size(t.tree.size)),
            Page::LargeFiles => self.large.done().map(|f| {
                let n = f.iter().filter(|f| f.size >= self.large_min).count();
                (n.to_string(), pl.text3)
            }),
            Page::Duplicates => self.dupes.done().map(|d| size(d.iter().map(|s| s.wasted()).sum())),
            Page::Protection => self.protection.done().map(|t| {
                let suspicious = t.threats.iter().filter(|t| t.severity == Severity::Suspicious).count();
                if t.malicious() > 0 { (t.malicious().to_string(), pl.red) }
                else if suspicious > 0 { (suspicious.to_string(), pl.orange) }
                else if t.error.is_some() { ("!".to_string(), pl.orange) }
                else { ("✓".to_string(), pl.green) }
            }),
            _ => None,
        }
    }

    fn nav_running(&self, page: Page) -> bool {
        match page {
            Page::Cleanup => self.junk.is_running(),
            Page::SpaceLens => self.lens.is_running(),
            Page::LargeFiles => self.large.is_running(),
            Page::Duplicates => self.dupes.is_running(),
            Page::Protection => self.protection.is_running(),
            Page::Uninstaller => self.apps.is_running(),
            Page::Overview => self.smart_scan_running(),
            Page::Performance => false,
            Page::Network => self.network_running || self.network_ports_running || self.port_stop_pending,
        }
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let mut nav = div().flex().flex_col().gap(px(1.)).px(px(10.));
        for (section, pages) in NAV {
            if !section.is_empty() {
                nav = nav.child(section_label(section).px(px(10.)).pt(px(16.)).pb(px(6.)));
            }
            for &page in pages {
                let active = self.page == page;
                let meta = self.nav_meta(page);
                let running = self.nav_running(page);
                nav = nav.child(
                    div()
                        .id(SharedString::from(page.title()))
                        .h(px(30.))
                        .px(px(10.))
                        .flex()
                        .items_center()
                        .gap(px(9.))
                        .rounded(px(7.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(c(pl.pressed)))
                        .when(!active, |d| d.hover(|s| s.bg(c(pl.hover))))
                        .child(icon(page.icon(), if active { pl.accent } else { pl.text2 }, 16.))
                        .child(
                            text(BODY, if active { pl.text } else { pl.text2 })
                                .flex_1()
                                .when(active, |d| d.font_weight(FontWeight::MEDIUM))
                                .child(page.title()),
                        )
                        .when(running, |d| d.child(spinner(12., pl.text3)))
                        .when_some(meta.filter(|_| !running), |d, (label, color)| d.child(text(MICRO, color).child(label)))
                        .on_click(cx.listener(move |this, _, window, cx| this.go(page, window, cx))),
                );
            }
        }

        div()
            .w(px(228.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(c(pl.sidebar))
            .border_r_1()
            .border_color(c(pl.border))
            .child(
                // Title-bar area: draggable, clears the traffic lights.
                div()
                    .h(px(52.))
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()),
            )
            .child(
                div()
                    .px(px(20.))
                    .pb(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(9.))
                    .child(img("images/chip-cleaner.png").size(px(25.)).rounded(px(7.)))
                    .child(strong(HEADLINE, pl.text).child("prunemymac")),
            )
            .child(nav)
            .child(div().flex_1())
            .children(self.health.as_ref().map(|s| self.sidebar_health(s)))
    }

    fn sidebar_health(&self, s: &Snapshot) -> impl IntoElement {
        let pl = p();
        let row = |icon_name: &str, label: String, color: u32| {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .child(icon(icon_name, pl.text3, 13.))
                .child(text(CAPTION, color).child(label))
        };
        div()
            .m(px(12.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(c(pl.surface2))
            .border_1()
            .border_color(c(pl.border))
            .flex()
            .flex_col()
            .gap(px(7.))
            .child(row("disk", format!("{} free", human_bytes(s.disk_free)), level_text(s.disk_level())))
            .child(row("memory", s.mem_pressure.map(|p| format!("{p}% memory pressure")).unwrap_or_else(|| "Memory pressure unavailable".into()), level_text(s.mem_level())))
            .child(row(
                "thermometer",
                match s.cpu_temp {
                    Some(t) => format!("{t:.0}°C · {:.0}% CPU", s.cpu_load),
                    None => format!("{:.0}% CPU", s.cpu_load),
                },
                level_text(s.temp_level()),
            ))
    }

    fn toast_view(&self) -> Option<impl IntoElement> {
        let t = self.toast.as_ref()?;
        let pl = p();
        Some(
            div().absolute().bottom(px(76.)).left_0().right_0().flex().justify_center().child(fade_in(
                div()
                    .h(px(38.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded_full()
                    .bg(c(pl.surface2))
                    .border_1()
                    .border_color(c(pl.border_strong))
                    .shadow_lg()
                    .child(icon(if t.ok { "check-circle" } else { "alert" }, if t.ok { pl.green } else { pl.orange }, 16.))
                    .child(text(BODY, pl.text).child(t.message.clone())),
                format!("toast-{:?}", t.at),
                0,
            )),
        )
    }

    fn on_key(&mut self, e: &KeyDownEvent, cx: &mut Context<Self>) {
        let key = e.keystroke.key.as_str();
        #[cfg(target_os = "linux")]
        if self.flatpak_confirm.is_some() {
            if key == "escape" { self.flatpak_confirm = None; cx.notify(); }
            return;
        }
        if self.port_confirm.is_some() {
            if key == "escape" { self.port_confirm = None; } else { return; }
        } else if self.confirm.is_some() {
            match key {
                "escape" => self.confirm = None,
                "enter" => self.run_confirmed(cx),
                _ => return,
            }
        } else if self.cleaning.as_ref().is_some_and(|c| c.completed_at.is_some()) && (key == "escape" || key == "enter") {
            self.cleaning = None;
        } else {
            return;
        }
        cx.notify();
    }

    pub fn copy(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        self.show_toast("Copied to clipboard", true);
        cx.notify();
    }
}

impl Render for MacScrap {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let page = match self.page {
            Page::Overview => self.render_overview(window, cx).into_any_element(),
            Page::Cleanup => self.render_cleanup(cx).into_any_element(),
            Page::SpaceLens => self.render_space_lens(cx).into_any_element(),
            Page::LargeFiles => self.render_large(cx).into_any_element(),
            Page::Duplicates => self.render_duplicates(cx).into_any_element(),
            Page::Uninstaller => self.render_uninstaller(window, cx).into_any_element(),
            Page::Performance => self.render_performance(cx).into_any_element(),
            Page::Network => self.render_network(cx).into_any_element(),
            Page::Protection => self.render_protection(cx).into_any_element(),
        };
        div()
            .size_full()
            .relative()
            .flex()
            .bg(c(pl.bg))
            .font_family("Inter")
            .text_size(px(BODY))
            .text_color(c(pl.text))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.on_key(e, cx)))
            .child(self.sidebar(cx))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().h(px(28.)).flex_none().on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move()))
                    .child(page),
            )
            .children(self.toast_view())
            .children(self.sheet(window, cx))
    }
}

fn is_dark_appearance(a: WindowAppearance) -> bool {
    matches!(a, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

pub fn level_color(level: Level) -> u32 {
    let pl = p();
    match level {
        Level::Good => pl.green,
        Level::Warn => pl.orange,
        Level::Critical => pl.red,
    }
}

/// Sidebar text stays neutral when healthy; color only signals a problem.
fn level_text(level: Level) -> u32 {
    match level {
        Level::Good => p().text2,
        other => level_color(other),
    }
}

pub fn notify(message: &str) {
    #[cfg(target_os = "linux")]
    { macscrap_core::sys::spawn_detached(Command::new("notify-send").args(["prunemymac", message])); }
    #[cfg(target_os = "macos")]
    {
    let script = format!("display notification \"{}\" with title \"prunemymac\"", message.replace('"', "'"));
    macscrap_core::sys::spawn_detached(Command::new("/usr/bin/osascript").args(["-e", &script]));
    }
}

pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    { macscrap_core::sys::spawn_detached(Command::new("xdg-open").arg(if path.is_dir() { path } else { path.parent().unwrap_or(path) })); }
    #[cfg(target_os = "macos")]
    macscrap_core::sys::spawn_detached(Command::new("/usr/bin/open").arg("-R").arg(path));
}

pub fn open_path(path: &std::path::Path) {
    #[cfg(target_os = "linux")]
    macscrap_core::sys::spawn_detached(Command::new("xdg-open").arg(path));
    #[cfg(target_os = "macos")]
    macscrap_core::sys::spawn_detached(Command::new("/usr/bin/open").arg(path));
}

/// Run a shell command as root after the standard macOS admin password prompt.
pub fn run_as_admin(shell: &str) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let out = Command::new("pkexec").args(["/bin/sh", "-c", shell]).output().map_err(|e| format!("Could not start administrator authorization: {e}"))?;
    #[cfg(target_os = "macos")]
    let out = {
    let script = format!("do shell script \"{}\" with administrator privileges", shell.replace('"', "\\\""));
    Command::new("/usr/bin/osascript").args(["-e", &script]).output().map_err(|e| e.to_string())?
    };
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}
