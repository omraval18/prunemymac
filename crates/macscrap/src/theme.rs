//! Design system: palette (light + dark, follows macOS), type scale, and the
//! small set of components every page is built from. Flat colors only.

use gpui::prelude::*;
use gpui::{
    div, ease_out_quint, percentage, px, rgb, svg, Animation, AnimationExt, AnyElement, Div, FontWeight, Hsla,
    SharedString, Stateful, Transformation,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub struct Palette {
    pub bg: u32,
    pub sidebar: u32,
    pub surface: u32,
    pub surface2: u32,
    pub hover: u32,
    pub pressed: u32,
    pub border: u32,
    pub border_strong: u32,
    pub text: u32,
    pub text2: u32,
    pub text3: u32,
    pub accent: u32,
    pub accent_hover: u32,
    pub accent_pressed: u32,
    pub on_accent: u32,
    pub green: u32,
    pub orange: u32,
    pub red: u32,
    pub purple: u32,
    pub teal: u32,
    pub indigo: u32,
    pub pink: u32,
}

// Apple system colors so status colors read the way macOS users expect.
const DARK: Palette = Palette {
    bg: 0x111113,
    sidebar: 0x161618,
    surface: 0x1a1a1d,
    surface2: 0x202024,
    hover: 0x252529,
    pressed: 0x2c2c31,
    border: 0x29292e,
    border_strong: 0x36363c,
    text: 0xf2f2f4,
    text2: 0xa2a2ab,
    text3: 0x6c6c75,
    accent: 0x0a84ff,
    accent_hover: 0x2793ff,
    accent_pressed: 0x0071e3,
    on_accent: 0xffffff,
    green: 0x30d158,
    orange: 0xff9f0a,
    red: 0xff453a,
    purple: 0xbf5af2,
    teal: 0x40c8e0,
    indigo: 0x5e5ce6,
    pink: 0xff375f,
};

const LIGHT: Palette = Palette {
    bg: 0xffffff,
    sidebar: 0xf5f5f7,
    surface: 0xffffff,
    surface2: 0xf5f5f7,
    hover: 0xefeff2,
    pressed: 0xe6e6ea,
    border: 0xe5e5ea,
    border_strong: 0xd2d2d8,
    text: 0x1d1d1f,
    text2: 0x6e6e73,
    text3: 0x9b9ba1,
    accent: 0x007aff,
    accent_hover: 0x1a88ff,
    accent_pressed: 0x0068d9,
    on_accent: 0xffffff,
    green: 0x28b04a,
    orange: 0xf08c00,
    red: 0xff3b30,
    purple: 0xaf52de,
    teal: 0x1aa5c0,
    indigo: 0x5856d6,
    pink: 0xff2d55,
};

static IS_DARK: AtomicBool = AtomicBool::new(true);
static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

pub fn set_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

/// Honor System Settings → Accessibility → Display → Reduce motion.
pub fn load_motion_preference() {
    let reduce = std::process::Command::new("/usr/bin/defaults")
        .args(["read", "com.apple.universalaccess", "reduceMotion"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "1");
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
}

pub fn p() -> &'static Palette {
    if is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}

/// Flat tint of a color for icon tiles, pills and banners.
pub fn tint(hex: u32) -> Hsla {
    c(hex).opacity(if is_dark() { 0.16 } else { 0.12 })
}

// ───────────────────────────── Type scale ─────────────────────────────

pub const TITLE: f32 = 22.;
pub const HEADLINE: f32 = 15.;
pub const BODY: f32 = 13.;
pub const CAPTION: f32 = 12.;
pub const MICRO: f32 = 11.;

pub fn text(size: f32, color: u32) -> Div {
    div().text_size(px(size)).text_color(c(color))
}

pub fn strong(size: f32, color: u32) -> Div {
    text(size, color).font_weight(FontWeight::SEMIBOLD)
}

// ───────────────────────────── Motion ─────────────────────────────

/// Fade an element in once, `index` steps after its siblings (30 ms
/// stagger, capped). Used only where results arrive, never on navigation.
pub fn fade_in<E: IntoElement + Styled + 'static>(el: E, id: impl Into<SharedString>, index: usize) -> AnyElement {
    if REDUCE_MOTION.load(Ordering::Relaxed) {
        return el.into_any_element();
    }
    let delay = (index.min(12) as f32) * 0.03;
    let dur = 0.22;
    let total = delay + dur;
    let ease = ease_out_quint();
    let id: SharedString = id.into();
    el.with_animation(
        gpui::ElementId::Name(id),
        Animation::new(Duration::from_secs_f32(total)),
        move |el, t| {
            let local = ((t * total - delay) / dur).clamp(0., 1.);
            el.opacity(ease(local))
        },
    )
    .into_any_element()
}

pub fn spinner(size: f32, color: u32) -> impl IntoElement {
    svg()
        .path("icons/loader.svg")
        .size(px(size))
        .flex_none()
        .text_color(c(color))
        .with_animation(
            "spinner",
            Animation::new(Duration::from_millis(800)).repeat(),
            |s, t| s.with_transformation(Transformation::rotate(percentage(t))),
        )
}

// ───────────────────────────── Components ─────────────────────────────

pub fn icon(name: &str, color: u32, size: f32) -> gpui::Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(c(color))
}

/// Rounded square with a tinted background, used to label categories.
pub fn tile_icon(name: &str, color: u32, size: f32) -> Div {
    div()
        .size(px(size))
        .flex_none()
        .rounded(px(size * 0.28))
        .bg(tint(color))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(name, color, size * 0.55))
}

pub fn card() -> Div {
    div()
        .bg(c(p().surface))
        .border_1()
        .border_color(c(p().border))
        .rounded(px(12.))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

pub fn button(id: impl Into<SharedString>, kind: Kind, icon_name: Option<&str>, label: impl Into<SharedString>) -> Stateful<Div> {
    let pl = p();
    let (bg, hover, pressed, fg, border) = match kind {
        Kind::Primary => (pl.accent, pl.accent_hover, pl.accent_pressed, pl.on_accent, None),
        Kind::Secondary => (pl.surface2, pl.hover, pl.pressed, pl.text, Some(pl.border)),
        Kind::Ghost => (0, pl.hover, pl.pressed, pl.text2, None),
        Kind::Danger => (pl.red, pl.red, pl.red, 0xffffff, None),
    };
    let ghost = kind == Kind::Ghost;
    div()
        .id(id.into())
        .h(px(30.))
        .px(px(if icon_name.is_some() { 11. } else { 13. }))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(6.))
        .rounded(px(8.))
        .when(!ghost, |d| d.bg(c(bg)))
        .when_some(border, |d, b| d.border_1().border_color(c(b)))
        .text_size(px(BODY))
        .font_weight(FontWeight::MEDIUM)
        .text_color(c(fg))
        .cursor_pointer()
        .hover(move |s| if kind == Kind::Danger { s.opacity(0.9) } else { s.bg(c(hover)) })
        .active(move |s| if kind == Kind::Danger { s.opacity(0.8) } else { s.bg(c(pressed)) })
        .when_some(icon_name, |d, name| d.child(icon(name, fg, 15.)))
        .child(label.into())
}

pub fn icon_button(id: impl Into<SharedString>, icon_name: &str) -> Stateful<Div> {
    let pl = p();
    div()
        .id(id.into())
        .size(px(28.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .cursor_pointer()
        .hover(|s| s.bg(c(pl.hover)))
        .active(|s| s.bg(c(pl.pressed)))
        .child(icon(icon_name, pl.text2, 15.))
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Check {
    On,
    Off,
    Mixed,
}

impl From<bool> for Check {
    fn from(b: bool) -> Self {
        if b {
            Check::On
        } else {
            Check::Off
        }
    }
}

pub fn checkbox(state: impl Into<Check>) -> Div {
    let state = state.into();
    let pl = p();
    let filled = state != Check::Off;
    div()
        .size(px(16.))
        .flex_none()
        .rounded(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .when(filled, |d| d.bg(c(pl.accent)))
        .when(!filled, |d| d.border_1().border_color(c(pl.border_strong)).bg(c(pl.surface)))
        .when(state == Check::On, |d| d.child(icon("check", pl.on_accent, 11.)))
        .when(state == Check::Mixed, |d| d.child(icon("minus", pl.on_accent, 11.)))
}

pub fn pill(label: impl Into<SharedString>, color: u32) -> Div {
    div()
        .h(px(20.))
        .px(px(7.))
        .flex()
        .flex_none()
        .items_center()
        .rounded(px(6.))
        .bg(tint(color))
        .text_size(px(MICRO))
        .font_weight(FontWeight::MEDIUM)
        .text_color(c(color))
        .child(label.into())
}

/// Thin horizontal meter, `frac` in 0..=1.
pub fn meter(frac: f32, color: u32) -> Div {
    div()
        .h(px(4.))
        .w_full()
        .rounded_full()
        .bg(c(p().border))
        .child(div().h_full().rounded_full().bg(c(color)).w(gpui::relative(frac.clamp(0.0, 1.0))))
}

pub fn section_label(label: &str) -> Div {
    text(MICRO, p().text3).font_weight(FontWeight::SEMIBOLD).child(label.to_uppercase())
}

pub fn page_header(title: &str, subtitle: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .min_w_0()
        .child(text(TITLE, p().text).font_weight(FontWeight::BOLD).child(title.to_string()))
        .child(text(BODY, p().text2).child(subtitle.to_string()))
}

/// Centered placeholder: icon, title, one line of guidance, optional action.
pub fn empty_view(icon_name: &str, color: u32, title: &str, body: &str, action: Option<AnyElement>) -> Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .pb(px(40.))
        .child(tile_icon(icon_name, color, 52.).mb(px(4.)))
        .child(strong(HEADLINE, p().text).child(title.to_string()))
        .child(text(BODY, p().text2).max_w(px(380.)).text_center().child(body.to_string()))
        .children(action.map(|a| div().pt(px(8.)).child(a)))
}

/// Centered progress state shown while a job runs.
pub fn scanning_view(title: &str, detail: String, stop: Option<AnyElement>) -> Div {
    let pl = p();
    div()
        .flex_1()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(10.))
        .pb(px(40.))
        .child(
            div()
                .size(px(52.))
                .rounded_full()
                .bg(tint(pl.accent))
                .flex()
                .items_center()
                .justify_center()
                .mb(px(4.))
                .child(spinner(24., pl.accent)),
        )
        .child(strong(HEADLINE, pl.text).child(title.to_string()))
        .child(text(BODY, pl.text2).max_w(px(420.)).text_center().truncate().child(detail))
        .children(stop.map(|s| div().pt(px(6.)).child(s)))
}

/// Sticky footer that shows the current selection and the page's main action.
pub fn action_bar(summary: impl IntoElement, actions: impl IntoElement) -> Div {
    div()
        .flex_none()
        .h(px(60.))
        .px(px(28.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .border_t_1()
        .border_color(c(p().border))
        .bg(c(p().bg))
        .child(summary)
        .child(div().flex().items_center().gap(px(8.)).child(actions))
}

pub fn segmented() -> Div {
    div().flex().flex_none().p(px(2.)).gap(px(2.)).rounded(px(8.)).bg(c(p().surface2)).border_1().border_color(c(p().border))
}

pub fn segment(id: impl Into<SharedString>, label: impl Into<SharedString>, active: bool) -> Stateful<Div> {
    let pl = p();
    div()
        .id(id.into())
        .h(px(24.))
        .px(px(10.))
        .flex()
        .items_center()
        .rounded(px(6.))
        .text_size(px(CAPTION))
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .when(active, |d| d.bg(c(if is_dark() { pl.pressed } else { pl.surface })).text_color(c(pl.text)).shadow_sm())
        .when(!active, |d| d.text_color(c(pl.text2)).hover(|s| s.text_color(c(pl.text))))
        .child(label.into())
}

/// A list row: hover highlight, consistent height and spacing.
pub fn list_row(id: impl Into<SharedString>) -> Stateful<Div> {
    let pl = p();
    div()
        .id(id.into())
        .min_h(px(44.))
        .px(px(12.))
        .py(px(7.))
        .flex()
        .items_center()
        .gap(px(12.))
        .rounded(px(8.))
        .hover(|s| s.bg(c(pl.hover)))
}

pub fn divider() -> Div {
    div().h(px(1.)).w_full().bg(c(p().border))
}

/// Circular progress ring, `frac` in 0..=1, drawn from 12 o'clock clockwise.
/// `sweep` (a phase in 0..1) adds a short moving highlight on the track, so
/// the ring visibly works even while progress is momentarily flat.
pub fn ring(frac: f32, sweep: Option<f32>, diameter: f32, stroke: f32, track: u32, fill: u32) -> impl IntoElement {
    use gpui::{canvas, point, PathBuilder};
    let (track, fill) = (c(track), c(fill));
    let glow = fill.opacity(0.35);
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let center = bounds.center();
            let r = px(diameter / 2. - stroke / 2.);
            let arc = |from: f32, to: f32| {
                let mut b = PathBuilder::stroke(px(stroke));
                let steps = ((to - from) * 120.).ceil().max(2.) as usize;
                for i in 0..=steps {
                    let t = from + (to - from) * i as f32 / steps as f32;
                    let a = -std::f32::consts::FRAC_PI_2 + t * std::f32::consts::TAU;
                    let p = point(center.x + r * a.cos(), center.y + r * a.sin());
                    if i == 0 {
                        b.move_to(p)
                    } else {
                        b.line_to(p)
                    }
                }
                b.build().ok()
            };
            if let Some(path) = arc(0.0, 1.0) {
                window.paint_path(path, track);
            }
            if let Some(phase) = sweep {
                if let Some(path) = arc(phase, phase + 0.14) {
                    window.paint_path(path, glow);
                }
            }
            if frac > 0.003 {
                if let Some(path) = arc(0.0, frac.min(1.0)) {
                    window.paint_path(path, fill);
                }
            }
        },
    )
    .size(px(diameter))
    .flex_none()
}
