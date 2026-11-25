//! The system tray. With "Minimize to the system tray" on, closing the window hides lata to a
//! tray icon instead of quitting: its click or "Show lata" brings the window back, "Quit" ends
//! lata for real. On Windows, on macOS (in the menu bar), and on Linux where the desktop shows
//! StatusNotifierItems; elsewhere the setting stays off.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::link::Incoming;

/// Where the tray's clicks go: the same channel as what reaches lata from outside.
static TO_APP: OnceLock<UnboundedSender<Incoming>> = OnceLock::new();
/// Set once lata is quitting, so closing the window then doesn't hide it again.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Gives the tray the channel its clicks are sent through.
pub fn connect(to_app: UnboundedSender<Incoming>) {
    let _ = TO_APP.set(to_app);
}

#[cfg(any(windows, target_os = "linux", target_os = "macos"))]
fn send(message: Incoming) {
    if let Some(to) = TO_APP.get() {
        let _ = to.send(message);
    }
}

/// Whether lata can sit in the tray on this system, right now.
pub fn supported() -> bool {
    imp::supported()
}

/// Whether the setting is on, here where it can work.
pub fn enabled() -> bool {
    crate::settings::load().minimize_to_tray && supported()
}

/// Puts the icon there or takes it away to match the setting. Call it at startup and when the
/// setting changes.
pub fn apply(cx: &mut App) {
    match (enabled(), imp::has_icon(cx)) {
        (true, false) => imp::create(cx),
        (false, true) => imp::remove(cx),
        _ => {}
    }
}

/// What the main window does when asked to close: `true` lets it close, `false` keeps it. With
/// the icon there lata keeps running with the window hidden (on Linux the window does close, see
/// `imp::hide`); without it lata quits, so the window can't be lost with nothing to bring it back.
pub fn close(window: &mut Window, cx: &mut App) -> bool {
    if !QUITTING.load(Ordering::SeqCst) && imp::has_icon(cx) && enabled() {
        return imp::hide(window, cx);
    }
    // A Linux window that went to the tray before left lata quitting only when told to.
    cx.set_quit_mode(QuitMode::Default);
    true
}

/// Brings the window back from the tray, or to the front.
pub fn show(window: &mut Window, cx: &mut App) {
    imp::unhide(cx);
    window.activate_window();
}

/// Quits lata, whether or not its window is hidden.
pub fn quit(cx: &mut App) {
    QUITTING.store(true, Ordering::SeqCst);
    cx.quit();
}

#[cfg(any(windows, target_os = "macos"))]
mod imp {
    use gpui_kit::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
    #[cfg(windows)]
    use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

    use super::send;
    use crate::link::Incoming;

    const PNG: &[u8] = include_bytes!("../../../../assets/icons/png/lata-32.png");
    const SHOW: &str = "tray-show";
    const QUIT: &str = "tray-quit";

    /// Keeps the icon alive: it goes away when this is dropped.
    struct Held(Option<TrayIcon>);

    impl Global for Held {}

    pub fn supported() -> bool {
        true
    }

    pub fn has_icon(cx: &App) -> bool {
        cx.try_global::<Held>().is_some_and(|held| held.0.is_some())
    }

    pub fn create(cx: &mut App) {
        match build() {
            Ok(icon) => cx.set_global(Held(Some(icon))),
            Err(err) => eprintln!("couldn't put lata in the tray: {err}"),
        }
    }

    pub fn remove(cx: &mut App) {
        cx.set_global(Held(None));
    }

    fn build() -> Result<TrayIcon, String> {
        let png = image::load_from_memory_with_format(PNG, image::ImageFormat::Png)
            .map_err(|err| err.to_string())?
            .into_rgba8();
        let (width, height) = png.dimensions();
        let icon = Icon::from_rgba(png.into_raw(), width, height).map_err(|err| err.to_string())?;
        let show = MenuItem::with_id(SHOW, "Show lata", true, None);
        let quit = MenuItem::with_id(QUIT, "Quit", true, None);
        let menu = Menu::with_items(&[&show, &PredefinedMenuItem::separator(), &quit])
            .map_err(|err| err.to_string())?;
        // These run on the thread that pumps the window messages, which is lata's own, and only
        // pass the click on: the app handles it where it has its windows.
        MenuEvent::set_event_handler(Some(|event: MenuEvent| match event.id.0.as_str() {
            SHOW => send(Incoming::Show),
            QUIT => send(Incoming::Quit),
            _ => {}
        }));
        // A click on the icon itself shows the window on Windows; on macOS it opens the menu.
        #[cfg(windows)]
        TrayIconEvent::set_event_handler(Some(|event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                send(Incoming::Show);
            }
        }));
        TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("lata")
            .with_menu(Box::new(menu))
            // On Windows the right button opens the menu and the left one shows the window; a
            // macOS menu bar item opens its menu on any click.
            .with_menu_on_left_click(cfg!(target_os = "macos"))
            .build()
            .map_err(|err| err.to_string())
    }

    /// Hides the window, which stays; `Window::activate_window` shows it again
    /// (vendor/gpui-pre-windows).
    #[cfg(windows)]
    pub fn hide(window: &Window, _: &mut App) -> bool {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            gpui_windows::hide_window(win.hwnd.get());
        }
        false
    }

    #[cfg(windows)]
    pub fn unhide(_: &mut App) {}

    /// Hides lata the way Cmd+H does: the window stays, the app steps back.
    #[cfg(target_os = "macos")]
    pub fn hide(_: &Window, cx: &mut App) -> bool {
        cx.hide();
        false
    }

    /// GPUI can hide the app but not unhide it; AppKit's `unhide:` also makes it active.
    #[cfg(target_os = "macos")]
    pub fn unhide(cx: &mut App) {
        if let Some(main) = objc2::MainThreadMarker::new() {
            objc2_app_kit::NSApplication::sharedApplication(main).unhide(None);
        }
        // Also when it wasn't hidden, only behind other apps.
        cx.activate(true);
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use gpui_kit::*;
    use ksni::TrayMethods;
    use ksni::menu::StandardItem;

    use super::send;
    use crate::link::Incoming;

    const PNGS: [&[u8]; 2] = [
        include_bytes!("../../../../assets/icons/png/lata-32.png"),
        include_bytes!("../../../../assets/icons/png/lata-64.png"),
    ];
    /// What shows StatusNotifierItems. GNOME has none unless an extension adds it.
    const WATCHER: &str = "org.kde.StatusNotifierWatcher";

    /// Keeps the icon: it goes away when this is shut down.
    struct Held(Option<ksni::Handle<Lata>>);

    impl Global for Held {}

    pub fn supported() -> bool {
        crate::runtime::block_on(async {
            let bus = zbus::Connection::session().await.ok()?;
            let dbus = zbus::fdo::DBusProxy::new(&bus).await.ok()?;
            dbus.name_has_owner(WATCHER.try_into().ok()?).await.ok()
        })
        .unwrap_or(false)
    }

    pub fn has_icon(cx: &App) -> bool {
        cx.try_global::<Held>().is_some_and(|held| held.0.is_some())
    }

    pub fn create(cx: &mut App) {
        // Fails when nothing would show the icon, and then the window doesn't go to the tray.
        match crate::runtime::block_on(Lata.spawn()) {
            Ok(handle) => cx.set_global(Held(Some(handle))),
            Err(err) => eprintln!("couldn't put lata in the tray: {err}"),
        }
    }

    pub fn remove(cx: &mut App) {
        if let Some(Held(Some(handle))) = cx.try_global::<Held>() {
            // Asks the service to stop; nothing to wait for.
            drop(handle.shutdown());
        }
        cx.set_global(Held(None));
    }

    /// Wayland can't hide a window, so it closes; lata keeps running (it quits only when told
    /// to now) and so does its state, which `main` holds on to. Show opens a new window on it.
    pub fn hide(_: &Window, cx: &mut App) -> bool {
        cx.set_quit_mode(QuitMode::Explicit);
        true
    }

    pub fn unhide(_: &mut App) {}

    struct Lata;

    impl ksni::Tray for Lata {
        fn id(&self) -> String {
            "lata".into()
        }

        fn title(&self) -> String {
            "lata".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            PNGS.iter().filter_map(|png| icon(png)).collect()
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "lata".into(),
                ..Default::default()
            }
        }

        // The clicks arrive on the tokio runtime and are only passed on: the app handles them
        // where it has its windows.
        fn activate(&mut self, _x: i32, _y: i32) {
            send(Incoming::Show);
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            vec![
                StandardItem {
                    label: "Show lata".into(),
                    activate: Box::new(|_: &mut Self| send(Incoming::Show)),
                    ..Default::default()
                }
                .into(),
                ksni::MenuItem::Separator,
                StandardItem {
                    label: "Quit".into(),
                    activate: Box::new(|_: &mut Self| send(Incoming::Quit)),
                    ..Default::default()
                }
                .into(),
            ]
        }

        /// The bar that showed the icon went away: the window comes back, so lata isn't left
        /// running with no way to reach it. The icon returns with the bar.
        fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
            send(Incoming::Show);
            true
        }
    }

    /// A PNG the way StatusNotifierItem wants it: ARGB, big-endian.
    fn icon(png: &[u8]) -> Option<ksni::Icon> {
        let png = image::load_from_memory_with_format(png, image::ImageFormat::Png)
            .ok()?
            .into_rgba8();
        let (width, height) = png.dimensions();
        let mut data = png.into_raw();
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1);
        }
        Some(ksni::Icon {
            width: width as i32,
            height: height as i32,
            data,
        })
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn icons_are_argb() {
            for png in super::PNGS {
                let icon = super::icon(png).expect("icon must decode");
                let rgba = image::load_from_memory(png).unwrap().into_rgba8();
                let (x, y) = (rgba.width() / 2, rgba.height() / 2);
                let [r, g, b, a] = rgba.get_pixel(x, y).0;
                let at = ((y * rgba.width() + x) * 4) as usize;
                assert_eq!(icon.data[at..at + 4], [a, r, g, b]);
            }
        }
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod imp {
    use gpui_kit::*;

    pub fn supported() -> bool {
        false
    }

    pub fn has_icon(_: &App) -> bool {
        false
    }

    pub fn create(_: &mut App) {}

    pub fn remove(_: &mut App) {}

    pub fn hide(_: &Window, _: &mut App) -> bool {
        true
    }

    pub fn unhide(_: &mut App) {}
}
