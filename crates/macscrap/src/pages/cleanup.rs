//! Cleanup: rule-based junk, grouped by category, reviewed before removal.

use super::*;
use crate::app::MacScrap;
use gpui::{Context, SharedString, Transformation};
use macscrap_core::clean::Method;
use macscrap_core::human_bytes;
use macscrap_core::junk::{Action, JunkGroup, Safety};

fn category_color(category: &str) -> u32 {
    let pl = p();
    match category {
        "Trash" => pl.text2,
        "Developer" => pl.indigo,
        "Downloads" => pl.teal,
        "Mail" => pl.accent,
        "Large Data" => pl.orange,
        "AI Clutter" => pl.purple,
        _ => pl.accent,
    }
}

impl MacScrap {
    /// After cleaning, drop what's gone so totals stay accurate without a rescan.
    pub fn prune_removed_junk(&mut self) {
        if let Some(groups) = self.junk.done_mut() {
            for g in groups.iter_mut() {
                g.items.retain(|i| i.path.exists());
            }
            groups.retain(|g| !g.items.is_empty());
        }
        self.junk_items.retain(|p| p.exists());
    }

    fn group_check(&self, g: &JunkGroup) -> Check {
        let n = g.items.iter().filter(|i| self.junk_items.contains(&i.path)).count();
        match n {
            0 => Check::Off,
            n if n == g.items.len() => Check::On,
            _ => Check::Mixed,
        }
    }

    pub fn selected_junk_items(&self) -> Vec<(std::path::PathBuf, u64, Method)> {
        let Some(groups) = self.junk.done() else { return vec![] };
        groups
            .iter()
            .flat_map(|g| {
                let m = if g.rule.action == Action::Delete { Method::Delete } else { Method::Trash };
                g.items.iter().filter(|i| self.junk_items.contains(&i.path)).map(move |i| (i.path.clone(), i.size, m))
            })
            .collect()
    }

    pub fn render_cleanup(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let actions = if self.junk.is_running() {
            button("junk-stop", Kind::Secondary, Some("stop"), "Stop").on_click(cx.listener(|this, _, _, cx| {
                this.junk.cancel();
                cx.notify();
            }))
        } else if self.junk.done().is_some() {
            button("junk-rescan", Kind::Secondary, Some("refresh"), "Rescan").on_click(cx.listener(|this, _, _, cx| this.start_junk_scan(cx)))
        } else {
            button("junk-scan-h", Kind::Secondary, Some("scan"), "Scan").on_click(cx.listener(|this, _, _, cx| this.start_junk_scan(cx)))
        };
        let head = header("Cleanup", "Caches, logs, developer leftovers, old installers and AI models.", actions);

        if self.junk.is_running() {
            let phase = self.junk_phase.lock().unwrap().clone();
            return page(head, None, scanning_view("Looking for junk", if phase.is_empty() { "Starting…".into() } else { phase }, None), None);
        }
        let Some(groups) = self.junk.done() else {
            let scan = button("junk-scan", Kind::Primary, Some("scan"), "Scan for Junk")
                .on_click(cx.listener(|this, _, _, cx| this.start_junk_scan(cx)))
                .into_any_element();
            return page(
                head,
                None,
                empty_view("sparkles", pl.accent, "Find files you don't need", "You'll review everything before anything is removed.", Some(scan)),
                None,
            );
        };
        if groups.is_empty() {
            return page(head, None, empty_view("check-circle", pl.green, "Nothing to clean", "Your Mac has no junk worth removing right now.", None), None);
        }

        let gen = self.junk.generation();
        let mut categories: Vec<(&str, Vec<&JunkGroup>)> = vec![];
        for g in groups {
            match categories.iter_mut().find(|(c, _)| *c == g.rule.category) {
                Some((_, v)) => v.push(g),
                None => categories.push((&g.rule.category, vec![g])),
            }
        }

        let mut body = scroll_body("junk").gap(px(8.));
        let mut row_ix = 0;
        for (category, groups) in categories {
            let cat_size: u64 = groups.iter().map(|g| g.size()).sum();
            body = body.child(
                div()
                    .flex()
                    .justify_between()
                    .px(px(4.))
                    .pt(px(10.))
                    .child(section_label(category))
                    .child(text(MICRO, pl.text3).child(human_bytes(cat_size))),
            );
            let rows = groups
                .into_iter()
                .map(|g| {
                    row_ix += 1;
                    fade_in(self.junk_row(g, category_color(category), cx), format!("jr-{gen}-{}", g.rule.id), row_ix)
                })
                .collect();
            body = body.child(grouped(rows));
        }

        let selected = self.selected_junk_items();
        let sel_size: u64 = selected.iter().map(|i| i.1).sum();
        let total: u64 = groups.iter().map(|g| g.size()).sum();
        let footer = action_bar(
            div()
                .flex()
                .flex_col()
                .child(strong(BODY, pl.text).child(format!("{} selected", human_bytes(sel_size))))
                .child(text(CAPTION, pl.text3).child(format!("{} found in total", human_bytes(total)))),
            button("junk-clean", Kind::Primary, Some("sparkles"), if sel_size > 0 { format!("Clean {}", human_bytes(sel_size)) } else { "Clean".into() })
                .when(sel_size == 0, |b| b.opacity(0.45).cursor_default())
                .on_click(cx.listener(|this, _, _, cx| {
                    let items = this.selected_junk_items();
                    this.ask("Clean selected junk?", "Clean", items, |this, _| this.prune_removed_junk(), cx);
                })),
        );
        page(head, None, body, Some(footer.into_any_element()))
    }

    fn junk_row(&self, g: &JunkGroup, color: u32, cx: &mut Context<Self>) -> Div {
        let pl = p();
        let id = g.rule.id.clone();
        let state = self.group_check(g);
        let expanded = self.junk_expanded.contains(&id);
        let expand_id = id.clone();
        let paths: Vec<std::path::PathBuf> = g.items.iter().map(|i| i.path.clone()).collect();

        let main = list_row(SharedString::from(format!("g-{id}")))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                // Mixed or off → select all; all → select none.
                if state == Check::On {
                    for p in &paths {
                        this.junk_items.remove(p);
                    }
                } else {
                    this.junk_items.extend(paths.iter().cloned());
                }
                cx.notify();
            }))
            .child(checkbox(state))
            .child(tile_icon(if g.rule.icon.is_empty() { "layers" } else { &g.rule.icon }, color, 30.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .child(text(BODY, pl.text).font_weight(gpui::FontWeight::MEDIUM).child(g.rule.name.clone()))
                            .when(g.rule.safety == Safety::Review, |d| d.child(pill("Review", pl.orange)))
                            .when(g.rule.action == Action::Trash, |d| d.child(pill("To Trash", pl.text2))),
                    )
                    .child(text(CAPTION, pl.text2).truncate().child(g.rule.description.clone())),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(size_text(g.size()))
                    .when(state == Check::Mixed, |d| {
                        let sel: u64 = g.items.iter().filter(|i| self.junk_items.contains(&i.path)).map(|i| i.size).sum();
                        d.child(text(MICRO, pl.text3).child(format!("{} selected", human_bytes(sel))))
                    }),
            )
            .child(
                div()
                    .id(SharedString::from(format!("ex-{id}")))
                    .size(px(24.))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .rounded(px(6.))
                    .cursor_pointer()
                    .hover(|s| s.bg(c(pl.pressed)))
                    .child(
                        icon("chevron-right", pl.text3, 14.)
                            .with_transformation(Transformation::rotate(gpui::radians(if expanded { std::f32::consts::FRAC_PI_2 } else { 0. }))),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        if !this.junk_expanded.remove(&expand_id) {
                            this.junk_expanded.insert(expand_id.clone());
                        }
                        cx.notify();
                    })),
            );

        let mut col = div().flex().flex_col().child(main);
        if expanded {
            let shown = 200;
            col = col.child(
                div()
                    .flex()
                    .flex_col()
                    .pl(px(40.))
                    .pr(px(8.))
                    .pb(px(6.))
                    .children(g.items.iter().take(shown).enumerate().map(|(ix, item)| {
                        let checked = self.junk_items.contains(&item.path);
                        let (toggle, rv, browse) = (item.path.clone(), item.path.clone(), item.path.clone());
                        let is_dir = item.path.is_dir();
                        let group = SharedString::from(format!("it-{id}-{ix}"));
                        div()
                            .id(group.clone())
                            .group(group.clone())
                            .h(px(32.))
                            .px(px(8.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .rounded(px(6.))
                            .cursor_pointer()
                            .hover(|s| s.bg(c(pl.hover)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if !this.junk_items.remove(&toggle) {
                                    this.junk_items.insert(toggle.clone());
                                }
                                cx.notify();
                            }))
                            .child(checkbox(checked))
                            .child(icon(if is_dir { "folder" } else { "file" }, pl.text3, 14.))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .gap(px(6.))
                                    .child(text(CAPTION, pl.text).flex_none().child(file_name(&item.path)))
                                    .child(text(CAPTION, pl.text3).min_w_0().truncate().child(parent_display(&item.path))),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap(px(2.))
                                    .opacity(0.)
                                    .group_hover(group, |s| s.opacity(1.))
                                    .when(is_dir, |d| {
                                        d.child(icon_button(SharedString::from(format!("br-{id}-{ix}")), "pie").size(px(24.)).on_click(cx.listener(
                                            move |this, _, window, cx| {
                                                cx.stop_propagation();
                                                this.start_lens_scan(crate::app::LensScope::Folder(browse.clone()), cx);
                                                this.go(crate::app::Page::SpaceLens, window, cx);
                                            },
                                        )))
                                    })
                                    .child(icon_button(SharedString::from(format!("rv-{id}-{ix}")), "folder").size(px(24.)).on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        crate::app::reveal(&rv)
                                    })),
                            )
                            .child(text(CAPTION, pl.text2).w(px(76.)).text_right().child(human_bytes(item.size)))
                    }))
                    .when(g.items.len() > shown, |d| {
                        d.child(text(CAPTION, pl.text3).pl(px(34.)).pt(px(4.)).child(format!("and {} more (follow the group checkbox)", g.items.len() - shown)))
                    }),
            );
        }
        col
    }
}
