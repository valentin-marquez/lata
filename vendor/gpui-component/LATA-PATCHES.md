# lata patches to gpui-component 0.9.1

Copied from crates.io and wired in with `[patch.crates-io]` in the workspace `Cargo.toml`.
Every change is marked with a `// lata:` comment. Keep this list current, and redo the
changes on a fresh copy when upgrading.

- `src/popover.rs`: dropdowns open in 200 ms on ease-out quint (transitions.dev "Menu
  dropdown" curve, `cubic-bezier(0.22, 1, 0.36, 1)`), instead of 150 ms ease-out cubic.
- `src/popover.rs`: every `Popover` (so every `dropdown_menu`) fades and slides in like
  `dropdown_popup` does; upstream only animated selects, comboboxes and date pickers, so
  button menus appeared at once.
- `src/menu/context_menu.rs`: context menus fade in rising 6px over 200 ms on ease-out quint,
  like the popovers above; upstream showed them at once.
- `src/notification.rs`: toasts enter and leave on Modrinth's timing (250 ms in, 200 ms out,
  56 px of travel, ease-out quint) and the stack reflows on the same 250 ms; upstream is
  Sonner's 400 ms over 96 px.
- `src/dialog/dialog.rs`: the dialog's close button shows the pointer cursor.
- `src/notification.rs`: a toast is a compact pill that hugs its text (status icon, one line;
  title plus muted detail when it has both), centered in the stack, dismissed by a click; the
  stack is a list of the newest three instead of Sonner's pile. Upstream is a fixed-width card
  with a hover close button.
- `src/notification.rs`: one-line toasts hide after 3 s, those with a title and detail after 5 s
  (upstream: 5 s for all).
- `src/menu/popup_menu.rs`: menu rows are 34 px tall with 0.75rem side padding and the menu is
  at least 12rem wide (upstream: 26 px, 0.5rem, 8rem).
- `src/menu/popup_menu.rs`: menu items show the pointer cursor.
- `src/menu/popup_menu.rs`: `PopupMenu::scroll_to_checked(bool)`. A scrollable menu opens with
  its checked item first in view (kept as the default); a menu of several groups with headings
  (the Workshop's filters) turns it off so it opens at the top and its headings stay visible.
- `src/tab/tab.rs`, `src/tab/tab_bar.rs`: pill tabs and their sliding fill use the theme radius
  less 4 px (they sit in a tray with the inputs' radius) instead of being fully round.
- `src/menu/dropdown_menu.rs`: `DropdownMenuPopover::trigger_style`, forwarded to the
  popover's own, so a `dropdown_menu` trigger can fill its row like a select.
- `src/root.rs`: the toast layer paints deferred at priority 300, above dialogs (10 and up),
  menus (100) and tooltips (200): a toast is always on top, even one raised from a dialog.
- `src/notification.rs`: toast layout redone: 18 px icon centered on the text column, 13/18 muted
  detail, 12/14 px padding, a low-alpha foreground border, radius 14 px for two lines. `action`
  takes any element (upstream: a `Button`), vertically centered and never shrinking. The newest
  three toasts are one column inside a single `ToastStack` item with 8 px between them by real
  height (the stack's own layout assumed fixed-height toasts); the stack still springs its
  height and tracks hover. Width is capped at the window less 32 px.
- `src/theme/mod.rs`, `src/root.rs`: `Theme::font_fallbacks`, set on the window root next to
  `font_family`, so characters the main font lacks (CJK for Inter) come from the families the
  app names instead of whatever the OS fallback picks.
- `src/searchable_list/delegate.rs`, `item.rs`, `adapter.rs`: `SearchableListItem::tracks_hover`
  and `on_hover`, and `SearchableListItemElement::on_hover`. An item that tracks the mouse hears
  it enter and leave its whole row, padding included; an item's own `render` can only listen over
  what it draws, which is as wide as its text. The list's keyboard cursor counts too: moving it
  onto an item that tracks hover calls `on_hover(true)` and off it `on_hover(false)`. lata's
  character editor previews a hair style under the mouse or the cursor with it.
