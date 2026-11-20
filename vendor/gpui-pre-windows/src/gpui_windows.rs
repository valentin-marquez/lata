#![cfg(target_os = "windows")]

mod clipboard;
mod destination_list;
mod dialog;
mod direct_manipulation;
mod direct_write;
mod directx_atlas;
mod directx_devices;
mod directx_renderer;
mod dispatcher;
mod display;
mod events;
mod keyboard;
mod platform;
mod system_notifications;
mod system_settings;
mod util;
mod vsync;
mod window;
mod wrapper;

pub(crate) use clipboard::*;
pub(crate) use destination_list::*;
pub(crate) use direct_write::*;
pub(crate) use directx_atlas::*;
pub(crate) use directx_devices::*;
pub(crate) use directx_renderer::*;
pub(crate) use dispatcher::*;
pub(crate) use display::*;
pub(crate) use events::*;
pub(crate) use keyboard::*;
pub(crate) use platform::*;
pub(crate) use system_notifications::*;
pub(crate) use system_settings::*;
pub(crate) use util::*;
pub(crate) use vsync::*;
pub(crate) use window::*;
pub(crate) use wrapper::*;

pub use platform::WindowsPlatform;

/// lata: hides the window with this handle (`raw-window-handle`'s `hwnd`). GPUI has no call
/// for it: a window only disappears when it is dropped, and lata keeps running with none
/// shown while it sits in the tray. `activate` shows it again.
pub fn hide_window(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, ShowWindowAsync};
    let _ = unsafe { ShowWindowAsync(windows::Win32::Foundation::HWND(hwnd as *mut _), SW_HIDE) };
}

pub(crate) use windows::Win32::Foundation::HWND;
