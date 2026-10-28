//! Uninstaller: pick an app, see everything it installed, remove it all.

use super::*;
use crate::app::{reveal, MacScrap};
use gpui::{img, Context, FontWeight, KeyDownEvent, SharedString};
use macscrap_core::apps::AppInfo;
use macscrap_core::clean::Method;
use macscrap_core::human_bytes;

fn app_icon(app: &AppInfo, size: f32) -> AnyElement {
    match &app.icon {
        Some(path) => img(path.clone()).size(px(size)).flex_none().into_any_element(),
        None => tile_icon("app-window", p().text2, size).into_any_element(),
    }
}

fn leftover_icon(kind: &str) -> (&'static str, u32) {
    let pl = p();
    if kind.starts_with("Cache") {
        ("layers", pl.accent)
    } else if kind.starts_with("Preferences") {
        ("file-text", pl.text2)
    } else if kind.contains("Container") {
        ("box", pl.indigo)
    } else if kind.starts_with("Launch") || kind.contains("Helper") {
        ("power", pl.orange)
    } else if kind.contains("Log") || kind.contains("Crash") {
        ("file-text", pl.text3)
    } else if kind.contains("Web") || kind.contains("Cookies") {
        ("globe", pl.teal)
    } else {
        ("folder", pl.accent)
    }
}

impl MacScrap {
    fn filtered_apps(&self) -> Vec<AppInfo> {
        let q = self.app_query.to_lowercase();
        let mut apps: Vec<AppInfo> = self
            .apps
            .done()
            .map(|a| a.iter().filter(|a| q.is_empty() || a.name.to_lowercase().contains(&q)).cloned().collect())
            .unwrap_or_default();
        apps.sort_by(|a, b| b.size.cmp(&a.size));
        apps
    }

    fn on_search_key(&mut self, e: &KeyDownEvent, cx: &mut gpui::Context<Self>) {
        let k = &e.keystroke;
        if k.modifiers.platform || k.modifiers.control {
            return;
        }
        match k.key.as_str() {
            "backspace" => {
                self.app_query.pop();
            }
            "escape" if self.confirm.is_none() => self.app_query.clear(),
            "enter" | "tab" | "escape" | "up" | "down" | "left" | "right" => return,
            _ => match &k.key_char {
                Some(ch) if !ch.chars().any(char::is_control) => self.app_query.push_str(ch),
                _ => return,
            },
        }
        cx.notify();
    }

    fn select_app(&mut self, app: AppInfo, cx: &mut Context<Self>) {
        self.selected_app = Some(app.bundle_id.clone());
        self.leftovers = crate::app::Job::Idle;
        self.leftovers_selected.clear();
        self.spawn_job(
            cx,
            |s| &mut s.leftovers,
            move |cancel, _| macscrap_core::apps::leftovers(&app, &cancel),
            |s| {
                if cfg!(target_os = "macos") {
                    s.leftovers_selected = s.leftovers.done().map(|l| l.iter().map(|l| l.path.clone()).collect()).unwrap_or_default();
                }
            },
        );
    }

    pub fn render_uninstaller(&mut self, window: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let focused = self.search_focus.is_focused(window);
        let search = div()
            .id("search")
            .w(px(240.))
            .h(px(30.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(7.))
            .rounded(px(8.))
            .bg(c(pl.surface2))
            .border_1()
            .border_color(c(if focused { pl.accent } else { pl.border }))
            .track_focus(&self.search_focus)
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| this.on_search_key(e, cx)))
            .on_click(cx.listener(|this, _, window, _| window.focus(&this.search_focus)))
            .child(icon("search", pl.text3, 14.))
            .child(if self.app_query.is_empty() {
                text(BODY, pl.text3).flex_1().child("Search apps")
            } else {
                text(BODY, pl.text)
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(self.app_query.clone())
                    .when(focused, |d| d.child(div().w(px(1.5)).h(px(15.)).ml(px(1.)).bg(c(pl.accent))))
            })
            .when(!self.app_query.is_empty(), |d| {
                d.child(
                    div()
                        .id("clear-search")
                        .cursor_pointer()
                        .child(icon("x", pl.text3, 13.))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.app_query.clear();
                            cx.notify();
                        })),
                )
            });
        let head = header("Uninstaller", "Remove apps completely, including the files they leave behind.", search);

        if self.apps.is_running() || self.apps.done().is_none() {
            return page(head, None, scanning_view("Finding your apps", "Measuring app sizes".into(), None), None);
        }
        let apps = self.filtered_apps();
        let selected = self.selected_app.as_ref().and_then(|id| self.apps.done()?.iter().find(|a| &a.bundle_id == id).cloned());

        let list = card().w(px(290.)).flex_none().flex().flex_col().overflow_hidden().child(
            div()
                .id("app-list")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .p(px(4.))
                .flex()
                .flex_col()
                .when(apps.is_empty(), |d| d.child(text(BODY, pl.text3).p(px(16.)).child(format!("No apps match “{}”", self.app_query))))
                .children(apps.into_iter().map(|a| {
                    let active = selected.as_ref().is_some_and(|s| s.bundle_id == a.bundle_id);
                    let pick = a.clone();
                    div()
                        .id(SharedString::from(format!("app-{}", a.bundle_id)))
                        .h(px(44.))
                        .px(px(8.))
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(px(10.))
                        .rounded(px(8.))
                        .cursor_pointer()
                        .when(active, |d| d.bg(c(pl.accent)))
                        .when(!active, |d| d.hover(|s| s.bg(c(pl.hover))))
                        .child(app_icon(&a, 30.))
                        .child(text(BODY, if active { pl.on_accent } else { pl.text }).flex_1().min_w_0().truncate().child(a.name.clone()))
                        .child(text(CAPTION, if active { pl.on_accent } else { pl.text3 }).child(if a.size == 0 && cfg!(target_os = "linux") { "—".into() } else { human_bytes(a.size) }))
                        .on_click(cx.listener(move |this, _, _, cx| this.select_app(pick.clone(), cx)))
                })),
        );

        let (detail, footer) = match selected {
            None => (empty_view("grid", pl.accent, "Select an app", if cfg!(target_os = "linux") { "Review its desktop entry and matching user data." } else { "You'll see the app and every file it created across your Mac." }, None).into_any_element(), None),
            Some(app) => self.app_detail(app, cx),
        };

        page(
            head,
            None,
            div().flex_1().min_h_0().flex().gap(px(16.)).px(px(28.)).pb(px(20.)).child(list).child(div().flex_1().min_w_0().flex().flex_col().child(detail)),
            footer,
        )
    }

    fn app_detail(&mut self, app: AppInfo, cx: &mut Context<Self>) -> (AnyElement, Option<AnyElement>) {
        let pl = p();
        let summary = div()
            .flex()
            .items_center()
            .gap(px(14.))
            .pb(px(16.))
            .child(app_icon(&app, 56.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(text(17., pl.text).font_family("Geist").font_weight(FontWeight::SEMIBOLD).child(app.name.clone()))
                    .child(text(CAPTION, pl.text3).truncate().child(if app.version.is_empty() {
                        app.bundle_id.clone()
                    } else {
                        format!("Version {} · {}", app.version, app.bundle_id)
                    })),
            );

        let Some(left) = self.leftovers.done().cloned() else {
            return (
                div().flex_1().flex().flex_col().child(summary).child(scanning_view("Finding related files", if cfg!(target_os = "linux") { "Checking user data folders" } else { "Checking Library folders" }.into(), None)).into_any_element(),
                None,
            );
        };

        let gen = self.leftovers.generation();
        let app_path = app.path.clone();
        let mut rows: Vec<AnyElement> = vec![list_row("app-bundle")
            .child(if cfg!(target_os = "linux") { icon("info", pl.text3, 16.).into_any_element() } else { checkbox(true).opacity(0.5).into_any_element() })
            .child(app_icon(&app, 30.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).child(if cfg!(target_os = "linux") { format!("{} desktop entry", app.name) } else { format!("{}.app", app.name) }))
                    .child(text(CAPTION, pl.text3).truncate().child(short_path(&app.path))),
            )
            .child(pill("Application", pl.accent))
            .child(size_text(app.size))
            .child(icon_button("rv-app", "folder").on_click(move |_, _, _| reveal(&app_path)))
            .into_any_element()];
        rows.extend(left.iter().enumerate().map(|(ix, l)| {
            let checked = self.leftovers_selected.contains(&l.path);
            let (toggle, rv) = (l.path.clone(), l.path.clone());
            let (ic, color) = leftover_icon(l.kind);
            fade_in(
                list_row(SharedString::from(format!("lo-{ix}")))
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.leftovers_selected.remove(&toggle) {
                            this.leftovers_selected.insert(toggle.clone());
                        }
                        cx.notify();
                    }))
                    .child(checkbox(checked))
                    .child(tile_icon(ic, color, 30.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(BODY, pl.text).truncate().child(file_name(&l.path)))
                            .child(text(CAPTION, pl.text3).truncate().child(parent_display(&l.path))),
                    )
                    .child(pill(l.kind, pl.text2))
                    .child(size_text(l.size))
                    .child(icon_button(SharedString::from(format!("lrv-{ix}")), "folder").on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        reveal(&rv)
                    })),
                format!("lo-{gen}-{ix}"),
                ix + 1,
            )
        }));

        let extra: u64 = left.iter().filter(|l| self.leftovers_selected.contains(&l.path)).map(|l| l.size).sum();
        let n_extra = left.iter().filter(|l| self.leftovers_selected.contains(&l.path)).count();
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(summary)
            .child(
                div()
                    .flex()
                    .justify_between()
                    .px(px(4.))
                    .pb(px(8.))
                    .child(section_label(if cfg!(target_os = "linux") { "Application and related files" } else { "Will be removed" }))
                    .child(text(MICRO, pl.text3).child(format!("{} related items", left.len()))),
            )
            .child(div().id("leftovers").flex_1().min_h_0().overflow_y_scroll().child(grouped(rows)))
            .into_any_element();

        #[cfg(target_os = "macos")]
        let footer = action_bar(
            div()
                .flex()
                .flex_col()
                .child(strong(BODY, pl.text).child(format!("{} · {}", app.name, human_bytes(app.size + extra))))
                .child(text(CAPTION, pl.text3).child(format!("App + {n_extra} related items, moved to the Trash"))),
            button("uninstall", Kind::Danger, Some("trash"), "Uninstall").on_click(cx.listener(move |this, _, _, cx| {
                if macscrap_core::apps::is_running(&app) {
                    this.show_toast(format!("Quit {} first, then try again.", app.name), false);
                    cx.notify();
                    return;
                }
                let mut items = vec![(app.path.clone(), app.size, Method::Trash)];
                if let Some(left) = this.leftovers.done() {
                    items.extend(left.iter().filter(|l| this.leftovers_selected.contains(&l.path)).map(|l| (l.path.clone(), l.size, Method::Trash)));
                }
                this.ask(format!("Uninstall {}?", app.name), "Uninstall", items, |this, _| {
                    if let (Some(apps), Some(id)) = (this.apps.done_mut(), this.selected_app.clone()) {
                        apps.retain(|a| a.bundle_id != id || a.path.exists());
                    }
                    this.selected_app = None;
                    this.leftovers = crate::app::Job::Idle;
                }, cx);
            })),
        );
        #[cfg(target_os = "linux")]
        let footer = action_bar(
            div().flex().flex_col()
                .child(strong(BODY, pl.text).child(format!("{n_extra} related items selected · {}", human_bytes(extra))))
                .child(text(CAPTION, pl.text3).child(if app.user_flatpak.is_some() { "User data and shared runtimes are kept when uninstalling." } else { "Remove the app through your software manager." })),
            div().flex().gap(px(8.))
                .children((n_extra > 0 && !self.flatpak_uninstall_pending).then(|| button("remove-app-data", Kind::Danger, Some("trash"), "Move Data to Trash")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let items = this.leftovers.done().map(|left| left.iter().filter(|l| this.leftovers_selected.contains(&l.path))
                            .map(|l| (l.path.clone(), l.size, Method::Trash)).collect()).unwrap_or_default();
                        this.ask("Move selected app data to the Trash?", "Move", items, |this, _| {
                            this.leftovers = crate::app::Job::Idle;
                            this.leftovers_selected.clear();
                            this.selected_app = None;
                        }, cx);
                    }))))
                .child(if self.flatpak_uninstall_pending {
                    pill("Uninstalling…", pl.text3).into_any_element()
                } else if app.user_flatpak.is_some() {
                    button("uninstall", Kind::Danger, Some("trash"), "Uninstall…").on_click(cx.listener(move |this, _, _, cx| {
                        this.flatpak_confirm = Some(app.clone());
                        cx.notify();
                    })).into_any_element()
                } else {
                    button("uninstall", Kind::Secondary, Some("external"), "Open Software").on_click(cx.listener(move |this, _, _, cx| {
                        if let Err(error) = macscrap_core::apps::open_software_manager(&app) {
                            this.show_toast(error, false);
                        }
                        cx.notify();
                    })).into_any_element()
                }),
        );
        (body, Some(footer.into_any_element()))
    }
}
