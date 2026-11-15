# lata

Project Zomboid instance manager and launcher, written from scratch in Rust.
The app is named `lata` (renamed from the working name `lulo` on 2026-11-14, everywhere: crate,
binary, `lata://` links, `LATA_*` env vars, keyring, data dir; `instance::store::adopt_old_name`
moves an old `lulo` data folder once). Shared instance packs use
the `.lata` extension (a canned instance; not built yet). It replaces
`github.com/valentin-marquez/KnoxKit`.
Nothing is ported from KnoxKit: it is prior art for intent only.

## Names

The ecosystem is "lata" plus a word per piece (decided 2026-11-15): **lata launcher** is the app
(the title bar shows "lata" in the wordmark and "launcher" in muted Inter), **lata link** is the
browser extension in `extension/` (plain JS and CSS, no build step). The binary, crate, `lata://`
links and `LATA_*` variables stay just `lata`. lata link lives in the store listings that were
KnoxLink (Firefox slug `lata-link`, Chrome item `fopkfokkcmmhgjjdlbjjmdgkffddjbpm`); its texts
are in `extension/STORE.md` and it is released with `extension-v<version>` tags
(`.github/workflows/extension.yml`), apart from the launcher's `v*` tags.

Screenshots for stores and docs come from the demo library in `research/demo/home`
(`LATA_HOME=research/demo/home cargo run -p lata`), 1280x800, kept in `research/demo/shots`.

## Goals

- Native on Linux, Windows and macOS. Linux is a first-class target, not an afterthought
  (the old Tauri version failed there).
- Isolated instances: each one owns its game build (Steam branch), mods and config.
- Talk to Steam natively (CM over WebSocket, PICS, CDN, Workshop). No SteamCMD dependency.

## Stack

- Rust, edition 2024, Cargo workspace under `crates/`:
  - `crates/lata`: the app (GPUI).
  - `crates/steam`: native Steam client, no UI. tokio + tokio-tungstenite (CM WebSocket) +
    reqwest (web API, CDN). TLS is rustls with `ring` (no C toolchain needed on any OS); call
    `steam::http()` for the HTTP client, it installs the provider. Message types are written by hand in
    `crates/steam/src/proto.rs` (prost derives) with only the fields we use; take field numbers
    from the local research protos. Never commit Valve's `.proto` files or descriptor sets.
  - Live network tests are `#[ignore]`: `cargo test -p steam -- --ignored`.
    `cargo run -p steam --example branches` prints PZ's branches from Steam.
- UI: `gpui-kit` (GPUI + gpui-component). Prelude is `use gpui_kit::*;`,
  styling helpers like `v_flex` need `use gpui_kit::base::StyledExt;`.
- If GPUI or gpui-component can't do something, patch it (`[patch.crates-io]` with a vendored or
  forked copy) instead of working around it in app code.
- Visual identity is decided by the maintainer from references. Don't invent a style beyond them.

## Design references

- Layout: instance list on the left + selected instance on the right (Play, Mods/Worlds/Logs),
  from Playnite/Heroic/Xbox references. Modrinth stays a reference for details only.
- Interaction details: Modrinth (`modrinth.com/discover/mods`, project page, and the Modrinth App).
  App shell = left icon rail, top bar with running-instance status, home with "Play" (recent
  instances) and "Library" (instance grid, search, sort, filters). Content browser = filter sidebar
  (game version, category), search, sort, pagination, result cards with icon/title/author/tags/stats
  and an install action. Project page = header + tabs (description, changelog, versions) + side
  panel (compatibility, links, tags).
- Colors: Modrinth's structure (neutral surfaces, one strong accent). The accent is a grey-blue
  taken from the game's key art (decided 2026-11-13; it replaced the logo's orange):
  - accent `#447189` on light (hover `#3b6379`, pressed `#325567`) and `#4f86a3` on dark (hover
    `#5f97b4`); focus ring `#447189` light, `#629db8` dark. Info uses the accent too.
  - status colors, still from the logo: amber `#FAB202` (warning), deep red `#DE290F` (danger),
    lime (success). Amber is a fill color: warning text on a light surface is dark orange
    `#c2410c` instead (`ui::warning_text`; decided 2026-11-14, amber text didn't read).
  - the logo is two-tone "Key art": body `#447189`, face `#e6edf1` (decided 2026-11-14, from
    variants in Figma; more than two colors was rejected). Besides warning text, don't bring
    orange back into the UI.
  - neutrals: zinc (light `#fafafa`/`#f4f4f5`/`#ffffff`, dark `#09090b`/`#111113`/`#1c1c1f`).
    No warm/stone greys: they read as sand. Keep the accent to accents (Play, selection,
    status) rather than large areas.
- Themes: light and dark, following the system appearance.
- Primary button: AlignUI FancyButton (`alignui.com/docs/v1.2/ui/fancy-button#primary`):
  accent fill, 1px accent ring + `0 1px 2px rgba(14,18,27,.24)` shadow, top-to-bottom white gloss
  overlay at 16% opacity (24% on hover), 1px inner top highlight white 12% -> transparent,
  heights 40/36/32 px, radius 10/8/8 px, 20 px icon, 200 ms ease-out.
- Font: Inter, bundled in `assets/fonts/` (Regular, Medium, SemiBold, Bold) and set as `font.family`
  in the theme. Only use those four weights; others fall back to whatever the OS picks.
- Motion: springs, never linear timing (Emil Kowalski's approach: physical, interruptible).
  Use `gpui_kit::base::spring` with `ui::SPRING`; keep damping at 1.0 for anything bounded.
  Only indeterminate loops (the loader) use a repeating eased `Animation`.
- Reactivity rules, from a study of modrinth.com (details, sources and measured values in
  `research/ui/MOTION.md`). GPUI has no CSS filters and can't scale or translate boxes (only
  images and SVG icons), so Modrinth's `brightness()` becomes a precomputed color and its
  `scale(.97)` press becomes an insetting face layer. Springs, damping 1.0.
  - Hover: bg to its hover color (bg x0.9 in light, x1.25 in dark), text unchanged, ~150 ms.
    No lift, no shadow and no border change on cards. Helpers: `ui::hovered`, `ui::mix`,
    `ui::SPRING_FAST` (150 ms).
  - Press: shrink like Modrinth (buttons .97, cards .98, chips .95) by drawing fill, border
    and shadow on an absolute "face" layer that insets by that fraction per side while the
    content stays (see `FancyButton` and the Library card); gloss and drop shadow fade with
    it. `ui::SPRING_PRESS` (100 ms). Icon-only buttons scale the SVG icon instead.
  - Focus (inputs and keyboard focus on anything): one 4px accent ring at 40 to 70% alpha,
    box-shadow spread 0 to 4px in ~150 ms. Border and bg stay; never two focus indicators.
  - Selected (filters, sidebar, current page): accent fill at 25% alpha (15% in dark if loud),
    full-contrast text; hovering a selected item raises the alpha, never turns grey.
  - Rows (logs, mods, versions): hover bg; trailing actions fade 0 to 1. Menu items: hover
    bg instant or 80 ms, keyboard focus looks the same as hover.
  - Tabs: sliding indicator; leading edge arrives first (two springs, ~120 ms and 300 ms).
  - Switch: knob slides 24 px with the track color in 200 ms, grows 16 to 18 px on hover,
    squashes to 14 px on press. Checkbox: state instant, check icon scales .6 to 1.
  - Dropdown/popover: fade plus 8px slide from the trigger, 200 ms ease-out quint (patched
    in `vendor/gpui-component`); exit fade plus 4px back in ~120 ms (not done yet); chevron
    rotates 180 degrees.
  - Tooltip: ~300 ms show delay, 100 ms in, 80 ms out, no delay between adjacent ones.
    Modal: open dialogs with `.entrance(DialogEntrance::FadeSlide)` (fade while settling from
    just above, and back out); the default drops from the window top.
    Toast: slides in by its height with a fade, siblings spring to new places, 250 ms.
  - One-shot entrances (a step, a panel, a new picture): keyed `Animation` with `ui::ENTER`
    (200 ms) and `ui::ease_out_quint`, fade plus a few px of travel; the key changes to replay.
  - Disabled: opacity .5, no hover or press, arrow cursor. Reduced motion: springs snap to
    target and loops stop; color changes stay.
- Everything clickable gets `.cursor_pointer()`, including gpui-component `Button`s (their
  default is the arrow cursor; our style is applied last, so adding it works).
- UI text has no separator glyphs: no middle dots, en/em dashes or ellipsis characters. Show
  metadata as separate items (`ui::Meta`: icon + text) or put the count on its own.
- Loading: `ui::Loader`, the accent bar under the title bar, while a page's data loads.
- Logo: a tin can with a two-ring lid ("parecía switch" with the old oval tab; decided
  2026-11-14). Vector sources in `assets/icons/svg/` (`mark`, `app-dark`, `app-accent`,
  `wordmark-dark`, `wordmark-light`). The OS icon is `app-accent`: `png/lata-*.png` (16-1024),
  `lata.ico`, `lata.icns`, rendered from the SVG with `rsvg-convert` (ico: ImageMagick, icns:
  Pillow). The title bar shows `png/mark-64.png`. The wordmark is Outfit Bold
  (`assets/fonts/Outfit-Bold.ttf`, `theme::WORDMARK`), only for the name; UI text stays Inter.
- Instance pictures: game item icons on Modrinth-style gradients (`instance::BACKGROUNDS`), or a
  picture the user uploads. A themed set of PZ item icons is vendored in
  `assets/instance-icons/` and listed in `game::THEMES` (one macro embeds them). The Indie Stone's
  terms allow distributing parts of the base files while the game isn't playable
  (`research/indiestone/POLICY.md`); keep it a small set and never add Spiffo or other official
  branding. Regenerate with the ignored test `game::tests::vendors_the_item_icons`.

## Commands

```sh
cargo run -p lata
LATA_THEME=light cargo run -p lata   # pin light/dark regardless of the OS setting
watchexec -r -e rs,json -- cargo run -p lata   # rebuild and relaunch on save
cargo fmt
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

A change is done only when fmt, clippy (no warnings) and tests all pass. Run them; don't assume.

## Code conventions

### Layout: by feature, not by kind

Each feature is a folder (`home/`, `shell/`, `instance/`, later `workshop/`, `steam/`...). Shared UI
building blocks live in `ui/`, theming in `theme/`. No `utils/`, `helpers/`, `components/` buckets.

### File names: `name.kind.rs`

- Files are kebab-case with a kind suffix: `rail.view.rs`, `fancy-button.component.rs`,
  `lata.theme.json`. Kinds: `view` (a piece of a feature screen), `component` (reusable widget in
  `ui/`), `model` (data/state), `theme`. Add a kind only when it carries meaning.
- `mod.rs` holds the feature's own entry point and small items; Rust can't load dotted names on its
  own, so declare them with `#[path]`: `#[path = "rail.view.rs"] mod rail;`.

### Names: let the path speak

- Short, plain names; the module path carries the context. `workshop::find`, not
  `workshop::find_mod_pack_in_workshop`; `instance::samples()`, `theme::init(cx)`.
- Types are named for what they are (`Instance`, `Shell`, `Rail`), not prefixed with the feature.

### General

- Small crates with clear boundaries once there is a real second consumer; don't split early.
- No `unsafe` (forbidden at workspace level).
- Errors: `thiserror` in library crates, `anyhow` only at the binary edge.
- Logic that parses bytes or talks protocol gets tests against real captured data.
- Comments explain *why*, not *what*. English for code, identifiers and commit messages.

## Steam research

Local only (`research/` is in `.git/info/exclude`, never commit it): reverse-engineering notes,
reference scripts and the protos extracted from steamclient.so.

- Notes: `research/steam/FINDINGS.md` (English; read before touching Steam code). The research
  session keeps its own Spanish copy in `/mnt/datos/dev/juegos/steamcmd/re/HALLAZGOS.md`.
- Reference clients: `cm_client.py`, `verify_depot.py`, `auth_probe.py` (`python3 -I`).
- Protos: `research/steam/protos/` (field numbers for `proto.rs`).
- Fixtures: `/mnt/datos/dev/juegos/steamcmd/re/runs/` (`LATA_STEAMCMD_MOD` points the live test at
  the SteamCMD copy of mod 3832274571). Never copy anything from a `runs/` folder: it caches depot keys.
- Public, English explanation of how the client works: `docs/steam.md`.

## Git

- Remote: `origin` = `github.com/valentin-marquez/lata` (formerly lulo, before that KnoxKit), pushed directly. This
  repo is the exception to the global Forgejo-first rule; Forgejo's old `valentin/KnoxKit` is kept
  only as an archive of the previous project and no longer mirrors to GitHub.
- Conventional Commits (`feat:`, `fix:`, `refactor:`, `chore:`, `docs:`, `test:`).
- No `Co-Authored-By` trailers and no mentions of AI tools in commits, PRs or code.
- AI tooling files (`CLAUDE.md`, `.claude/`, `openspec/`, generated design docs) are listed in
  `.git/info/exclude`, not `.gitignore`. Never `git add -f` them.
