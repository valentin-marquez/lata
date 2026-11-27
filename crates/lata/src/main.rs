// A release build on Windows is a window, not a console program with a window: without this
// a terminal opens next to lata every time.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod account;
mod assets;
mod characters;
mod discover;
mod game;
mod instance;
mod library;
mod link;
mod news;
mod pack;
mod runtime;
mod screenshots;
mod settings;
mod shell;
mod storage;
mod theme;
mod tray;
mod ui;
mod update;
mod workshop;

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
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Before anything else, and touching nothing: an update asks the lata it downloaded for
    // this, to know it starts on this computer before it replaces the one that works.
    if args.first().is_some_and(|arg| arg == "--version") {
        println!("lata {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    instance::store::adopt_old_name();
    i18n::init();
    let started_with = link::from_args(args.iter().cloned());
    // Only one lata per data folder: a second one hands its links over (or asks the first to
    // show itself) and leaves, instead of opening another window. Not when a lata started
    // this one to take its place (a restart, an update): that one is still closing, and
    // would take the message with it.
    let lock = instance::store::data().join("lata.lock");
    let restarted = args
        .iter()
        .any(|arg| arg == update::RESTARTED || arg == update::UPDATED);
    if !restarted && link::single::forward(&lock, &link::single::messages(&started_with)) {
        return;
    }

    // What reaches this lata from outside: the links it was started with, those another lata
    // hands over, and (on macOS, which delivers them this way) links that arrive while it runs.
    let (outside, incoming) = tokio::sync::mpsc::unbounded_channel::<link::Incoming>();
    for link in started_with {
        let _ = outside.send(link::Incoming::Link(link));
    }
    let (from_others, back) = (outside.clone(), outside.clone());
    tray::connect(outside.clone());
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
        // The update window comes first; it opens the main one unless lata replaces itself.
        update::start(&args, back, incoming, open, cx);
    });
}

/// Opens the main window, and delivers to it what reaches lata from outside.
fn open(mut incoming: tokio::sync::mpsc::UnboundedReceiver<link::Incoming>, cx: &mut App) {
    // Held here and not only by the window: on Linux going to the tray closes the window, and
    // the downloads, running games and the rest carry on in the shell for the next one.
    let shell = cx.new(Shell::new);
    let mut handle = window(&shell, cx);
    // With the tray on, closing the window can leave lata running without it. After the window
    // opens: the Windows icon is sized for the scale of the first window, and when no update
    // splash came first there is none before this one.
    tray::apply(cx);
    // Once the window is up: a dialog needs its root view in place.
    let first = shell.downgrade();
    let _ = handle.update(cx, |_, window, cx| {
        window.defer(cx, move |window, cx| {
            Shell::welcome(first, window, cx);
            // Here and not at the update splash: after an update it is this window, of the
            // new lata, that says what changed.
            news::start(window, cx);
        })
    });
    cx.spawn(async move |cx| {
        while let Some(message) = incoming.recv().await {
            // The window closed when lata went to the tray (Linux): a new one opens on the same
            // shell for a link or a Show.
            let gone = handle.update(cx, |_, _, _| ()).is_err();
            if gone && message != link::Incoming::Quit {
                handle = cx.update(|cx| {
                    shell.update(cx, |shell, cx| shell.new_window(cx));
                    window(&shell, cx)
                });
            }
            match message {
                link::Incoming::Link(text) => {
                    cx.update(|cx| {
                        shell.update(cx, |shell, cx| shell.receive_link(text, handle, cx))
                    });
                }
                // Also brings the window back from the tray.
                link::Incoming::Show => {
                    let _ = handle.update(cx, |_, window, cx| tray::show(window, cx));
                }
                link::Incoming::Quit => {
                    cx.update(tray::quit);
                }
            }
        }
    })
    .detach();
    // Only now is the lata an update replaced let go: this one got as far as its window.
    update::tidy();
}

/// Opens a main window on `shell`.
fn window(shell: &Entity<Shell>, cx: &mut App) -> AnyWindowHandle {
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
    let shell = shell.clone();
    gpui_kit::open_window(options, cx, move |window, cx| {
        theme::follow_system(window, cx);
        // With the tray on, closing the window hides it instead of quitting lata.
        window.on_window_should_close(cx, tray::close);
        shell
    })
    .expect("failed to open main window")
    .0
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_icon_decodes() {
        let icon = super::window_icon().expect("icon must decode");
        assert_eq!(icon.dimensions(), (256, 256));
    }
}
