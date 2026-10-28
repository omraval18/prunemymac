//! Sampled network connections and local ports visible to this user.

use super::performance::note;
use super::*;
use crate::app::MacScrap;
use gpui::{Context, FontWeight};

impl MacScrap {
    pub fn render_network(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = segmented()
            .child(segment("net-connections", "Connections", self.network_tab == 0).on_click(cx.listener(|this, _, _, cx| {
                this.network_tab = 0;
                this.refresh_network(cx);
                cx.notify();
            })))
            .child(segment("net-ports", "Ports", self.network_tab == 1).on_click(cx.listener(|this, _, _, cx| {
                this.network_tab = 1;
                this.refresh_ports(cx);
                cx.notify();
            })));
        let actions = div().flex().items_center().gap(px(8.)).child(tabs)
            .child(icon_button("net-refresh", "refresh").on_click(cx.listener(|this, _, _, cx| {
                if this.network_tab == 0 { this.refresh_network(cx); } else { this.refresh_ports(cx); }
            })));
        let subtitle = if self.network_tab == 0 { "Live TCP connections visible to your user account." }
            else { "Local TCP listeners and UDP sockets visible to your user account." };
        let head = header("Network", subtitle, actions);
        if self.network_tab == 0 {
            let filters = segmented()
                .child(segment("net-all", "All connections", !self.network_review_only).on_click(cx.listener(|this, _, _, cx| {
                    this.network_review_only = false;
                    cx.notify();
                })))
                .child(segment("net-review", "Needs review", self.network_review_only).on_click(cx.listener(|this, _, _, cx| {
                    this.network_review_only = true;
                    cx.notify();
                })));
            page(head, Some(filters.into_any_element()), self.connections_view(), None)
        } else {
            page(head, None, self.ports_view(cx), None)
        }
    }

    fn connections_view(&self) -> AnyElement {
        let pl = p();
        let mut body = scroll_body("network-list").gap(px(12.))
            .child(note("info", "Refreshes while this page is open. Short connections, other users' processes, UDP, and request details may be missing. Connection direction is not shown."));
        match &self.network {
            None => body = body.child(note("loader", "Reading connections…")),
            Some(Err(error)) => body = body.child(note("alert", error)),
            Some(Ok(connections)) => {
                let shown: Vec<_> = connections.iter().filter(|c| !self.network_review_only || c.review_reason.is_some()).collect();
                let review_count = connections.iter().filter(|c| c.review_reason.is_some()).count();
                body = body.child(text(CAPTION, pl.text2).child(format!("{} connections · {} need review", connections.len(), review_count)));
                if shown.is_empty() {
                    body = body.child(note("info", if self.network_review_only {
                        "No connections currently match the review indicators."
                    } else {
                        "No visible established TCP connections right now."
                    }));
                } else {
                    let rows: Vec<AnyElement> = shown.into_iter().enumerate().map(|(ix, connection)| {
                        let reason = connection.review_reason;
                        let color = if reason.is_some() { pl.orange } else { pl.text2 };
                        let detail = connection.executable.as_ref().map(|path| short_path(path))
                            .unwrap_or_else(|| "Executable path unavailable".into());
                        list_row(format!("net-row-{ix}"))
                            .child(tile_icon(if reason.is_some() { "alert" } else { "globe" }, color, 30.))
                            .child(div().flex_1().min_w_0().flex().flex_col()
                                .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate()
                                    .child(if connection.pid == 0 { connection.process.clone() } else { format!("{} · PID {}", connection.process, connection.pid) }))
                                .child(text(CAPTION, pl.text3).truncate().child(detail)))
                            .child(div().w(px(250.)).flex_none().flex().flex_col()
                                .child(text(BODY, pl.text).truncate().child(connection.remote.clone()))
                                .child(text(CAPTION, pl.text3).truncate().child(format!("Local: {}", connection.local))))
                            .when_some(reason, |row, reason| row.child(pill(reason, pl.orange)))
                            .into_any_element()
                    }).collect();
                    body = body.child(grouped(rows));
                }
                body = body.child(note("info", "Review indicators describe where a process runs from; they do not establish that it is malicious. Unflagged connections are not verified safe."));
            }
        }
        body.into_any_element()
    }

    fn ports_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let pl = p();
        let mut body = scroll_body("network-ports-list").gap(px(12.))
            .child(note("info", "Shows listening TCP ports and bound UDP sockets visible to your account. Some system or other users' ports may be missing."))
            .child(note("lock", if cfg!(target_os = "linux") {
                "System processes, privileged ports, and unrecognized programs are protected. Stopping a program can interrupt work or services."
            } else {
                "System, privileged, and unverified processes are protected. Stopping an app may interrupt work or services even on a high-numbered port."
            }));
        match &self.network_ports {
            None => body = body.child(note("loader", "Reading ports…")),
            Some(Err(error)) => body = body.child(note("alert", error)),
            Some(Ok(ports)) if ports.is_empty() => body = body.child(note("info", "No visible listening TCP ports or bound UDP sockets.")),
            Some(Ok(ports)) => {
                body = body.child(text(CAPTION, pl.text2).child(format!("{} visible sockets", ports.len())));
                let rows: Vec<AnyElement> = ports.iter().cloned().enumerate().map(|(ix, port)| {
                    let locked = port.locked_reason;
                    let process = port.process.clone();
                    let detail = port.executable.as_ref().map(|p| short_path(p)).unwrap_or_else(|| "Executable path unavailable".into());
                    let action = if let Some(reason) = locked {
                        pill(reason, pl.text3).into_any_element()
                    } else if self.port_stop_pending {
                        pill("Stopping…", pl.text3).into_any_element()
                    } else {
                        let target = port.clone();
                        button(format!("port-stop-{ix}"), Kind::Secondary, Some("stop"), "Stop…")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.port_confirm = Some(target.clone());
                                cx.notify();
                            })).into_any_element()
                    };
                    list_row(format!("port-{ix}"))
                        .child(tile_icon("wifi", pl.text2, 30.))
                        .child(div().flex_1().min_w_0().flex().flex_col()
                            .child(text(BODY, pl.text).font_weight(FontWeight::MEDIUM).truncate().child(if port.pid == 0 { process } else { format!("{process} · PID {}", port.pid) }))
                            .child(text(CAPTION, pl.text3).truncate().child(detail)))
                        .child(div().w(px(150.)).flex_none().flex().flex_col()
                            .child(strong(BODY, pl.text).child(format!("{} {}", port.protocol, port.number)))
                            .child(text(CAPTION, pl.text3).truncate().child(port.address.clone())))
                        .child(action)
                        .into_any_element()
                }).collect();
                body = body.child(grouped(rows));
            }
        }
        body.into_any_element()
    }
}
