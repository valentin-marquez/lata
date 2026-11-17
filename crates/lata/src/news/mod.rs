//! "What's new": the notes of each version, shown once after lata was updated and kept in
//! Settings. They ship inside the binary, so a build knows its own notes without a network.
//!
//! To add a version, put an entry at the top of `crates/lata/changelog.json`. An entry for a
//! version newer than the build stays hidden until the version in `Cargo.toml` reaches it.

use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;
use serde::Deserialize;

use crate::ui::FancyButton;
use crate::update::Version;

#[derive(Clone, Deserialize)]
struct Entry {
    version: String,
    /// Shown as written.
    date: String,
    #[serde(default)]
    added: Vec<String>,
    #[serde(default)]
    changed: Vec<String>,
    #[serde(default)]
    fixed: Vec<String>,
}

fn entries() -> Vec<Entry> {
    serde_json::from_str(include_str!("../../changelog.json")).unwrap_or_default()
}

/// The entries someone who last saw `seen` hasn't read, on a lata at `current`, newest
/// first. `None` is everything up to `current`.
fn unseen(entries: Vec<Entry>, seen: Option<&Version>, current: &Version) -> Vec<Entry> {
    let mut new: Vec<(Version, Entry)> = entries
        .into_iter()
        .filter_map(|entry| Some((Version::parse(&entry.version)?, entry)))
        .filter(|(version, _)| version <= current && seen.is_none_or(|seen| version > seen))
        .collect();
    new.sort_by(|a, b| b.0.cmp(&a.0));
    new.into_iter().map(|(_, entry)| entry).collect()
}

fn current() -> Option<Version> {
    Version::parse(env!("CARGO_PKG_VERSION"))
}

/// On start: shows what changed since the lata that last ran here, once. A first start has
/// nothing to compare with, so it only remembers this version.
pub fn start(window: &mut Window, cx: &mut App) {
    let seen = crate::settings::load().news_seen;
    if seen.as_deref() == Some(env!("CARGO_PKG_VERSION")) {
        return;
    }
    crate::settings::update(|saved| saved.news_seen = Some(env!("CARGO_PKG_VERSION").into()));
    let (Some(seen), Some(current)) = (seen.as_deref().and_then(Version::parse), current()) else {
        return;
    };
    show(unseen(entries(), Some(&seen), &current), window, cx);
}

/// Every version's notes up to this lata's, for Settings.
pub fn open(window: &mut Window, cx: &mut App) {
    if let Some(current) = current() {
        show(unseen(entries(), None, &current), window, cx);
    }
}

fn show(entries: Vec<Entry>, window: &mut Window, cx: &mut App) {
    if entries.is_empty() {
        return;
    }
    let view = cx.new(|_| News {
        entries,
        scroll: ScrollHandle::new(),
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title("What's new in lata")
            .centered()
            .w(px(WIDTH))
            .child(view.clone())
    });
}

const WIDTH: f32 = 560.;
/// The tallest the notes get before they scroll.
const TALL: f32 = 440.;
/// What the dialog has around the notes (title, button, paddings) plus air above and below
/// it, so the whole dialog fits a short window.
const AROUND: f32 = 260.;

struct News {
    entries: Vec<Entry>,
    scroll: ScrollHandle,
}

fn group(title: &'static str, lines: &[String], cx: &App) -> Option<Div> {
    let theme = cx.theme();
    let line = |text: &String| {
        div()
            .flex()
            .gap_2()
            .child(
                // Drawn, not typed: centred on the first line of text.
                div()
                    .flex_none()
                    .mt(px(8.))
                    .size(px(4.))
                    .rounded_full()
                    .bg(theme.muted_foreground),
            )
            .child(div().flex_1().min_w_0().text_sm().child(text.clone()))
    };
    (!lines.is_empty()).then(|| {
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child(title),
            )
            .children(lines.iter().map(line))
    })
}

fn entry(entry: &Entry, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::BOLD)
                        .child(entry.version.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(entry.date.clone()),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_4()
                .p_4()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .children(group("Added", &entry.added, cx))
                .children(group("Changed", &entry.changed, cx))
                .children(group("Fixed", &entry.fixed, cx)),
        )
}

impl Render for News {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let room = window.viewport_size().height - px(AROUND);
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .relative()
                    .child(
                        div()
                            .id("news")
                            .flex()
                            .flex_col()
                            .gap_5()
                            .max_h(room.min(px(TALL)))
                            // Room for the scrollbar, which draws over the right edge.
                            .pr_3()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .children(self.entries.iter().map(|e| entry(e, cx))),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
            .child(
                div().flex().justify_end().child(
                    FancyButton::new("news-close", "Got it")
                        .small()
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the gpui glob brings its own `test` attribute, which would shadow Rust's.
    use super::{Entry, Version, entries, unseen};

    fn versions(seen: Option<&str>, current: &str) -> Vec<String> {
        let entry = |version: &str| Entry {
            version: version.into(),
            date: String::new(),
            added: Vec::new(),
            changed: Vec::new(),
            fixed: Vec::new(),
        };
        let all = ["0.9.0", "0.10.0", "0.11.0", "0.9.1"].map(entry).to_vec();
        let seen = seen.map(|seen| Version::parse(seen).unwrap());
        unseen(all, seen.as_ref(), &Version::parse(current).unwrap())
            .into_iter()
            .map(|entry| entry.version)
            .collect()
    }

    #[test]
    fn shows_what_is_newer_than_seen_newest_first() {
        // By number, not by text: 0.10.0 is after 0.9.1. And 0.11.0 waits for its build.
        assert_eq!(versions(Some("0.9.0"), "0.10.0"), ["0.10.0", "0.9.1"]);
        assert!(versions(Some("0.10.0"), "0.10.0").is_empty());
        // Going back to an older lata shows nothing.
        assert!(versions(Some("0.11.0"), "0.9.1").is_empty());
        assert_eq!(versions(None, "0.10.0"), ["0.10.0", "0.9.1", "0.9.0"]);
    }

    #[test]
    fn embedded_changelog_parses() {
        let entries = entries();
        assert!(!entries.is_empty());
        for entry in &entries {
            assert!(
                Version::parse(&entry.version).is_some(),
                "{}",
                entry.version
            );
            assert!(!entry.date.is_empty());
            assert!(
                !(entry.added.is_empty() && entry.changed.is_empty() && entry.fixed.is_empty())
            );
        }
    }
}
