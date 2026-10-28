//! Large & Old Files and Duplicates.

use super::*;
use crate::app::{reveal, MacScrap, LargeSort};
use gpui::{Context, FontWeight, SharedString};
use macscrap_core::clean::Method;
use macscrap_core::files::FileEntry;
use macscrap_core::human_bytes;

const UNUSED_DAYS: u64 = 180;
const MAX_ROWS: usize = 500;

impl MacScrap {
    fn visible_large(&self) -> Vec<FileEntry> {
        let Some(files) = self.large.done() else { return vec![] };
        let mut v: Vec<FileEntry> = files
            .iter()
            .filter(|f| f.size >= self.large_min)
            .filter(|f| !self.large_unused_only || f.idle_days().unwrap_or(0) >= UNUSED_DAYS)
            .cloned()
            .collect();
        if self.large_sort == LargeSort::Unused {
            v.sort_by_key(|f| std::cmp::Reverse(f.idle_days().unwrap_or(0)));
        }
        v
    }

    pub fn render_large(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let actions = if self.large.is_running() {
            button("lg-stop", Kind::Secondary, Some("stop"), "Stop").on_click(cx.listener(|this, _, _, cx| {
                this.large.cancel();
                cx.notify();
            }))
        } else {
            button("lg-rescan", Kind::Secondary, Some(if self.large.done().is_some() { "refresh" } else { "scan" }), if self.large.done().is_some() { "Rescan" } else { "Scan" })
                .on_click(cx.listener(|this, _, _, cx| this.start_large_scan(cx)))
        };
        let head = header("Large & Old Files", "The biggest files in your home folder, and the ones you haven't opened in months.", actions);

        if self.large.is_running() {
            return page(head, None, scanning_view("Looking for large files", running_detail(self.large.progress(), "files checked"), None), None);
        }
        if self.large.done().is_none() {
            let scan = button("lg-scan", Kind::Primary, Some("scan"), "Scan Home Folder")
                .on_click(cx.listener(|this, _, _, cx| this.start_large_scan(cx)))
                .into_any_element();
            return page(head, None, empty_view("file", pl.indigo, "Find what's taking up space", "Videos, archives and disk images add up quickly. You choose what to move to the Trash.", Some(scan)), None);
        }

        let thresholds = [(100_000_000u64, "100 MB"), (500_000_000, "500 MB"), (1_000_000_000, "1 GB")];
        let toolbar = div()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(text(CAPTION, pl.text3).child("Larger than"))
            .child(segmented().children(thresholds.iter().map(|&(v, label)| {
                segment(SharedString::from(format!("th-{v}")), label, self.large_min == v).on_click(cx.listener(move |this, _, _, cx| {
                    this.large_min = v;
                    cx.notify();
                }))
            })))
            .child(segmented().children([(false, "All"), (true, "Unused 6+ months")].map(|(only, label)| {
                segment(SharedString::from(format!("un-{only}")), label, self.large_unused_only == only).on_click(cx.listener(move |this, _, _, cx| {
                    this.large_unused_only = only;
                    cx.notify();
                }))
            })))
            .child(div().flex_1())
            .child(text(CAPTION, pl.text3).child("Sort"))
            .child(segmented().children([(LargeSort::Size, "Largest"), (LargeSort::Unused, "Least used")].map(|(sort, label)| {
                segment(SharedString::from(format!("so-{label}")), label, self.large_sort == sort).on_click(cx.listener(move |this, _, _, cx| {
                    this.large_sort = sort;
                    cx.notify();
                }))
            })));

        let files = self.visible_large();
        let body = if files.is_empty() {
            empty_view("check-circle", pl.green, "No files match", "Try a smaller size or show all files.", None).into_any_element()
        } else {
            let gen = self.large.generation();
            let rows: Vec<AnyElement> = files
                .iter()
                .take(MAX_ROWS)
                .enumerate()
                .map(|(ix, f)| fade_in(self.large_row(ix, f, cx), format!("lr-{gen}-{ix}"), ix))
                .collect();
            scroll_body("large").child(grouped(rows)).into_any_element()
        };

        let selected: Vec<(std::path::PathBuf, u64)> = files.iter().filter(|f| self.large_selected.contains(&f.path)).map(|f| (f.path.clone(), f.size)).collect();
        let sel_size: u64 = selected.iter().map(|s| s.1).sum();
        let footer = action_bar(
            div()
                .flex()
                .flex_col()
                .child(strong(BODY, pl.text).child(if selected.is_empty() { "Nothing selected".to_string() } else { format!("{} files · {}", selected.len(), human_bytes(sel_size)) }))
                .child(text(CAPTION, pl.text3).child(format!("{} files · {}", files.len(), human_bytes(files.iter().map(|f| f.size).sum())))),
            button("lg-trash", Kind::Primary, Some("trash"), "Move to Trash")
                .when(selected.is_empty(), |b| b.opacity(0.45).cursor_default())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let items = selected.iter().map(|(p, s)| (p.clone(), *s, Method::Trash)).collect();
                    this.ask("Move files to the Trash?", "Move", items, |this, _| {
                        if let Some(files) = this.large.done_mut() {
                            files.retain(|f| f.path.exists());
                        }
                        this.large_selected.clear();
                    }, cx);
                })),
        );
        page(head, Some(toolbar.into_any_element()), body, Some(footer.into_any_element()))
    }

    fn large_row(&self, ix: usize, f: &FileEntry, cx: &mut Context<Self>) -> gpui::Stateful<Div> {
        let pl = p();
        let checked = self.large_selected.contains(&f.path);
        let idle = f.idle_days().unwrap_or(0);
        let (kind_icon, kind_color) = file_kind(&f.path);
        let (toggle, rv) = (f.path.clone(), f.path.clone());
        list_row(SharedString::from(format!("lf-{ix}")))
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.large_selected.remove(&toggle) {
                    this.large_selected.insert(toggle.clone());
                }
                cx.notify();
            }))
            .child(checkbox(checked))
            .child(tile_icon(kind_icon, kind_color, 30.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(file_name(&f.path)))
                    .child(text(CAPTION, pl.text3).truncate().child(parent_display(&f.path))),
            )
            .child(
                div()
                    .w(px(130.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(5.))
                    .when(idle >= UNUSED_DAYS, |d| d.child(icon("clock", pl.orange, 12.)))
                    .child(text(CAPTION, if idle >= UNUSED_DAYS { pl.orange } else { pl.text3 }).child(format!("Used {}", ago_days(idle)))),
            )
            .child(size_text(f.size))
            .child(icon_button(SharedString::from(format!("lrv-{ix}")), "folder").on_click(move |_, _, cx| {
                cx.stop_propagation();
                reveal(&rv)
            }))
    }

    pub fn render_duplicates(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let actions = if self.dupes.is_running() {
            button("dp-stop", Kind::Secondary, Some("stop"), "Stop").on_click(cx.listener(|this, _, _, cx| {
                this.dupes.cancel();
                cx.notify();
            }))
        } else {
            button("dp-rescan", Kind::Secondary, Some(if self.dupes.done().is_some() { "refresh" } else { "scan" }), if self.dupes.done().is_some() { "Rescan" } else { "Scan" })
                .on_click(cx.listener(|this, _, _, cx| this.start_dupes_scan(cx)))
        };
        let head = header("Duplicates", "Identical files may be needed by different apps. Review each path before selecting files to remove.", actions);

        if self.dupes.is_running() {
            return page(head, None, scanning_view("Comparing files", running_detail(self.dupes.progress(), "files checked"), None), None);
        }
        let Some(sets) = self.dupes.done() else {
            let scan = button("dp-scan", Kind::Primary, Some("scan"), "Find Duplicates")
                .on_click(cx.listener(|this, _, _, cx| this.start_dupes_scan(cx)))
                .into_any_element();
            return page(head, None, empty_view("copy", pl.teal, "Find duplicate files", "Files are compared byte for byte, so only true copies are shown.", Some(scan)), None);
        };
        if sets.is_empty() {
            return page(head, None, empty_view("check-circle", pl.green, "No duplicates", "Every file in your home folder is one of a kind.", None), None);
        }

        let gen = self.dupes.generation();
        let sets = sets.iter().take(300).cloned().collect::<Vec<_>>();
        let mut body = scroll_body("dupes").gap(px(10.));
        for (si, set) in sets.iter().enumerate() {
            let (kind_icon, kind_color) = file_kind(&set.files[0].path);
            let mut rows = card().flex().flex_col().child(
                div()
                    .px(px(14.))
                    .py(px(11.))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(tile_icon(kind_icon, kind_color, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(file_name(&set.files[0].path)))
                            .child(text(CAPTION, pl.text3).child(format!("{} copies · {} each", set.files.len(), human_bytes(set.size)))),
                    )
                    .child(pill(format!("{} extra", human_bytes(set.wasted())), pl.text2)),
            );
            for (fi, f) in set.files.iter().enumerate() {
                let checked = self.dupes_selected.contains(&f.path);
                let (toggle, rv) = (f.path.clone(), f.path.clone());
                rows = rows.child(div().px(px(14.)).child(divider())).child(
                    list_row(SharedString::from(format!("d-{si}-{fi}")))
                        .mx(px(4.))
                        .min_h(px(38.))
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if !this.dupes_selected.remove(&toggle) {
                                this.dupes_selected.insert(toggle.clone());
                            }
                            cx.notify();
                        }))
                        .child(checkbox(checked))
                        .child(text(CAPTION, pl.text2).flex_1().min_w_0().truncate().child(format!("{} — {}", file_name(&f.path), parent_display(&f.path))))
                        .child(text(CAPTION, pl.text3).child(format!("Modified {}", ago(f.modified))))
                        .child(div().w(px(52.)).flex().justify_end().when(!checked, |d| d.child(pill("Keep", pl.green))))
                        .child(icon_button(SharedString::from(format!("drv-{si}-{fi}")), "folder").on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            reveal(&rv)
                        })),
                );
            }
            body = body.child(fade_in(rows.pb(px(4.)), format!("ds-{gen}-{si}"), si));
        }

        let selected: Vec<(std::path::PathBuf, u64)> =
            sets.iter().flat_map(|s| s.files.iter().filter(|f| self.dupes_selected.contains(&f.path)).map(move |f| (f.path.clone(), s.size))).collect();
        let sel_size: u64 = selected.iter().map(|s| s.1).sum();
        // Guard against removing every copy of a file.
        let all_copies = sets.iter().any(|s| s.files.iter().all(|f| self.dupes_selected.contains(&f.path)));
        let wasted: u64 = sets.iter().map(|s| s.wasted()).sum();
        let footer = action_bar(
            div()
                .flex()
                .flex_col()
                .child(strong(BODY, if all_copies { pl.orange } else { pl.text }).child(if all_copies {
                    "Every copy of a file is selected".to_string()
                } else {
                    format!("{} copies · {}", selected.len(), human_bytes(sel_size))
                }))
                .child(text(CAPTION, pl.text3).child(format!("{} sets · {} reclaimable", sets.len(), human_bytes(wasted)))),
            div()
                .flex()
                .gap(px(8.))
                .child(button("dp-reset", Kind::Ghost, None, "Deselect All").on_click(cx.listener(|this, _, _, cx| {
                    this.dupes_selected.clear();
                    cx.notify();
                })))
                .child(
                    button("dp-trash", Kind::Primary, Some("trash"), "Move to Trash")
                        .when(selected.is_empty() || all_copies, |b| b.opacity(0.45).cursor_default())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if all_copies {
                                return;
                            }
                            let items = selected.iter().map(|(p, s)| (p.clone(), *s, Method::Trash)).collect();
                            this.ask("Move duplicates to the Trash?", "Move", items, |this, _| {
                                if let Some(sets) = this.dupes.done_mut() {
                                    for s in sets.iter_mut() {
                                        s.files.retain(|f| f.path.exists());
                                    }
                                    sets.retain(|s| s.files.len() > 1);
                                }
                                this.dupes_selected.clear();
                            }, cx);
                        })),
                ),
        );
        page(head, None, body, Some(footer.into_any_element()))
    }
}
