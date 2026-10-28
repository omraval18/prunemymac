//! Page renderers (each an `impl MacScrap` block) and helpers they share.

mod cleanup;
mod files;
mod overview;
mod network;
mod performance;
mod protection;
mod space_lens;
mod uninstaller;

use crate::theme::*;
use gpui::prelude::*;
use gpui::{div, px, AnyElement, Div};
use macscrap_core::fsutil;
use std::path::Path;
use std::time::SystemTime;

/// Standard page frame: header row (title + actions), optional toolbar,
/// scrollable body, optional sticky action bar.
pub fn page(header: impl IntoElement, toolbar: Option<AnyElement>, body: impl IntoElement, footer: Option<AnyElement>) -> Div {
    div()
        .flex_1()
        .min_h_0()
        .flex()
        .flex_col()
        .child(div().px(px(28.)).pt(px(4.)).pb(px(16.)).flex_none().child(header))
        .children(toolbar.map(|t| div().px(px(28.)).pb(px(12.)).flex_none().child(t)))
        .child(div().flex_1().min_h_0().flex().flex_col().child(body))
        .children(footer)
}

pub fn header(title: &str, subtitle: &str, actions: impl IntoElement) -> Div {
    div()
        .flex()
        .items_end()
        .justify_between()
        .gap(px(16.))
        .w_full()
        .min_w_0()
        .child(page_header(title, subtitle))
        .child(div().flex().flex_none().items_center().gap(px(8.)).child(actions))
}

/// Scrollable padded area for list content.
pub fn scroll_body(id: &'static str) -> gpui::Stateful<Div> {
    div().id(id).flex_1().min_h_0().overflow_y_scroll().px(px(28.)).pb(px(24.)).flex().flex_col()
}

pub fn fmt_count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn short_path(p: &Path) -> String {
    match p.strip_prefix(fsutil::home()) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}

pub fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.display().to_string())
}

pub fn parent_display(p: &Path) -> String {
    short_path(p.parent().unwrap_or(p))
}

/// "today", "3 days ago", "5 months ago", "2 years ago".
pub fn ago_days(days: u64) -> String {
    match days {
        0 => "today".into(),
        1 => "yesterday".into(),
        2..=30 => format!("{days} days ago"),
        31..=364 => {
            let m = days / 30;
            if m <= 1 { "a month ago".into() } else { format!("{m} months ago") }
        }
        _ => {
            let y = days / 365;
            if y <= 1 { "a year ago".into() } else { format!("{y} years ago") }
        }
    }
}

pub fn ago(t: Option<SystemTime>) -> String {
    t.and_then(|t| SystemTime::now().duration_since(t).ok())
        .map(|d| ago_days(d.as_secs() / 86_400))
        .unwrap_or_else(|| "unknown".into())
}

/// Icon + color for a file by extension.
pub fn file_kind(p: &Path) -> (&'static str, u32) {
    let pl = crate::theme::p();
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mov" | "mp4" | "mkv" | "avi" | "m4v" | "webm" => ("video", pl.pink),
        "jpg" | "jpeg" | "png" | "heic" | "gif" | "tiff" | "raw" | "psd" | "webp" => ("image", pl.teal),
        "mp3" | "wav" | "aiff" | "flac" | "m4a" | "aac" => ("music", pl.red),
        "zip" | "tar" | "gz" | "tgz" | "xz" | "7z" | "rar" | "bz2" | "zst" => ("archive", pl.orange),
        "dmg" | "iso" | "img" | "pkg" | "xip" => ("disc", pl.indigo),
        "app" => ("app-window", pl.accent),
        "gguf" | "safetensors" | "bin" | "pt" | "onnx" | "mlmodel" => ("brain", pl.purple),
        "rs" | "js" | "ts" | "py" | "swift" | "go" | "json" | "sql" | "db" | "sqlite" => ("code", pl.green),
        _ => ("file", pl.text2),
    }
}

pub fn running_detail(progress: Option<(u64, u64)>, noun: &str) -> String {
    match progress {
        Some((n, secs)) if n > 0 => format!("{} {noun} · {secs}s", fmt_count(n)),
        Some((_, secs)) => format!("{secs}s"),
        None => String::new(),
    }
}

pub fn size_text(bytes: u64) -> Div {
    text(BODY, p().text).font_weight(gpui::FontWeight::MEDIUM).w(px(84.)).flex_none().text_right().child(macscrap_core::human_bytes(bytes))
}

/// Group of rows on a card, separated by hairlines (macOS Settings style).
pub fn grouped(rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    let mut d = card().p(px(4.)).flex().flex_col();
    for (i, r) in rows.into_iter().enumerate() {
        d = d.child(r);
        if i + 1 < n {
            d = d.child(div().px(px(12.)).child(divider()));
        }
    }
    d
}
