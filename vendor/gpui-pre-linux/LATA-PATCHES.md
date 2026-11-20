# lata patches to gpui-pre-linux 0.3.12

Copied from crates.io and wired in with `[patch.crates-io]` in the workspace `Cargo.toml`.
Every change is marked with a `// lata:` comment. Keep this list current, and redo the
changes on a fresh copy when upgrading.

- `src/linux/x11/client.rs`: `write_to_clipboard` writes an image entry with the clipboard's
  existing `set_image`; upstream wrote only the text, so a copied picture pasted as nothing.
- `src/linux/wayland/client.rs`, `src/linux/wayland/clipboard.rs`: a copied image is offered
  under its own MIME type and its bytes are sent when that type is asked for; upstream offered
  and sent only text.
- `src/linux/wayland/window.rs`: a window that isn't resizable and has a min size gets that
  size as its max size too, instead of the GPU's texture size. Tiling compositors float a
  fixed-size toplevel, so the update splash shows as a centered card instead of being tiled.
