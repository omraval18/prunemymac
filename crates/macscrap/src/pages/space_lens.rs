//! Space Lens: browse a size-sorted tree of the disk, folder by folder.
//! Rows are virtualized with `uniform_list`, so huge folders stay smooth.

use super::*;
use crate::app::{open_path, reveal, MacScrap, LensScope, LensTree};
use gpui::{uniform_list, Context, FontWeight, ListSizingBehavior, PathPromptOptions, SharedString};
use macscrap_core::clean::Method;
use macscrap_core::human_bytes;
use macscrap_core::space::Dir;
use std::path::PathBuf;

const ROW_H: f32 = 52.;
const FDA_URL: &str = "x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles";

/// One visible row of the current folder.
#[derive(Clone)]
enum Row {
    Dir { index: usize, name: String, size: u64, files: u64, unreadable: bool },
    File { name: String, size: u64 },
    Others { count: u64, size: u64 },
    Hidden { size: u64 },
}

impl Row {
    fn size(&self) -> u64 {
        match self {
            Row::Dir { size, .. } | Row::File { size, .. } | Row::Others { size, .. } | Row::Hidden { size } => *size,
        }
    }
}

/// Flat colors for the composition bar's top segments.
fn segment_colors() -> [u32; 6] {
    let pl = p();
    [pl.accent, pl.indigo, pl.teal, pl.orange, pl.pink, pl.purple]
}

fn rows_for(t: &LensTree, dir: &Dir, at_root: bool) -> Vec<Row> {
    let mut rows: Vec<Row> = Vec::with_capacity(dir.dirs.len() + dir.top_files.len() + 2);
    rows.extend(dir.dirs.iter().enumerate().map(|(index, d)| Row::Dir {
        index,
        name: d.name.to_string(),
        size: d.size,
        files: d.files,
        unreadable: d.unreadable,
    }));
    rows.extend(dir.top_files.iter().map(|f| Row::File { name: f.name.to_string(), size: f.size }));
    if dir.other_count > 0 {
        rows.push(Row::Others { count: dir.other_count, size: dir.other_size });
    }
    if at_root && t.hidden > 0 {
        rows.push(Row::Hidden { size: t.hidden });
    }
    // Keep one ordering for everything: biggest first.
    rows.sort_by(|a, b| b.size().cmp(&a.size()));
    rows
}

impl MacScrap {
    fn lens_dir(&self) -> Option<(&LensTree, &Dir)> {
        let t = self.lens.done()?;
        Some((t, t.tree.at(&self.lens_chain)?))
    }

    fn lens_path(&self) -> Option<PathBuf> {
        let t = self.lens.done()?;
        let mut p = t.root.clone();
        let mut d = &t.tree;
        for &i in &self.lens_chain {
            d = d.dirs.get(i)?;
            p.push(&*d.name);
        }
        Some(p)
    }

    fn lens_rows(&self) -> Vec<Row> {
        match self.lens_dir() {
            Some((t, d)) => rows_for(t, d, self.lens_chain.is_empty()),
            None => vec![],
        }
    }

    fn lens_open(&mut self, index: usize, cx: &mut Context<Self>) {
        self.lens_chain.push(index);
        self.lens_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    fn lens_up_to(&mut self, depth: usize, cx: &mut Context<Self>) {
        self.lens_chain.truncate(depth);
        self.lens_scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
        cx.notify();
    }

    fn choose_folder(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Map Folder".into()),
        });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                if let Some(path) = paths.into_iter().next() {
                    this.update(cx, |this, cx| this.start_lens_scan(LensScope::Folder(path), cx)).ok();
                }
            }
        })
        .detach();
    }

    pub fn render_space_lens(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let pl = p();
        let scope_ix = match &self.lens_scope {
            LensScope::Home => 0,
            LensScope::Mac => 1,
            LensScope::Folder(_) => 2,
        };
        let running = self.lens.is_running();
        let scopes = segmented()
            .child(segment("sc-home", "Home", scope_ix == 0).on_click(cx.listener(|this, _, _, cx| this.start_lens_scan(LensScope::Home, cx))))
            .child(segment("sc-mac", "Entire Mac", scope_ix == 1).on_click(cx.listener(|this, _, _, cx| this.start_lens_scan(LensScope::Mac, cx))))
            .child(segment("sc-folder", "Folder…", scope_ix == 2).on_click(cx.listener(|this, _, _, cx| this.choose_folder(cx))));
        let actions = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(scopes)
            .child(if running {
                button("lens-stop", Kind::Secondary, Some("stop"), "Stop")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.lens.cancel();
                        cx.notify();
                    }))
                    .into_any_element()
            } else {
                icon_button("lens-rescan", "refresh")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let scope = this.lens_scope.clone();
                        this.start_lens_scan(scope, cx)
                    }))
                    .into_any_element()
            });
        let head = header("Space Lens", "See what's using your disk, folder by folder, biggest first.", actions);

        if running {
            let what = match &self.lens_scope {
                LensScope::Home => "Mapping your home folder".to_string(),
                LensScope::Mac => "Mapping your entire Mac".to_string(),
                LensScope::Folder(p) => format!("Mapping {}", file_name(p)),
            };
            return page(head, None, scanning_view(&what, running_detail(self.lens.progress(), "files"), None), None);
        }
        if self.lens.done().is_none() {
            let start = div()
                .flex()
                .gap(px(8.))
                .child(button("lens-home", Kind::Primary, Some("scan"), "Map Home Folder").on_click(cx.listener(|this, _, _, cx| this.start_lens_scan(LensScope::Home, cx))))
                .child(button("lens-mac", Kind::Secondary, Some("disk"), "Entire Mac").on_click(cx.listener(|this, _, _, cx| this.start_lens_scan(LensScope::Mac, cx))))
                .into_any_element();
            return page(
                head,
                None,
                empty_view("pie", pl.accent, "Map your storage", "Every folder sorted from biggest to smallest. Click into any folder to see what's inside.", Some(start)),
                None,
            );
        }

        let (toolbar, body) = self.lens_body(cx);
        let footer = self.lens_footer(cx);
        page(head, Some(toolbar), body, Some(footer))
    }

    fn lens_body(&mut self, cx: &mut Context<Self>) -> (AnyElement, AnyElement) {
        let pl = p();
        let Some((t, dir)) = self.lens_dir() else {
            return (div().into_any_element(), div().into_any_element());
        };
        let total = dir.size + if self.lens_chain.is_empty() { t.hidden } else { 0 };
        let at_root = self.lens_chain.is_empty();

        // Breadcrumbs: root › a › b, each clickable.
        let mut crumbs: Vec<(String, usize)> = vec![(
            match &t.scope {
                LensScope::Home => "Home".to_string(),
                LensScope::Mac => "Macintosh HD".to_string(),
                LensScope::Folder(p) => file_name(p),
            },
            0,
        )];
        let mut d = &t.tree;
        for (depth, &i) in self.lens_chain.iter().enumerate() {
            d = &d.dirs[i];
            crumbs.push((d.name.to_string(), depth + 1));
        }
        let last = crumbs.len() - 1;
        let crumb_bar = div().flex().flex_1().min_w_0().items_center().gap(px(2.)).overflow_hidden().children(crumbs.into_iter().map(|(name, depth)| {
            let is_last = depth == last;
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(2.))
                .when(depth > 0, |d| d.child(icon("chevron-right", pl.text3, 12.)))
                .child(
                    div()
                        .id(SharedString::from(format!("crumb-{depth}")))
                        .px(px(6.))
                        .h(px(24.))
                        .flex()
                        .items_center()
                        .rounded(px(6.))
                        .text_size(px(BODY))
                        .text_color(c(if is_last { pl.text } else { pl.text2 }))
                        .when(is_last, |d| d.font_family("Geist").font_weight(FontWeight::SEMIBOLD))
                        .when(!is_last, |d| d.cursor_pointer().hover(|s| s.bg(c(pl.hover)).text_color(c(pl.text))))
                        .child(name)
                        .on_click(cx.listener(move |this, _, _, cx| this.lens_up_to(depth, cx))),
                )
        }));

        let here = self.lens_path();
        let toolbar = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        icon_button("lens-back", "arrow-left")
                            .when(at_root, |b| b.opacity(0.35).cursor_default())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let depth = this.lens_chain.len().saturating_sub(1);
                                this.lens_up_to(depth, cx)
                            })),
                    )
                    .child(crumb_bar)
                    .child(text(CAPTION, pl.text3).flex_none().child(format!("{} · {} files", human_bytes(total), fmt_count(dir.files))))
                    .children(here.map(|p| icon_button("lens-reveal-here", "folder-open").on_click(move |_, _, _| open_path(&p)))),
            )
            .child(self.composition_bar(t, dir, total))
            .into_any_element();

        let rows = self.lens_rows();
        let n = rows.len();
        let unreadable_top = at_root && dir.dirs.iter().any(|d| d.unreadable);
        let body = div()
            .flex_1()
            .min_h_0()
            .px(px(28.))
            .pb(px(16.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .when(unreadable_top, |d| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(12.))
                        .py(px(9.))
                        .rounded(px(10.))
                        .bg(tint(pl.orange))
                        .child(icon("lock", pl.orange, 15.))
                        .child(text(CAPTION, pl.text).flex_1().child("Some folders are hidden by macOS privacy protection. Give prunemymac Full Disk Access to see everything."))
                        .child(button("fda", Kind::Secondary, None, "Open Settings").on_click(|_, _, _| {
                            macscrap_core::sys::spawn_detached(std::process::Command::new("/usr/bin/open").arg(FDA_URL));
                        })),
                )
            })
            .child(
                card().flex_1().min_h_0().p(px(4.)).child(
                    uniform_list(
                        "lens-rows",
                        n,
                        cx.processor(move |this, range: std::ops::Range<usize>, _window, cx| {
                            let rows = this.lens_rows();
                            let parent = this.lens_dir().map(|(_, d)| d.size).unwrap_or(1).max(1);
                            let here = this.lens_path().unwrap_or_default();
                            range.filter_map(|ix| rows.get(ix).cloned().map(|row| this.lens_row(ix, row, parent, &here, cx))).collect()
                        }),
                    )
                    .with_sizing_behavior(ListSizingBehavior::Infer)
                    .track_scroll(self.lens_scroll.clone())
                    .size_full(),
                ),
            )
            .into_any_element();
        (toolbar, body)
    }

    /// Stacked bar of the six largest children; the rest in a neutral tone.
    fn composition_bar(&self, t: &LensTree, dir: &Dir, total: u64) -> Div {
        let pl = p();
        let rows = rows_for(t, dir, self.lens_chain.is_empty());
        let colors = segment_colors();
        let total = total.max(1) as f32;
        let mut bar = div().h(px(10.)).w_full().flex().gap(px(2.)).rounded(px(4.)).overflow_hidden();
        let mut used = 0.0;
        for (i, row) in rows.iter().take(6).enumerate() {
            let frac = row.size() as f32 / total;
            if frac < 0.004 {
                break;
            }
            used += frac;
            bar = bar.child(div().h_full().w(gpui::relative(frac)).bg(c(colors[i])));
        }
        if used < 0.999 {
            bar = bar.child(div().h_full().flex_1().bg(c(pl.border_strong)));
        }
        bar
    }

    fn lens_row(&self, ix: usize, row: Row, parent_size: u64, here: &std::path::Path, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let colors = segment_colors();
        let dot = (ix < 6).then(|| colors[ix]);
        let frac = row.size() as f32 / parent_size as f32;
        let (icon_el, name, caption, path): (AnyElement, String, String, Option<PathBuf>) = match &row {
            Row::Dir { name, files, unreadable, .. } => (
                tile_icon(if *unreadable { "lock" } else { "folder" }, if *unreadable { pl.text3 } else { pl.accent }, 30.).into_any_element(),
                name.clone(),
                if *unreadable { "Needs Full Disk Access".into() } else { format!("{} files", fmt_count(*files)) },
                Some(here.join(name)),
            ),
            Row::File { name, .. } => {
                let path = here.join(name);
                let (ic, color) = file_kind(&path);
                (tile_icon(ic, color, 30.).into_any_element(), name.clone(), "File".into(), Some(path))
            }
            Row::Others { count, .. } => (
                tile_icon("more", pl.text3, 30.).into_any_element(),
                format!("{} smaller files", fmt_count(*count)),
                "Grouped to keep things fast".into(),
                None,
            ),
            Row::Hidden { .. } => (
                tile_icon("disk", pl.text3, 30.).into_any_element(),
                "macOS & system data".into(),
                "System volume, snapshots and other volumes".into(),
                None,
            ),
        };
        let selectable = path.is_some();
        let checked = path.as_ref().is_some_and(|p| self.lens_selected.contains(p));
        let open_index = match row {
            Row::Dir { index, unreadable: false, .. } => Some(index),
            _ => None,
        };
        let toggle = path.clone();
        let reveal_path = path.clone();
        let group = SharedString::from(format!("lr-{ix}"));

        div()
            .id(group.clone())
            .group(group.clone())
            .h(px(ROW_H))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(12.))
            .rounded(px(8.))
            .hover(|s| s.bg(c(pl.hover)))
            .when(open_index.is_some() || selectable, |d| d.cursor_pointer())
            .on_click(cx.listener(move |this, _, _, cx| match (open_index, &toggle) {
                (Some(i), _) => this.lens_open(i, cx),
                (None, Some(p)) => {
                    if !this.lens_selected.remove(p) {
                        this.lens_selected.insert(p.clone());
                    }
                    cx.notify();
                }
                _ => {}
            }))
            .child(
                div()
                    .id(SharedString::from(format!("lc-{ix}")))
                    .p(px(4.))
                    .m(px(-4.))
                    .when(!selectable, |d| d.opacity(0.))
                    .child(checkbox(checked))
                    .on_click(cx.listener({
                        let p = path.clone();
                        move |this, _, _, cx| {
                            cx.stop_propagation();
                            if let Some(p) = &p {
                                if !this.lens_selected.remove(p) {
                                    this.lens_selected.insert(p.clone());
                                }
                                cx.notify();
                            }
                        }
                    })),
            )
            .child(icon_el)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(name))
                    .child(text(CAPTION, pl.text3).truncate().child(caption)),
            )
            .child(
                div()
                    .w(px(150.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .h(px(4.))
                            .rounded_full()
                            .bg(c(pl.border))
                            .child(div().h_full().rounded_full().w(gpui::relative(frac.clamp(0.0, 1.0))).bg(c(dot.unwrap_or(pl.text3)))),
                    )
                    .child(text(CAPTION, pl.text3).w(px(38.)).text_right().child(if frac >= 0.001 { format!("{:.0}%", (frac * 100.).max(1.)) } else { "<1%".into() })),
            )
            .child(size_text(row.size()))
            .child(
                div()
                    .w(px(52.))
                    .flex()
                    .flex_none()
                    .justify_end()
                    .gap(px(2.))
                    .child(
                        div().opacity(0.).group_hover(group.clone(), |s| s.opacity(1.)).children(reveal_path.map(|p| {
                            icon_button(SharedString::from(format!("lrv-{ix}")), "folder").size(px(24.)).on_click(move |_, _, cx| {
                                cx.stop_propagation();
                                reveal(&p)
                            })
                        })),
                    )
                    .child(div().size(px(24.)).flex().items_center().justify_center().when(open_index.is_some(), |d| d.child(icon("chevron-right", pl.text3, 14.)))),
            )
            .into_any_element()
    }

    fn lens_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let selected: Vec<PathBuf> = self.lens_selected.iter().cloned().collect();
        let size: u64 = selected.iter().map(|p| self.lens_size_of(p)).sum();
        action_bar(
            div()
                .flex()
                .flex_col()
                .child(strong(BODY, pl.text).child(if selected.is_empty() {
                    "Nothing selected".to_string()
                } else {
                    format!("{} {} · {}", selected.len(), if selected.len() == 1 { "item" } else { "items" }, human_bytes(size))
                }))
                .child(text(CAPTION, pl.text3).child("Click a folder to open it. Use the checkbox to select.")),
            div()
                .flex()
                .gap(px(8.))
                .when(!selected.is_empty(), |d| {
                    d.child(button("lens-clear", Kind::Ghost, None, "Clear").on_click(cx.listener(|this, _, _, cx| {
                        this.lens_selected.clear();
                        cx.notify();
                    })))
                })
                .child(
                    button("lens-trash", Kind::Primary, Some("trash"), "Move to Trash")
                        .when(selected.is_empty(), |b| b.opacity(0.45).cursor_default())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let items: Vec<_> = this.lens_selected.iter().map(|p| (p.clone(), this.lens_size_of(p), Method::Trash)).collect();
                            this.ask("Move to the Trash?", "Move", items, |this, report| {
                                let _ = report;
                                this.lens_apply_removals();
                            }, cx);
                        })),
                ),
        )
        .into_any_element()
    }

    /// Size of a selected path, looked up in the tree (no disk access).
    fn lens_size_of(&self, path: &std::path::Path) -> u64 {
        let Some(t) = self.lens.done() else { return 0 };
        let Ok(rel) = path.strip_prefix(&t.root) else { return 0 };
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        let Some((last, dirs)) = parts.split_last() else { return 0 };
        let mut d = &t.tree;
        for name in dirs {
            match d.dirs.iter().find(|x| &*x.name == name.as_str()) {
                Some(x) => d = x,
                None => return 0,
            }
        }
        d.dirs
            .iter()
            .find(|x| &*x.name == last.as_str())
            .map(|x| x.size)
            .or_else(|| d.top_files.iter().find(|f| &*f.name == last.as_str()).map(|f| f.size))
            .unwrap_or(0)
    }

    /// Drop removed items from the tree so sizes update without a rescan.
    fn lens_apply_removals(&mut self) {
        let removed: Vec<PathBuf> = self.lens_selected.drain().filter(|p| !p.exists()).collect();
        let current = self.lens_path();
        if let Some(t) = self.lens.done_mut() {
            for p in &removed {
                if let Ok(rel) = p.strip_prefix(&t.root) {
                    let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                    let refs: Vec<&str> = parts.iter().map(String::as_str).collect();
                    t.tree.remove(&refs);
                }
            }
        }
        // If the folder we were viewing was removed, step back to a valid level.
        if current.is_some_and(|c| !c.exists()) || self.lens.done().and_then(|t| t.tree.at(&self.lens_chain)).is_none() {
            self.lens_chain.clear();
        }
    }
}
