# lata patches to gpui-pre-windows 0.3.12

Copied from crates.io and wired in with `[patch.crates-io]` in the workspace `Cargo.toml`.
Every change is marked with a `// lata:` comment. Keep this list current, and redo the
changes on a fresh copy when upgrading.

- `src/gpui_windows.rs`: `hide_window(hwnd)`. GPUI has no way to hide a window (it only goes away
  when dropped), and lata keeps running with its window hidden while it sits in the system tray.
  lata gets the handle from `raw-window-handle`.
- `src/window.rs`: `activate` shows a window that is hidden before restoring and focusing it
  (upstream only restored a minimized one), so `Window::activate_window` brings it back from the
  tray.
