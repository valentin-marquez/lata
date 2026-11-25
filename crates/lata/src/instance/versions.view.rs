//! "Change version" dialog: every version the author published for a Workshop item, as Steam
//! lists them, to go back to one when the newest breaks a save or another mod (or forward
//! again). Steam keeps serving old versions, so nothing is kept on disk for this. Versions made
//! for another game than the instance runs are put aside as they are found out ([`fit`]).

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::fit::{self, Fit};
use super::mods::Version;
use super::{APP, Instance};
use crate::shell::Shell;
use crate::ui::{Empty, Loader};
use steam::workshop::Change;

/// Opens the dialog for Workshop item `item` of `instance`, installed at `current`.
pub fn open(
    instance: Instance,
    (item, title, current): (u64, SharedString, Option<u64>),
    shell: WeakEntity<Shell>,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|cx| Versions::new(instance, item, current, shell, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(title.clone())
            .centered()
            .w(px(520.))
            .child(view.clone())
    });
}

enum History {
    Loading,
    Failed,
    Ready(Vec<Change>),
}

struct Versions {
    instance: Instance,
    item: u64,
    /// The manifest installed now.
    current: Option<u64>,
    shell: WeakEntity<Shell>,
    history: History,
    /// The instance's game version, when known; without it nothing is put aside.
    game: Option<Version>,
    /// What is found out about each version, by manifest.
    fits: HashMap<u64, Fit>,
    /// Still asking Steam about the versions.
    checking: bool,
    /// The versions for other games are shown too, greyed out.
    show_all: bool,
    /// The background work asking about the versions, stopped when the dialog closes.
    check: Option<tokio::task::AbortHandle>,
}

impl Drop for Versions {
    fn drop(&mut self) {
        if let Some(check) = &self.check {
            check.abort();
        }
    }
}

impl Versions {
    fn new(
        instance: Instance,
        item: u64,
        current: Option<u64>,
        shell: WeakEntity<Shell>,
        cx: &mut Context<Self>,
    ) -> Self {
        let asked = crate::runtime::spawn(async move {
            let http = steam::http();
            let mut client = steam::Client::anonymous(&http).await?;
            steam::workshop::history(&mut client, item).await
        });
        cx.spawn(async move |this, cx| {
            let history = match asked.await {
                Ok(Ok(changes)) => History::Ready(changes),
                _ => History::Failed,
            };
            let _ = this.update(cx, |this, cx| {
                if let History::Ready(changes) = &history {
                    this.check(changes.iter().map(|c| c.manifest).collect(), cx);
                }
                this.history = history;
                cx.notify();
            });
        })
        .detach();
        let game = instance.version().as_deref().and_then(Version::parse);
        Self {
            instance,
            item,
            current,
            shell,
            history: History::Loading,
            game,
            fits: HashMap::new(),
            checking: false,
            show_all: false,
            check: None,
        }
    }

    /// Finds out, newest first, which of the versions `manifests` run on the instance's game.
    /// Each answer shows as it comes; one Steam won't give doesn't stop the rest.
    fn check(&mut self, manifests: Vec<u64>, cx: &mut Context<Self>) {
        let Some(game) = self.game else {
            return;
        };
        let item = self.item;
        let (found, mut answers) = tokio::sync::mpsc::unbounded_channel();
        let work = crate::runtime::spawn(async move {
            let http = steam::http();
            let mut client = steam::Client::anonymous(&http).await?;
            let Some(details) = steam::workshop::details(&mut client, &[item])
                .await?
                .into_iter()
                .next()
            else {
                return Ok(());
            };
            let source = steam::workshop::source(&mut client, APP).await?;
            for manifest in manifests {
                if let Ok(infos) = fit::infos(&mut client, &http, &source, &details, manifest).await
                    && found.send((manifest, infos)).is_err()
                {
                    break;
                }
            }
            Ok::<_, steam::Error>(())
        });
        self.check = Some(work.abort_handle());
        self.checking = true;
        cx.spawn(async move |this, cx| {
            // Ends when the work does: it holds the only sender.
            while let Some((manifest, infos)) = answers.recv().await {
                let verdict = fit::fit(&infos, game);
                let alive = this.update(cx, |this, cx| {
                    this.fits.insert(manifest, verdict);
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.checking = false;
                cx.notify();
            });
        })
        .detach();
    }

    /// What `manifest` is made for when it isn't this game: the tag that says so.
    fn other_game(&self, manifest: u64) -> Option<String> {
        self.fits.get(&manifest).and_then(Fit::label)
    }

    /// Downloads `manifest` in place of the installed version. Anything but the newest stays
    /// put when updates come, until the newest is picked again.
    fn pick(&self, manifest: u64, newest: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (instance, shell, handle) = (
            self.instance.clone(),
            self.shell.clone(),
            window.window_handle(),
        );
        let exact = vec![(self.item, manifest)];
        window.close_dialog(cx);
        // After this update ends: the shell repaints views as the download starts.
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.mod_versions(instance, exact, !newest, handle, cx)
            });
        });
    }

    fn row(&self, ix: usize, change: &Change, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let installed = self.current == Some(change.manifest);
        let (manifest, newest) = (change.manifest, ix == 0);
        // Made for another game: shown only when asked, greyed out and not picked.
        let other = self.other_game(manifest).filter(|_| !installed);
        let (day, hour) = when(change.date, &chrono::Local);
        // The author's note, when they left one: its first line says what changed.
        let note = change.note.lines().next().unwrap_or_default().to_owned();
        let tag = |text: SharedString, strong: bool| {
            div()
                .flex_none()
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .when(strong, |this| {
                    this.bg(theme.primary.opacity(0.25))
                        .text_color(theme.foreground)
                })
                .when(!strong, |this| {
                    this.bg(theme.muted).text_color(theme.muted_foreground)
                })
                .child(text)
        };
        div()
            .id(("version", ix))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .when(ix > 0, |this| this.border_t_1().border_color(theme.border))
            .when(other.is_some(), |this| this.opacity(0.5))
            .when(!installed && other.is_none(), |this| {
                this.cursor_pointer()
                    .hover(|style| style.bg(theme.muted))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick(manifest, newest, window, cx)
                    }))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(day))
                            // Authors often publish several in a day: the hour tells them apart.
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(hour),
                            ),
                    )
                    .when(!note.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(note),
                        )
                    }),
            )
            .when(newest, |this| this.child(tag(t!("Newest").into(), false)))
            .when(installed, |this| {
                this.child(tag(t!("Installed").into(), true))
            })
            .when_some(other.clone(), |this, other| {
                this.child(tag(other.into(), false))
            })
            .when(!installed && other.is_none(), |this| {
                this.child(
                    Icon::new(IconName::Download)
                        .small()
                        .text_color(theme.muted_foreground),
                )
            })
            .into_any_element()
    }
}

/// The day and the hour of a Unix time in `zone` ("Nov 9, 2026", "11:04"): the computer's own,
/// so a version reads like the clock on the wall. Both from the same zone, so near midnight
/// the day matches the hour.
fn when<Tz: chrono::TimeZone>(unix: u64, zone: &Tz) -> (String, String)
where
    Tz::Offset: std::fmt::Display,
{
    let at = chrono::DateTime::from_timestamp(unix as i64, 0)
        .unwrap_or_default()
        .with_timezone(zone);
    (
        at.format("%b %-d, %Y").to_string(),
        at.format("%H:%M").to_string(),
    )
}

impl Render for Versions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rows first: they borrow `cx` for their listeners, the theme only reads it.
        // Put aside: made for another game, unless it is the one installed.
        let aside = |this: &Self, change: &Change| {
            this.current != Some(change.manifest) && this.other_game(change.manifest).is_some()
        };
        let (rows, hidden): (Vec<AnyElement>, usize) = match &self.history {
            History::Ready(changes) => (
                changes
                    .iter()
                    .enumerate()
                    .filter(|(_, change)| self.show_all || !aside(self, change))
                    .map(|(ix, change)| self.row(ix, change, cx))
                    .collect(),
                changes.iter().filter(|change| aside(self, change)).count(),
            ),
            _ => (Vec::new(), 0),
        };
        // Always there once the list is, so the dialog doesn't jump when the check ends: what
        // it is doing, then what it found.
        let total = match &self.history {
            History::Ready(changes) => changes.len(),
            _ => 0,
        };
        let status = self.game.filter(|_| total > 0).map(|v| {
            let game = format!("{}.{}", v.build, v.major);
            match (self.checking, total - hidden) {
                (true, _) => t!("Checking which versions run on {game}", game = game),
                (false, fit) if fit == total && total == 1 => t!("It runs on {game}", game = game),
                (false, fit) if fit == total => t!("All of them run on {game}", game = game),
                (false, fit) => t!(
                    "{fit} of {total} run on {game}",
                    fit = fit,
                    total = total,
                    game = game
                ),
            }
        });
        let toggle = (hidden > 0).then(|| {
            let label = match (self.show_all, hidden) {
                (true, _) => t!("Hide versions for other games").to_owned(),
                (false, n) => tn!(
                    n,
                    "Show {n} for another game version",
                    "Show {n} for other game versions"
                ),
            };
            Button::new("other-versions")
                .ghost()
                .small()
                .cursor_pointer()
                .label(label)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_all = !this.show_all;
                    cx.notify();
                }))
        });
        let theme = cx.theme();
        let body = match &self.history {
            History::Loading => div().py_6().child(Loader).into_any_element(),
            History::Failed => Empty::new(
                IconName::CircleX,
                t!("Steam didn't answer"),
                t!("Close this and try again in a bit."),
            )
            .into_any_element(),
            History::Ready(changes) if changes.is_empty() => Empty::new(
                IconName::Inbox,
                t!("No versions to show"),
                t!("Steam has no history for this one."),
            )
            .into_any_element(),
            History::Ready(_) => div()
                .id("versions")
                .flex()
                .flex_col()
                .max_h(px(360.))
                .overflow_y_scroll()
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .children(rows)
                .into_any_element(),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_sm().text_color(theme.muted_foreground).child(t!(
                "Pick the version to play with. An older one stays put when updates come, \
                 until you pick the newest again."
            )))
            .child(body)
            .when_some(status, |this, status| {
                this.child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .h(px(28.))
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(status)
                        .children(toggle),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tells_the_day_and_hour_where_the_player_is() {
        let zone = |hours: i32| chrono::FixedOffset::east_opt(hours * 3600).unwrap();
        // 2026-11-09 14:04 UTC.
        let at = 1_794_233_055;
        assert_eq!(
            super::when(at, &zone(0)),
            ("Nov 9, 2026".into(), "14:04".into())
        );
        assert_eq!(
            super::when(at, &zone(-3)),
            ("Nov 9, 2026".into(), "11:04".into())
        );
        // Past midnight there, the day moves with the hour.
        assert_eq!(
            super::when(at, &zone(11)),
            ("Nov 10, 2026".into(), "01:04".into())
        );
    }
}
