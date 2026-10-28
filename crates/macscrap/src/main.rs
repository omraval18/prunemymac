mod app;
mod pages;
mod sheet;
mod theme;

use gpui::{
    point, prelude::*, px, size, App, Application, AssetSource, Bounds, SharedString, TitlebarOptions, WindowBounds,
    WindowOptions,
};
use std::borrow::Cow;

include!(concat!(env!("OUT_DIR"), "/icons.rs"));

/// Icons are compiled into the binary so the app is a single file.
struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(name, _)| *name == path).map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }
    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().filter(|(n, _)| n.starts_with(path)).map(|(n, _)| SharedString::from(*n)).collect())
    }
}

fn main() {
    // Protection / YARA runs in the sibling `prunemymac-protect` binary so this
    // GUI process does not link cranelift/wasmtime.
    macscrap_core::sys::init_thread_pool();
    theme::load_motion_preference();
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.text_system()
            .add_fonts(vec![
                Cow::Borrowed(include_bytes!("../assets/fonts/Geist.ttf").as_slice()),
                Cow::Borrowed(include_bytes!("../assets/fonts/Inter.ttf").as_slice()),
            ])
            .expect("load bundled fonts");
        let bounds = Bounds::centered(None, size(px(1120.), px(760.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("prunemymac".into()),
                    appears_transparent: true,
                    traffic_light_position: Some(point(px(18.), px(18.))),
                }),
                window_min_size: Some(size(px(920.), px(600.))),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| app::MacScrap::new(window, cx)),
        )
        .unwrap();
        cx.on_window_closed(|cx| cx.quit()).detach();
        cx.activate(true);
    });
}
