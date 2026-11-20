//! The system tray. With "Minimize to the system tray" on, closing the window hides lata to a
//! tray icon instead of quitting: its click or "Show lata" brings the window back, "Quit" ends
//! lata for real. On Windows only for now; elsewhere the setting stays off.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::*;
use tokio::sync::mpsc::UnboundedSender;

use crate::link::Incoming;

/// Whether lata can sit in the tray on this system.
pub const SUPPORTED: bool = cfg!(windows);

/// Where the tray's clicks go: the same channel as what reaches lata from outside.
static TO_APP: OnceLock<UnboundedSender<Incoming>> = OnceLock::new();
/// Set once lata is quitting, so closing the window then doesn't hide it again.
static QUITTING: AtomicBool = AtomicBool::new(false);

/// Gives the tray the channel its clicks are sent through.
pub fn connect(to_app: UnboundedSender<Incoming>) {
    let _ = TO_APP.set(to_app);
}

#[cfg(windows)]
fn send(message: Incoming) {
    if let Some(to) = TO_APP.get() {
        let _ = to.send(message);
    }
}

/// Whether the setting is on, here where it can work.
pub fn enabled() -> bool {
    SUPPORTED && crate::settings::load().minimize_to_tray
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

/// What the main window does when asked to close: `true` lets it close (lata quits), `false`
/// keeps lata running with the window hidden. It only hides when the icon is there, so the
/// window can't be lost with nothing to bring it back.
pub fn close(window: &mut Window, cx: &mut App) -> bool {
    if QUITTING.load(Ordering::SeqCst) || !enabled() || !imp::has_icon(cx) {
        return true;
    }
    imp::hide(window);
    false
}

/// Quits lata, whether or not its window is hidden.
pub fn quit(cx: &mut App) {
    QUITTING.store(true, Ordering::SeqCst);
    cx.quit();
}

#[cfg(windows)]
mod imp {
    use gpui_kit::*;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::send;
    use crate::link::Incoming;

    const PNG: &[u8] = include_bytes!("../../../../assets/icons/png/lata-32.png");
    const SHOW: &str = "tray-show";
    const QUIT: &str = "tray-quit";

    /// Keeps the icon alive: it goes away when this is dropped.
    struct Held(Option<TrayIcon>);

    impl Global for Held {}

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
            // The right button opens the menu; the left one shows the window.
            .with_menu_on_left_click(false)
            .build()
            .map_err(|err| err.to_string())
    }

    /// Hides the window; `Window::activate_window` shows it again (vendor/gpui-pre-windows).
    pub fn hide(window: &Window) {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};
        if let Ok(handle) = HasWindowHandle::window_handle(window)
            && let RawWindowHandle::Win32(win) = handle.as_raw()
        {
            gpui_windows::hide_window(win.hwnd.get());
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use gpui_kit::*;

    pub fn has_icon(_: &App) -> bool {
        false
    }

    pub fn create(_: &mut App) {}

    pub fn remove(_: &mut App) {}

    pub fn hide(_: &Window) {}
}
