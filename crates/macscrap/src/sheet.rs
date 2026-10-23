//! The modal sheet: confirm → cleaning (live ring) → done (check + total).
//! Cleaning is a rare, rewarding moment, so it gets the app's one
//! expressive animation; everything else stays quiet.

use crate::app::MacScrap;
use crate::theme::*;
use gpui::prelude::*;
use gpui::{
    div, px, size, svg, Animation, AnimationExt, AnyElement, Context, FontWeight, MouseButton, Transformation, Window,
};
use macscrap_core::clean::Method;
use macscrap_core::human_bytes;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Shortest time the ring takes to fill, so small cleans still read as work.
const MIN_FILL_SECS: f32 = 1.1;

/// Ease-out with a small overshoot, for the check mark landing.
fn ease_out_back(t: f32) -> f32 {
    let c1 = 1.4;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

fn backdrop(content: impl IntoElement, dismiss: bool, cx: &mut Context<MacScrap>) -> gpui::Div {
    div()
        .absolute()
        .size_full()
        .bg(gpui::hsla(0., 0., 0., if is_dark() { 0.5 } else { 0.25 }))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _, cx| {
                if dismiss {
                    this.confirm = None;
                    if this.cleaning.as_ref().is_some_and(|c| c.completed_at.is_some()) {
                        this.cleaning = None;
                    }
                    cx.notify();
                }
            }),
        )
        .child(content)
}

fn panel(width: f32) -> gpui::Stateful<gpui::Div> {
    let pl = p();
    div()
        .id("sheet")
        .w(px(width))
        .p(px(24.))
        .rounded(px(16.))
        .bg(c(pl.surface))
        .border_1()
        .border_color(c(pl.border_strong))
        .shadow_xl()
        .flex()
        .flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

impl MacScrap {
    pub fn sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.confirm.is_some() {
            return Some(self.confirm_view(cx));
        }
        if self.cleaning.is_some() {
            return Some(self.cleaning_view(window, cx));
        }
        None
    }

    fn confirm_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let confirm = self.confirm.as_ref().unwrap();
        let total: u64 = confirm.items.iter().map(|i| i.1).sum();
        let trash = confirm.items.iter().filter(|i| i.2 == Method::Trash).count();
        let delete = confirm.items.len() - trash;
        let line = |icon_name: &str, body: String| {
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(icon(icon_name, pl.text3, 15.).mt(px(1.)))
                .child(text(BODY, pl.text2).flex_1().min_w_0().child(body))
        };
        let id = format!("confirm-{:?}", confirm.opened);
        fade_in(
            backdrop(
                panel(420.)
                    .gap(px(16.))
                    .child(tile_icon("trash", pl.accent, 40.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(strong(HEADLINE, pl.text).child(confirm.title.clone()))
                            .child(text(BODY, pl.text2).child(format!(
                                "{} {} · {}",
                                confirm.items.len(),
                                if confirm.items.len() == 1 { "item" } else { "items" },
                                human_bytes(total)
                            ))),
                    )
                    .child(
                        div()
                            .w_full()
                            .flex()
                            .flex_col()
                            .gap(px(8.))
                            .p(px(12.))
                            .rounded(px(10.))
                            .bg(c(pl.surface2))
                            .when(trash > 0, |d| d.child(line("trash", format!("{trash} moved to the Trash. You can put them back until you empty it."))))
                            .when(delete > 0, |d| d.child(line("refresh", format!("{delete} caches and logs deleted. Apps recreate them when needed.")))),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(px(8.))
                            .pt(px(4.))
                            .child(button("sheet-cancel", Kind::Secondary, None, "Cancel").on_click(cx.listener(|this, _, _, cx| {
                                this.confirm = None;
                                cx.notify();
                            })))
                            .child(
                                button("sheet-go", Kind::Primary, None, format!("{} {}", confirm.verb, human_bytes(total)))
                                    .on_click(cx.listener(|this, _, _, cx| this.run_confirmed(cx))),
                            ),
                    ),
                true,
                cx,
            ),
            id,
            0,
        )
    }

    fn cleaning_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let c_ = self.cleaning.as_mut().unwrap();

        // Bytes are reported per file, so they move steadily even inside one
        // huge folder. The time cap keeps fast cleans from flashing by.
        let real = if c_.report.is_some() {
            1.0
        } else if c_.total_bytes > 0 {
            (c_.progress.freed.load(Ordering::Relaxed) as f32 / c_.total_bytes as f32).min(0.98)
        } else {
            (c_.progress.done.load(Ordering::Relaxed) as f32 / c_.total_items.max(1) as f32).min(0.98)
        };
        let cap = c_.started.elapsed().as_secs_f32() / MIN_FILL_SECS;
        let target = real.min(cap);
        c_.shown += (target - c_.shown) * 0.16;
        if c_.report.is_some() && c_.shown > 0.995 && c_.completed_at.is_none() {
            c_.shown = 1.0;
            c_.completed_at = Some(Instant::now());
        }
        let done = c_.completed_at.is_some();
        if !done {
            window.request_animation_frame();
        }

        let freed_total = c_.report.as_ref().map(|r| r.freed).unwrap_or(c_.total_bytes);
        let freed_shown = (freed_total as f64 * c_.shown as f64) as u64;
        let files_removed = c_.progress.files.load(Ordering::Relaxed);
        // One revolution every 1.4 s.
        let sweep = (!done).then(|| (c_.started.elapsed().as_secs_f32() / 1.4).fract());
        let shown = c_.shown;
        let current = c_.progress.current.lock().map(|s| s.clone()).unwrap_or_default();
        let failures = c_.report.as_ref().map(|r| r.failures.len()).unwrap_or(0);
        let removed = c_.report.as_ref().map(|r| r.removed).unwrap_or(0);
        let verb = c_.title.clone();
        let key = format!("{:?}", c_.started);

        const D: f32 = 132.;
        let center: AnyElement = if done {
            svg()
                .path("icons/check.svg")
                .size(px(52.))
                .text_color(c(pl.green))
                .with_animation(
                    gpui::ElementId::Name(format!("check-{key}").into()),
                    Animation::new(Duration::from_millis(520)),
                    |s, t| {
                        let k = 0.4 + 0.6 * ease_out_back(t);
                        s.with_transformation(Transformation::scale(size(k, k))).opacity((t * 2.5).min(1.0))
                    },
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .items_center()
                .child(text(26., pl.text).font_weight(FontWeight::SEMIBOLD).child(format!("{:.0}%", shown * 100.)))
                .into_any_element()
        };

        let dial = div()
            .relative()
            .size(px(D))
            .child(ring(shown, sweep, D, 8., pl.border, if done { pl.green } else { pl.accent }))
            .child(div().absolute().inset_0().flex().items_center().justify_center().child(center));

        let (title, subtitle): (String, String) = if done {
            (
                format!("{} freed", human_bytes(freed_total)),
                format!("{removed} {} {}", if removed == 1 { "item" } else { "items" }, if verb == "Uninstall" { "removed" } else { "cleaned up" }),
            )
        } else {
            (human_bytes(freed_shown), format!("of {}", human_bytes(c_total(self))))
        };

        let body = panel(360.)
            .items_center()
            .gap(px(18.))
            .pt(px(32.))
            .child(dial)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(4.))
                    .child(text(20., pl.text).font_weight(FontWeight::SEMIBOLD).child(title))
                    .child(text(BODY, pl.text2).child(subtitle))
                    .child(
                        text(CAPTION, pl.text3)
                            .h(px(18.))
                            .max_w(px(300.))
                            .truncate()
                            .child(match (done, files_removed) {
                                (true, _) => String::new(),
                                (false, 0) => format!("Working on {current}"),
                                (false, n) => format!("Removed {} files · {current}", crate::pages::fmt_count(n)),
                            }),
                    ),
            )
            .when(done && failures > 0, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon("alert", pl.orange, 14.))
                        .child(text(CAPTION, pl.orange).child(format!("{failures} items were in use or protected and were skipped"))),
                )
            })
            .child(div().h(px(32.)).w_full().flex().justify_center().when(done, |d| {
                d.child(fade_in(
                    button("clean-done", Kind::Primary, None, "Done").w(px(120.)).justify_center().on_click(cx.listener(|this, _, _, cx| {
                        this.cleaning = None;
                        cx.notify();
                    })),
                    format!("done-btn-{key}"),
                    4,
                ))
            }));

        backdrop(body, done, cx).into_any_element()
    }
}

fn c_total(app: &MacScrap) -> u64 {
    app.cleaning.as_ref().map(|c| c.total_bytes).unwrap_or(0)
}
