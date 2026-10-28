//! Overview: Smart Scan (one click to check everything) and live health.

use super::*;
use crate::app::{level_color, MacScrap, Page};
use gpui::{Context, FontWeight, SharedString, Window};
use macscrap_core::clean::Method;
use macscrap_core::health::{Level, Snapshot};
use macscrap_core::human_bytes;

impl MacScrap {
    pub fn render_overview(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let mut body = scroll_body("overview").gap(px(20.));

        for (msg, color) in self.alerts() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(14.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .bg(tint(color))
                    .child(icon("alert", color, 16.))
                    .child(text(BODY, pl.text).child(msg)),
            );
        }

        body = body.child(self.hero(cx));
        if self.smart_scan_started {
            body = body.child(self.result_tiles(cx));
        }
        if let Some(s) = self.health.clone() {
            body = body.child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .pt(px(4.))
                    .child(section_label("Health"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(net_stat("arrow-up", s.net_up))
                            .child(net_stat("arrow-down", s.net_down)),
                    ),
            );
            body = body.child(
                div()
                    .grid()
                    .grid_cols(2)
                    .gap(px(12.))
                    .child(storage_card(&s))
                    .child(memory_card(&s))
                    .child(cpu_card(&s, &self.cpu_history))
                    .child(power_card(&s)),
            );
        }

        page(
            header(
                "Overview",
                self.health.as_ref().map(|s| model_name(&s.model)).unwrap_or("Your Mac"),
                div(),
            ),
            None,
            body,
            None,
        )
    }

    fn safe_junk(&self) -> (u64, Vec<(std::path::PathBuf, u64, Method)>) {
        let items = self.selected_junk_items();
        (items.iter().map(|i| i.1).sum(), items)
    }

    fn hero(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let running = self.smart_scan_running();
        let (safe, _) = self.safe_junk();
        let threats = self.protection.done().map(|t| t.malicious()).unwrap_or(0);

        let (tile, color, title, subtitle): (&str, u32, String, String) = if !self.smart_scan_started {
            (
                "scan",
                pl.accent,
                "Let's check on your Mac".into(),
                "Smart Scan looks for junk, malware, heavy background apps and large files. Nothing is removed until you review it.".into(),
            )
        } else if running {
            let phase = self.junk_phase.lock().unwrap().clone();
            ("scan", pl.accent, "Scanning your Mac…".into(), if phase.is_empty() { "Getting started".into() } else { format!("Checking {phase}") })
        } else if threats > 0 {
            ("shield-alert", pl.red, format!("{threats} {} need your attention", if threats == 1 { "threat" } else { "threats" }), "Review them in Protection.".into())
        } else if safe > 0 {
            ("sparkles", pl.green, format!("{} can be freed safely", human_bytes(safe)), "Caches, logs and leftovers apps rebuild on their own. No malware found.".into())
        } else {
            ("check-circle", pl.green, "Your Mac is in great shape".into(), "No junk worth cleaning and no malware found.".into())
        };

        let actions = if running {
            div().child(button("hero-scanning", Kind::Secondary, None, "Scanning…").child(spinner(13., pl.text2)).cursor_default())
        } else if self.smart_scan_started {
            div()
                .flex()
                .gap(px(8.))
                .child(button("hero-rescan", Kind::Secondary, Some("refresh"), "Scan Again").on_click(cx.listener(|this, _, _, cx| this.smart_scan(cx))))
                .when(safe > 0, |d| {
                    d.child(button("hero-clean", Kind::Primary, Some("sparkles"), format!("Clean {}", human_bytes(safe))).on_click(cx.listener(
                        |this, _, _, cx| {
                            let (_, items) = this.safe_junk();
                            this.ask("Clean junk files?", "Clean", items, |this, _| this.prune_removed_junk(), cx);
                        },
                    )))
                })
        } else {
            div().child(button("hero-scan", Kind::Primary, Some("scan"), "Smart Scan").on_click(cx.listener(|this, _, _, cx| this.smart_scan(cx))))
        };

        card()
            .p(px(20.))
            .flex()
            .items_center()
            .gap(px(16.))
            .child(if running {
                div()
                    .size(px(48.))
                    .flex_none()
                    .rounded(px(13.))
                    .bg(tint(pl.accent))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(spinner(22., pl.accent))
            } else {
                tile_icon(tile, color, 48.)
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(17., pl.text).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(text(BODY, pl.text2).child(subtitle)),
            )
            .child(actions)
    }

    fn result_tiles(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let heavy = self.processes.done().map(|p| p.iter().filter(|p| p.cpu >= 30.0).count());
        let startup = self.launch.done().map(|l| l.iter().filter(|i| !i.is_apple()).count());
        let large: Option<(usize, u64)> = self.large.done().map(|f| {
            let big: Vec<_> = f.iter().filter(|f| f.size >= 500_000_000).collect();
            (big.len(), big.iter().map(|f| f.size).sum())
        });

        let tiles: [(Page, &str, u32, &str, Option<(String, String)>, bool); 4] = [
            (
                Page::Cleanup,
                "sparkles",
                pl.accent,
                "Cleanup",
                self.junk.done().map(|g| (human_bytes(g.iter().map(|g| g.size()).sum()), "of junk found".into())),
                self.junk.is_running(),
            ),
            (
                Page::Protection,
                "shield",
                pl.green,
                "Protection",
                self.protection.done().map(|t| match t.malicious() {
                    0 => ("No threats".into(), format!("{} files checked", fmt_count(t.scanned))),
                    n => (format!("{n} threats"), "need review".into()),
                }),
                self.protection.is_running(),
            ),
            (
                Page::Performance,
                "activity",
                pl.orange,
                "Performance",
                heavy.zip(startup).map(|(h, s)| {
                    (
                        if h == 0 { "All calm".into() } else { format!("{h} busy {}", if h == 1 { "app" } else { "apps" }) },
                        format!("{s} startup items"),
                    )
                }),
                self.processes.is_running(),
            ),
            (
                Page::LargeFiles,
                "file",
                pl.indigo,
                "Large Files",
                large.map(|(n, size)| (format!("{n} files"), format!("{} over 500 MB", human_bytes(size)))),
                self.large.is_running(),
            ),
        ];

        div().grid().grid_cols(4).gap(px(12.)).children(tiles.into_iter().enumerate().map(|(ix, (page, icon_name, color, label, value, running))| {
            let gen = match page {
                Page::Cleanup => self.junk.generation(),
                Page::Protection => self.protection.generation(),
                Page::Performance => self.processes.generation(),
                _ => self.large.generation(),
            };
            let content = card()
                .id(SharedString::from(format!("tile-{label}")))
                .p(px(14.))
                .flex()
                .flex_col()
                .gap(px(10.))
                .cursor_pointer()
                .hover(move |s| s.border_color(c(pl.border_strong)).bg(c(pl.surface2)))
                .on_click(cx.listener(move |this, _, window, cx| this.go(page, window, cx)))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .child(div().flex().items_center().gap(px(8.)).child(tile_icon(icon_name, color, 24.)).child(text(CAPTION, pl.text2).child(label)))
                        .child(if running { spinner(13., pl.text3).into_any_element() } else { icon("chevron-right", pl.text3, 14.).into_any_element() }),
                )
                .child(match value {
                    Some((v, caption)) => fade_in(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(text(18., pl.text).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child(v))
                            .child(text(CAPTION, pl.text3).child(caption)),
                        format!("tile-{label}-{gen}"),
                        ix,
                    ),
                    None => div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(18., pl.text3).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child("—"))
                        .child(text(CAPTION, pl.text3).child(if running { "Checking…" } else { "Not scanned" }))
                        .into_any_element(),
                });
            content
        }))
    }
}

fn model_name(id: &str) -> &'static str {
    // hw.model identifiers → marketing family; exact model years aren't needed here.
    if id.starts_with("MacBookPro") {
        "MacBook Pro"
    } else if id.starts_with("MacBookAir") {
        "MacBook Air"
    } else if id.starts_with("Macmini") {
        "Mac mini"
    } else if id.starts_with("iMac") {
        "iMac"
    } else if id.starts_with("MacStudio") || id.starts_with("Mac1") {
        "Mac Studio"
    } else if id.starts_with("MacPro") {
        "Mac Pro"
    } else {
        "Your Mac"
    }
}

fn net_stat(icon_name: &str, bytes: u64) -> impl IntoElement {
    let pl = p();
    div()
        .flex()
        .items_center()
        .gap(px(4.))
        .child(icon(icon_name, pl.text3, 12.))
        .child(text(CAPTION, pl.text2).child(format!("{}/s", human_bytes(bytes))))
}

fn health_card(icon_name: &str, title: &str, status: Option<(String, u32)>) -> Div {
    let pl = p();
    card().p(px(16.)).flex().flex_col().gap(px(12.)).child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .child(div().flex().items_center().gap(px(8.)).child(icon(icon_name, pl.text2, 16.)).child(strong(BODY, pl.text).child(title.to_string())))
            .children(status.map(|(label, color)| pill(label, color))),
    )
}

fn big_number(value: String, unit: &str) -> Div {
    let pl = p();
    div()
        .flex()
        .items_baseline()
        .gap(px(5.))
        .child(text(26., pl.text).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child(value))
        .child(text(BODY, pl.text2).child(unit.to_string()))
}

fn level_status(level: Level, good: &str, warn: &str, bad: &str) -> (String, u32) {
    let label = match level {
        Level::Good => good,
        Level::Warn => warn,
        Level::Critical => bad,
    };
    (label.to_string(), level_color(level))
}

fn split_size(bytes: u64) -> (String, String) {
    let s = human_bytes(bytes);
    match s.split_once(' ') {
        Some((v, u)) => (v.to_string(), u.to_string()),
        None => (s, String::new()),
    }
}

fn storage_card(s: &Snapshot) -> Div {
    let pl = p();
    let used = s.disk_total.saturating_sub(s.disk_free);
    let (v, u) = split_size(s.disk_free);
    health_card("disk", "Storage", Some(level_status(s.disk_level(), "Plenty of space", "Getting full", "Almost full")))
        .child(big_number(v, &format!("{u} available")))
        .child(meter(used as f32 / s.disk_total.max(1) as f32, if s.disk_level() == Level::Good { pl.accent } else { level_color(s.disk_level()) }))
        .child(text(CAPTION, pl.text3).child(format!("{} used of {}", human_bytes(used), human_bytes(s.disk_total))))
}

fn memory_card(s: &Snapshot) -> Div {
    let pl = p();
    health_card("memory", "Memory", Some(level_status(s.mem_level(), "Normal", "Elevated", "High")))
        .child(big_number(s.mem_pressure.to_string(), "% pressure"))
        .child(meter(s.mem_pressure as f32 / 100.0, if s.mem_level() == Level::Good { pl.green } else { level_color(s.mem_level()) }))
        .child(text(CAPTION, pl.text3).child(format!(
            "{} of {} in use · {} swap",
            human_bytes(s.mem_used),
            human_bytes(s.mem_total),
            human_bytes(s.swap_used)
        )))
}

fn cpu_card(s: &Snapshot, history: &std::collections::VecDeque<f32>) -> Div {
    let pl = p();
    let status = s.cpu_temp.map(|t| {
        let (_, color) = level_status(s.temp_level(), "", "", "");
        (format!("{t:.0}°C"), color)
    });
    // Sparkline of the last ~70 s of load, drawn as bars (no gradients).
    let spark = div().h(px(28.)).flex().items_end().gap(px(2.)).children(history.iter().map(|&v| {
        div()
            .flex_1()
            .rounded(px(1.))
            .h(px((v / 100.0 * 28.0).clamp(2.0, 28.0)))
            .bg(if v >= 80.0 { c(pl.orange) } else { c(pl.accent).opacity(0.55) })
    }));
    health_card("cpu", "CPU", status)
        .child(big_number(format!("{:.0}", s.cpu_load), "% load"))
        .child(spark)
        .child(text(CAPTION, if s.temp_level() == Level::Critical { pl.red } else { pl.text3 }).child(match s.temp_level() {
            Level::Critical => "Running hot. See Performance for busy apps.",
            Level::Warn => "Warm, but within normal range",
            Level::Good => "Temperature normal",
        }))
}

fn power_card(s: &Snapshot) -> Div {
    let pl = p();
    match &s.battery {
        Some(b) => {
            let low = b.percent < 15 && !b.charging;
            health_card(
                if b.charging { "bolt" } else { "battery" },
                "Battery",
                Some(if b.charging { ("Charging".into(), pl.green) } else if low { ("Low".into(), pl.red) } else { ("On battery".into(), pl.text2) }),
            )
            .child(big_number(b.percent.to_string(), "%"))
            .child(meter(b.percent as f32 / 100.0, if low { pl.red } else { pl.green }))
            .child(text(CAPTION, pl.text3).child(match (&b.remaining, b.charging) {
                (Some(r), false) => format!("About {r} remaining"),
                (Some(r), true) => format!("Full in about {r}"),
                _ => "Estimating…".into(),
            }))
        }
        None => health_card("bolt", "Power", Some(("Connected".into(), pl.green)))
            .child(big_number("AC".into(), "power"))
            .child(text(CAPTION, pl.text3).child("This Mac has no battery")),
    }
}
