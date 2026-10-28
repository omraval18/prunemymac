//! Performance: what's busy right now, what starts by itself, and one-click
//! maintenance tasks.

use super::*;
use crate::app::{reveal, run_as_admin, MacScrap, Job};
use gpui::{Context, FontWeight, SharedString};
use macscrap_core::background::Scope;
use macscrap_core::human_bytes;

struct Task {
    id: &'static str,
    icon: &'static str,
    title: &'static str,
    description: &'static str,
    command: &'static str,
}

const TASKS: [Task; 4] = [
    Task {
        id: "ram",
        icon: "memory",
        title: "Free Up RAM",
        description: "Clears inactive memory so apps you open next have room. Useful when memory pressure is high.",
        command: "purge",
    },
    Task {
        id: "dns",
        icon: "globe",
        title: "Flush DNS Cache",
        description: "Fixes websites that won't load or point to an old address after a network change.",
        command: "dscacheutil -flushcache; killall -HUP mDNSResponder",
    },
    Task {
        id: "spotlight",
        icon: "search",
        title: "Reindex Spotlight",
        description: "Rebuilds the search index when Spotlight misses files. Search is slower for a while afterwards.",
        command: "mdutil -E /",
    },
    Task {
        id: "snapshots",
        icon: "clock",
        title: "Thin Time Machine Snapshots",
        description: "Removes local snapshots that hold on to deleted files. Your backups on external drives are untouched.",
        command: "tmutil thinlocalsnapshots / 999999999999 4",
    },
];

impl MacScrap {
    pub fn render_performance(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = segmented().children(["Processes", "Startup Items", "Maintenance"].into_iter().enumerate().map(|(ix, label)| {
            segment(SharedString::from(format!("pt-{ix}")), label, self.perf_tab == ix).on_click(cx.listener(move |this, _, _, cx| {
                this.perf_tab = ix;
                cx.notify();
            }))
        }));
        let head = header(
            "Performance",
            "See what's working your Mac hardest and control what starts automatically.",
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(tabs)
                .child(icon_button("perf-refresh", "refresh").on_click(cx.listener(|this, _, _, cx| this.load_performance(cx)))),
        );
        let body = match self.perf_tab {
            0 => self.processes_view(cx),
            1 => self.startup_view(cx),
            _ => self.maintenance_view(cx),
        };
        page(head, None, body, None)
    }

    fn processes_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let Job::Done(procs, gen) = &self.processes else {
            return scanning_view("Measuring activity", "Sampling CPU and memory".into(), None).into_any_element();
        };
        let gen = *gen;
        let mut by_app = std::collections::BTreeMap::<Option<std::path::PathBuf>, Vec<_>>::new();
        for proc_ in procs.iter().cloned() {
            by_app.entry(proc_.app_bundle.clone()).or_default().push(proc_);
        }
        let mut groups: Vec<_> = by_app.into_iter().collect();
        groups.sort_by(|(_, a), (_, b)| {
            let a_cpu: f32 = a.iter().map(|p| p.cpu).sum();
            let b_cpu: f32 = b.iter().map(|p| p.cpu).sum();
            b_cpu.total_cmp(&a_cpu)
        });
        let sections: Vec<AnyElement> = groups.into_iter().enumerate().map(|(group_ix, (bundle, procs))| {
            let name = bundle.as_ref().and_then(|p| p.file_stem()).map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Other processes".into());
            let cpu: f32 = procs.iter().map(|p| p.cpu).sum();
            let memory: u64 = procs.iter().map(|p| p.memory).sum();
            let count = procs.len();
            let key = bundle.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "other-processes".into());
            let expanded = self.process_groups_expanded.contains(&key);
            let toggle_key = key.clone();
            let rows: Vec<AnyElement> = if expanded { procs.into_iter().enumerate().map(|(ix, proc_)| {
                let busy = proc_.cpu >= 50.0;
                let frac = (proc_.cpu / 100.0).min(1.0);
                let name = proc_.name.clone();
                fade_in(
                    list_row(SharedString::from(format!("p-{}", proc_.pid)))
                        .child(tile_icon(if busy { "zap" } else { "activity" }, if busy { pl.orange } else { pl.text2 }, 30.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(proc_.name.clone()))
                                .child(text(CAPTION, pl.text3).child(format!("PID {} · {} memory", proc_.pid, human_bytes(proc_.memory)))),
                        )
                        .child(div().w(px(90.)).flex_none().child(meter(frac, if busy { pl.orange } else { pl.accent })))
                        .child(text(BODY, if busy { pl.orange } else { pl.text }).font_weight(FontWeight::MEDIUM).w(px(56.)).text_right().child(format!("{:.0}%", proc_.cpu)))
                        .child(if proc_.can_quit {
                            button(SharedString::from(format!("q-{}", proc_.pid)), Kind::Ghost, None, "Quit").on_click(cx.listener(move |_, _, _, cx| {
                                let process = proc_.clone();
                                let name = name.clone();
                                let task = cx.background_executor().spawn(async move { macscrap_core::background::quit_process(&process) });
                                cx.spawn(async move |this, cx| {
                                    let result = task.await;
                                    this.update(cx, |this, cx| {
                                        match result {
                                            Ok(()) => this.show_toast(format!("Asked {name} to quit"), true),
                                            Err(e) => this.show_toast(format!("Couldn't quit {name}: {e}"), false),
                                        }
                                        this.load_performance(cx);
                                        cx.notify();
                                    }).ok();
                                }).detach();
                            })).into_any_element()
                        } else {
                            pill("Protected", pl.text3).into_any_element()
                        }),
                    format!("pr-{gen}-{group_ix}-{ix}"),
                    ix,
                )
            })
            .collect() } else { Vec::new() };
            div().flex().flex_col().gap(px(6.))
                .child(div().id(SharedString::from(format!("process-group-{key}"))).flex().items_center().gap(px(8.))
                    .px(px(8.)).py(px(8.)).rounded(px(8.)).cursor_pointer().hover(|s| s.bg(c(pl.hover)))
                    .child(icon(if expanded { "chevron-down" } else { "chevron-right" }, pl.text3, 14.))
                    .child(div().flex_1().min_w_0()
                    .child(strong(BODY, pl.text).child(name))
                    )
                    .child(text(CAPTION, pl.text3).child(format!("{count} processes · {cpu:.0}% CPU · {} memory", human_bytes(memory))))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.process_groups_expanded.remove(&toggle_key) {
                            this.process_groups_expanded.insert(toggle_key.clone());
                        }
                        cx.notify();
                    })))
                .children(expanded.then(|| grouped(rows))).into_any_element()
        }).collect();
        scroll_body("procs")
            .gap(px(14.))
            .child(note("info", "Expand an app to see its processes. Only verified third-party app processes can be quit; quitting can lose unsaved work."))
            .children(sections)
            .into_any_element()
    }

    fn startup_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let Job::Done(items, gen) = &self.launch else {
            return scanning_view("Reading startup items", "Checking launch agents and daemons".into(), None).into_any_element();
        };
        let gen = *gen;
        let items: Vec<_> = items.iter().filter(|i| !i.is_apple()).cloned().collect();
        if items.is_empty() {
            return empty_view("check-circle", pl.green, "No third-party startup items", "Nothing besides macOS starts automatically.", None).into_any_element();
        }
        let rows: Vec<AnyElement> = items
            .into_iter()
            .enumerate()
            .map(|(ix, item)| {
                let user = item.scope == Scope::UserAgent;
                let (tile_color, tile) = if item.broken {
                    (pl.red, "alert")
                } else if item.signer.is_none() && item.program.is_some() {
                    (pl.orange, "power")
                } else {
                    (pl.text2, "power")
                };
                let it = item.clone();
                let plist = item.plist.clone();
                let detail = item
                    .signer
                    .as_ref()
                    .map(|s| s.trim_start_matches("Developer ID Application: ").to_string())
                    .or_else(|| item.program.as_ref().map(|p| short_path(p)))
                    .unwrap_or_default();
                fade_in(
                    list_row(SharedString::from(format!("li-{ix}")))
                        .child(tile_icon(tile, tile_color, 30.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(item.label.clone()))
                                .child(text(CAPTION, pl.text3).truncate().child(detail)),
                        )
                        .when(item.broken, |d| d.child(pill("Broken", pl.red)))
                        .when(!item.broken && item.signer.is_none() && item.program.is_some(), |d| d.child(pill("Unsigned", pl.orange)))
                        .when(item.loaded, |d| d.child(pill("Running", pl.green)))
                        .child(pill(item.scope.label(), pl.text2))
                        .child(icon_button(SharedString::from(format!("lirv-{ix}")), "folder").on_click(move |_, _, _| reveal(&plist)))
                        .child(if user {
                            button(SharedString::from(format!("rm-{ix}")), Kind::Ghost, None, "Remove")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    match macscrap_core::background::disable(&it) {
                                        Ok(()) => this.show_toast(format!("Removed {}", it.label), true),
                                        Err(e) => this.show_toast(e, false),
                                    }
                                    this.load_performance(cx);
                                }))
                                .into_any_element()
                        } else {
                            div().w(px(68.)).flex().justify_center().child(icon("lock", pl.text3, 13.)).into_any_element()
                        }),
                    format!("li-{gen}-{ix}"),
                    ix,
                )
            })
            .collect();
        scroll_body("startup")
            .gap(px(10.))
            .child(note("info", "Removing a user item stops it now and on next login. System items (lock icon) need an administrator."))
            .child(grouped(rows))
            .into_any_element()
    }

    fn maintenance_view(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let rows: Vec<AnyElement> = TASKS
            .iter()
            .map(|t| {
                let running = self.maintenance_running == Some(t.id);
                let (id, title, cmd) = (t.id, t.title, t.command);
                list_row(SharedString::from(format!("m-{id}")))
                    .py(px(10.))
                    .child(tile_icon(t.icon, pl.accent, 34.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(2.))
                            .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).child(t.title))
                            .child(text(CAPTION, pl.text2).child(t.description)),
                    )
                    .child(if running {
                        div().w(px(72.)).flex().justify_center().child(spinner(16., pl.accent)).into_any_element()
                    } else {
                        button(SharedString::from(format!("run-{id}")), Kind::Secondary, Some("lock"), "Run")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if this.maintenance_running.is_some() {
                                    return;
                                }
                                this.maintenance_running = Some(id);
                                let task = cx.background_executor().spawn(async move { run_as_admin(cmd) });
                                cx.spawn(async move |this, cx| {
                                    let r = task.await;
                                    this.update(cx, |this, cx| {
                                        this.maintenance_running = None;
                                        match r {
                                            Ok(()) => this.show_toast(format!("{title} finished"), true),
                                            Err(e) if e.contains("-128") => {} // user cancelled the password prompt
                                            Err(e) => this.show_toast(format!("{title} failed: {e}"), false),
                                        }
                                        cx.notify();
                                    })
                                    .ok();
                                })
                                .detach();
                                cx.notify();
                            }))
                            .into_any_element()
                    })
                    .into_any_element()
            })
            .collect();
        scroll_body("maint")
            .gap(px(10.))
            .child(note("lock", "Each task asks for your administrator password through the standard macOS prompt."))
            .child(grouped(rows))
            .into_any_element()
    }
}

pub fn note(icon_name: &str, message: &str) -> Div {
    let pl = p();
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .px(px(4.))
        .child(icon(icon_name, pl.text3, 13.))
        .child(text(CAPTION, pl.text3).child(message.to_string()))
}
