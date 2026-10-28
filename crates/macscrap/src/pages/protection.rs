//! Protection: malware scan status, findings, and the engines behind it.

use super::performance::note;
use super::*;
use crate::app::{open_path, reveal, MacScrap};
use gpui::{Context, FontWeight, SharedString};
use macscrap_core::clean::Method;
use macscrap_core::malware::Severity;

const CLAMAV_INSTALL: &str = if cfg!(target_os = "linux") { "sudo apt install clamav" } else { "brew install clamav && freshclam" };

impl MacScrap {
    pub fn render_protection(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let actions = if self.protection.is_running() {
            button("pr-stop", Kind::Secondary, Some("stop"), "Stop").on_click(cx.listener(|this, _, _, cx| {
                this.protection.cancel();
                cx.notify();
            }))
        } else {
            button("pr-scan-h", Kind::Secondary, Some(if self.protection.done().is_some() { "refresh" } else { "scan" }), if self.protection.done().is_some() { "Scan Again" } else { "Scan" })
                .on_click(cx.listener(|this, _, _, cx| this.start_protection_scan(cx)))
        };
        let head = header("Protection", if cfg!(target_os = "linux") { "Scan for threats using your YARA rules and ClamAV." } else { "Checks the places malware hides using Apple's XProtect signatures." }, actions);

        if self.protection.is_running() {
            return page(head, None, scanning_view("Scanning for malware", running_detail(self.protection.progress(), "files checked"), None), None);
        }

        let mut body = scroll_body("protection").gap(px(16.));
        body = body.child(self.protection_status(cx));

        if let Some(result) = self.protection.done() {
            if let Some(err) = &result.error {
                body = body.child(note("alert", err));
            }
            if !result.threats.is_empty() {
                let gen = self.protection.generation();
                let threats = result.threats.clone();
                let rows: Vec<AnyElement> = threats
                    .into_iter()
                    .enumerate()
                    .map(|(ix, t)| {
                        let (color, tile) = match t.severity {
                            Severity::Malicious => (pl.red, "shield-alert"),
                            Severity::Suspicious => (pl.orange, "alert"),
                            Severity::Info => (pl.text2, "info"),
                        };
                        let (rv, rm) = (t.path.clone(), t.path.clone());
                        let label = match t.severity {
                            Severity::Malicious => "Malware",
                            Severity::Suspicious => "Suspicious",
                            Severity::Info => "Worth a look",
                        };
                        fade_in(
                            list_row(SharedString::from(format!("th-{ix}")))
                                .child(tile_icon(tile, color, 30.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .flex()
                                        .flex_col()
                                        .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(t.name.clone()))
                                        .child(text(CAPTION, pl.text3).truncate().child(short_path(&t.path))),
                                )
                                .child(pill(label, color))
                                .child(pill(t.source, pl.text2))
                                .child(icon_button(SharedString::from(format!("trv-{ix}")), "folder").on_click(move |_, _, _| reveal(&rv)))
                                .when(t.severity != Severity::Info, |d| {
                                    d.child(button(SharedString::from(format!("tq-{ix}")), Kind::Secondary, Some("trash"), "Remove").on_click(cx.listener(
                                        move |this, _, _, cx| {
                                            let size = macscrap_core::fsutil::size_of(&rm, &macscrap_core::Cancel::new());
                                            this.ask("Move this item to the Trash?", "Move", vec![(rm.clone(), size, Method::Trash)], |this, _| {
                                                if let Some(r) = this.protection.done_mut() {
                                                    r.threats.retain(|t| t.path.exists());
                                                }
                                            }, cx);
                                        },
                                    )))
                                }),
                            format!("th-{gen}-{ix}"),
                            ix,
                        )
                    })
                    .collect();
                body = body.child(section_label("Findings").px(px(4.))).child(grouped(rows));
            }
        }

        body = body.child(section_label("Detection engines").px(px(4.))).child(self.engines(cx));
        body = body.child(section_label("What gets scanned").px(px(4.))).child(
            div().flex().flex_wrap().gap(px(6.)).children(macscrap_core::malware::quick_scan_roots().into_iter().map(|r| {
                div()
                    .h(px(26.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(7.))
                    .bg(c(pl.surface2))
                    .border_1()
                    .border_color(c(pl.border))
                    .child(icon("folder", pl.text3, 12.))
                    .child(text(CAPTION, pl.text2).child(short_path(&r)))
            })),
        );
        page(head, None, body, None)
    }

    fn protection_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let (tile, color, title, subtitle) = match self.protection.done() {
            None => ("shield", pl.text2, "Not scanned yet".to_string(), "A quick scan takes about 15 seconds.".to_string()),
            Some(r) if r.malicious() > 0 => (
                "shield-alert",
                pl.red,
                format!("{} {} found", r.malicious(), if r.malicious() == 1 { "threat" } else { "threats" }),
                "Review the findings below and remove anything you don't recognize.".to_string(),
            ),
            Some(r) if r.threats.iter().any(|t| t.severity == Severity::Suspicious) => (
                "alert", pl.orange, "Findings need review".to_string(),
                "Review the findings below; a match is not proof of malware.".to_string(),
            ),
            Some(r) if r.error.is_some() => ("alert", pl.orange, "Scan incomplete".to_string(),
                r.error.clone().unwrap_or_default()),
            Some(r) => (
                "shield-check",
                pl.green,
                "No threats found".to_string(),
                if r.clamav {
                    "ClamAV deep scan found nothing harmful.".to_string()
                } else {
                    format!("{} files checked against {} signatures.", fmt_count(r.scanned), r.rule_count)
                },
            ),
        };
        let when = self.last_protection_scan.map(|t| {
            let m = t.elapsed().as_secs() / 60;
            if m == 0 { "Scanned just now".to_string() } else { format!("Scanned {m} min ago") }
        });
        card()
            .p(px(20.))
            .flex()
            .items_center()
            .gap(px(16.))
            .child(tile_icon(tile, color, 48.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(text(17., pl.text).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(text(BODY, pl.text2).child(subtitle))
                    .children(when.map(|w| text(CAPTION, pl.text3).child(w))),
            )
            .when(self.protection.done().is_none(), |d| {
                d.child(button("pr-scan", Kind::Primary, Some("shield"), "Quick Scan").on_click(cx.listener(|this, _, _, cx| this.start_protection_scan(cx))))
            })
    }

    fn engines(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let clamav = macscrap_core::malware::clamav_path().is_some();
        let engine_row = |id: &str, icon_name: &str, color: u32, title: &str, detail: &str| {
            list_row(SharedString::from(id.to_string()))
                .py(px(10.))
                .child(tile_icon(icon_name, color, 32.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).child(title.to_string()))
                        .child(text(CAPTION, pl.text2).child(detail.to_string())),
                )
        };
        grouped(vec![
            engine_row("eng-xp", "shield-check", if cfg!(target_os = "linux") { pl.text2 } else { pl.green },
                if cfg!(target_os = "linux") { "Linux signatures" } else { "Apple XProtect signatures" },
                if cfg!(target_os = "linux") { "A small set of known-family YARA rules is bundled. Add rules or use ClamAV for broader coverage." } else { "Read from macOS and updated automatically with system updates. Matched with the YARA-X engine." })
                .child(pill(if cfg!(target_os = "linux") { "Limited" } else { "Active" }, if cfg!(target_os = "linux") { pl.text2 } else { pl.green }))
                .into_any_element(),
            engine_row("eng-yara", "code", pl.indigo, "Your YARA rules", "Add .yar files to use community rules such as YARA-Forge or Elastic.")
                .child(button("open-yara", Kind::Secondary, Some("folder"), "Open Folder").on_click(|_, _, _| {
                    let dir = macscrap_core::malware::user_yara_dir();
                    let _ = std::fs::create_dir_all(&dir);
                    open_path(&dir);
                }))
                .into_any_element(),
            if clamav {
                engine_row("eng-clam", "scan", pl.teal, "ClamAV deep scan", "Open-source antivirus with millions of signatures. Slower, more thorough.")
                    .child(button("clam", Kind::Secondary, Some("scan"), "Deep Scan").on_click(cx.listener(|this, _, _, cx| this.start_clamav_scan(cx))))
                    .into_any_element()
            } else {
                engine_row("eng-clam", "scan", pl.text3, "ClamAV deep scan", if cfg!(target_os = "linux") { "Optional. Install ClamAV with your distribution's package manager and update its signatures." } else { "Optional. Install it with Homebrew to enable a deeper scan." })
                    .child(
                        div()
                            .id("copy-clam")
                            .h(px(28.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .rounded(px(7.))
                            .bg(c(pl.surface2))
                            .border_1()
                            .border_color(c(pl.border))
                            .cursor_pointer()
                            .hover(|s| s.border_color(c(pl.border_strong)))
                            .child(text(CAPTION, pl.text2).font_family(if cfg!(target_os = "linux") { "monospace" } else { "Menlo" }).child(CLAMAV_INSTALL))
                            .child(icon("clipboard", pl.text3, 13.))
                            .on_click(cx.listener(|this, _, _, cx| this.copy(CLAMAV_INSTALL, cx))),
                    )
                    .into_any_element()
            },
        ])
    }
}
