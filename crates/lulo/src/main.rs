mod account;
mod assets;
mod discover;
mod game;
mod instance;
mod library;
mod link;
mod runtime;
mod screenshots;
mod settings;
mod shell;
mod theme;
mod ui;

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::AppAssets;
use crate::shell::Shell;

// Must match the .desktop file name so Wayland compositors resolve the icon.
const APP_ID: &str = "io.github.valentin_marquez.Lulo";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/png/lulo-256.png");

fn window_icon() -> Option<Arc<image::RgbaImage>> {
    image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()
        .map(|img| Arc::new(img.into_rgba8()))
}

fn main() {
    // Links to open, from wherever they come: what the system started lulo with, and (on
    // macOS, which hands them over this way) the ones that arrive while it runs.
    let (links, mut incoming) = tokio::sync::mpsc::unbounded_channel::<String>();
    for link in link::from_args(std::env::args().skip(1)) {
        let _ = links.send(link);
    }
    let app = gpui_kit::application().with_assets(AppAssets);
    app.on_open_urls(move |urls| {
        for url in link::from_args(urls) {
            let _ = links.send(url);
        }
    });
    app.run(move |cx| {
        gpui_kit::init(cx);
        theme::init(cx);

        let options = WindowOptions {
            app_id: Some(APP_ID.into()),
            icon: window_icon(),
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            window_min_size: Some(size(px(900.), px(600.))),
            titlebar: Some(TitlebarOptions {
                // Centre macOS traffic lights in the taller title bar.
                traffic_light_position: Some(point(px(16.), px(17.))),
                ..TitleBar::title_bar_options()
            }),
            ..TitleBar::window_options()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            theme::follow_system(window, cx);
            let shell = cx.new(Shell::new);
            let (handle, weak) = (window.window_handle(), shell.downgrade());
            cx.spawn(async move |cx| {
                while let Some(text) = incoming.recv().await {
                    let _ = weak.update(cx, |shell, cx| shell.receive_link(text, handle, cx));
                }
            })
            .detach();
            shell
        })
        .expect("failed to open main window");
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_icon_decodes() {
        let icon = super::window_icon().expect("icon must decode");
        assert_eq!(icon.dimensions(), (256, 256));
    }
}
