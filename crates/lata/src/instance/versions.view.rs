//! "Change version" dialog: every version the author published for a Workshop item, as Steam
//! lists them, to go back to one when the newest breaks a save or another mod (or forward
//! again). Steam keeps serving old versions, so nothing is kept on disk for this.

use gpui_kit::assets::IconName;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Instance, branches};
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
                this.history = history;
                cx.notify();
            });
        })
        .detach();
        Self {
            instance,
            item,
            current,
            shell,
            history: History::Loading,
        }
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
        // The author's note, when they left one: its first line says what changed.
        let note = change.note.lines().next().unwrap_or_default().to_owned();
        let tag = |text: &'static str, strong: bool| {
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
            .when(!installed, |this| {
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
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(branches::date(change.date)),
                            )
                            // Authors often publish several in a day: the hour tells them apart.
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(clock(change.date)),
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
            .when(newest, |this| this.child(tag("Newest", false)))
            .when(installed, |this| this.child(tag("Installed", true)))
            .when(!installed, |this| {
                this.child(
                    Icon::new(IconName::Download)
                        .small()
                        .text_color(theme.muted_foreground),
                )
            })
            .into_any_element()
    }
}

/// "14:05 UTC" for a Unix time.
fn clock(unix: u64) -> String {
    format!("{:02}:{:02} UTC", unix / 3600 % 24, unix / 60 % 60)
}

impl Render for Versions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Rows first: they borrow `cx` for their listeners, the theme only reads it.
        let rows: Vec<AnyElement> = match &self.history {
            History::Ready(changes) => changes
                .iter()
                .enumerate()
                .map(|(ix, change)| self.row(ix, change, cx))
                .collect(),
            _ => Vec::new(),
        };
        let theme = cx.theme();
        let body = match &self.history {
            History::Loading => div().py_6().child(Loader).into_any_element(),
            History::Failed => Empty::new(
                IconName::CircleX,
                "Steam didn't answer",
                "Close this and try again in a bit.",
            )
            .into_any_element(),
            History::Ready(changes) if changes.is_empty() => Empty::new(
                IconName::Inbox,
                "No versions to show",
                "Steam has no history for this one.",
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
            .child(div().text_sm().text_color(theme.muted_foreground).child(
                "Pick the version to play with. An older one stays put when updates come, \
                 until you pick the newest again.",
            ))
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn tells_the_hour_in_utc() {
        assert_eq!(super::clock(1_794_233_055), "14:04 UTC");
    }
}
