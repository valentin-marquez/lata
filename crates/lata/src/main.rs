// A release build on Windows is a window, not a console program with a window: without this
// a terminal opens next to lata every time.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod account;
mod assets;
mod discover;
mod game;
mod instance;
mod library;
mod link;
mod pack;
mod runtime;
mod screenshots;
mod settings;
mod shell;
mod storage;
mod theme;
mod ui;

use std::sync::Arc;

use gpui_kit::component::TitleBar;
use gpui_kit::*;

use crate::assets::AppAssets;
use crate::shell::Shell;

// Must match the .desktop file name so Wayland compositors resolve the icon.
const APP_ID: &str = "io.github.valentin_marquez.Lata";
const ICON_PNG: &[u8] = include_bytes!("../../../assets/icons/png/lata-256.png");

fn window_icon() -> Option<Arc<image::RgbaImage>> {
    image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        .ok()
        .map(|img| Arc::new(img.into_rgba8()))
}

fn main() {
    instance::store::adopt_old_name();
    let started_with = link::from_args(std::env::args().skip(1));
    // Only one lata per data folder: a second one hands its links over (or asks the first to
    // show itself) and leaves, instead of opening another window.
    let lock = instance::store::data().join("lata.lock");
    if link::single::forward(&lock, &link::single::messages(&started_with)) {
        return;
    }

    // What reaches this lata from outside: the links it was started with, those another lata
    // hands over, and (on macOS, which delivers them this way) links that arrive while it runs.
    let (outside, mut incoming) = tokio::sync::mpsc::unbounded_channel::<link::Incoming>();
    for link in started_with {
        let _ = outside.send(link::Incoming::Link(link));
    }
    let from_others = outside.clone();
    if let Err(err) = link::single::serve(&lock, move |message| {
        let _ = from_others.send(message);
    }) {
        eprintln!("couldn't listen for a second lata: {err}");
    }
    let app = gpui_kit::application().with_assets(AppAssets);
    app.on_open_urls(move |urls| {
        for url in link::from_args(urls) {
            let _ = outside.send(link::Incoming::Link(url));
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
                while let Some(message) = incoming.recv().await {
                    match message {
                        link::Incoming::Link(text) => {
                            let _ =
                                weak.update(cx, |shell, cx| shell.receive_link(text, handle, cx));
                        }
                        link::Incoming::Show => {
                            let _ = handle.update(cx, |_, window, _| window.activate_window());
                        }
                    }
                }
            })
            .detach();
            // Once the window is up: a dialog needs its root view in place.
            let first = shell.downgrade();
            window.defer(cx, move |window, cx| Shell::welcome(first, window, cx));
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
