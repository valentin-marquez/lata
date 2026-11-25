//! The library location: where it is now, and picking a new place as drive tiles, like Windows'
//! "This PC". The move runs right here, and its dialog can't close until it is done, so nothing
//! launches or downloads into a library that is half copied.

use std::path::PathBuf;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, Icon as Glyph, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::drives::{self, Drive, Kind};
use crate::instance::install::size as bytes;
use crate::instance::store;
use crate::shell::Shell;
use crate::ui::{self, FancyButton, Meta};

/// About what a game build takes, for a first start on a nearly full drive.
const GAME: u64 = 6_000_000_000;
/// Less than this left after a move and game updates soon run out of room.
const LOW: u64 = 10_000_000_000;

pub struct Picker {
    shell: WeakEntity<Shell>,
    /// Where the library is now.
    current: PathBuf,
    drives: Vec<Drive>,
    /// Bytes and instances in the library; `None` while they are counted.
    size: Option<(u64, usize)>,
    /// Showing the drives instead of the current location.
    choosing: bool,
    /// The first start: nothing to move yet, and keeping the suggestion is fine.
    first: bool,
    /// The folder picked so far.
    target: Option<PathBuf>,
    /// Why `target` can't take the library.
    problem: Option<String>,
    /// Bytes copied and in all, while moving.
    moving: Option<(u64, u64)>,
}

#[derive(Clone, Copy)]
enum Tone {
    Plain,
    Warn,
    Bad,
}

impl Picker {
    pub fn new(shell: WeakEntity<Shell>, first: bool, cx: &mut Context<Self>) -> Self {
        let current = store::library();
        let counting = current.clone();
        cx.spawn(async move |this, cx| {
            let size = cx
                .background_spawn(async move { super::size(&counting) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.size = Some(size);
                cx.notify();
            });
        })
        .detach();
        // ponytail: drives are read once per dialog; plugging a USB stick in needs a reopen.
        let mut picker = Self {
            shell,
            drives: drives::list(),
            target: None,
            problem: None,
            size: None,
            choosing: first,
            first,
            moving: None,
            current: current.clone(),
        };
        if first {
            picker.stage(current);
        }
        picker
    }

    /// Whether files are being moved; the dialog stays open until they are.
    pub fn moving(&self) -> bool {
        self.moving.is_some()
    }

    fn stage(&mut self, target: PathBuf) {
        self.problem = (target != self.current)
            .then(|| super::check(&self.current, &target))
            .flatten();
        self.target = Some(target);
    }

    /// The folder a drive's tile stands for: the library itself on its own drive.
    fn pick(&mut self, ix: usize) {
        let drive = &self.drives[ix];
        let here = drives::of(&self.drives, &self.current).is_some_and(|d| d.mount == drive.mount);
        let target = if here {
            self.current.clone()
        } else {
            drives::folder(drive, &self.drives, &store::data())
        };
        self.stage(target);
    }

    fn browse(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("Keep the library here").into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.stage(path);
                cx.notify();
            });
        })
        .detach();
    }

    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(to) = self.target.clone() else {
            return;
        };
        let from = self.current.clone();
        let handle = window.window_handle();
        if to == from {
            // Only the first start gets here: keeping the suggestion moves nothing.
            crate::settings::update(|saved| saved.library = Some(to));
            self.finish(window, cx);
            return;
        }
        let busy = self
            .shell
            .read_with(cx, |shell, _| shell.busy())
            .unwrap_or(true);
        if busy {
            window.push_notification(
                ui::toast::failed(t!("Close the game and let downloads finish before moving")),
                cx,
            );
            return;
        }
        self.moving = Some((0, 0));
        cx.notify();
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let target = to.clone();
        let moving = crate::runtime::spawn(crate::runtime::blocking(move || {
            super::relocate(&from, &target, |done, total| {
                let _ = events_tx.send((done, total));
            })
            .map_err(|err| err.to_string())
        }));
        cx.spawn(async move |this, cx| {
            while let Some(step) = events.recv().await {
                let _ = this.update(cx, |this, cx| {
                    this.moving = Some(step);
                    cx.notify();
                });
            }
            let moved = moving
                .await
                .map_err(|err| err.to_string())
                .and_then(|result| result)
                .and_then(|result| result);
            if moved.is_ok() {
                crate::settings::update(|saved| saved.library = Some(to.clone()));
            }
            let _ = handle.update(cx, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.moving = None;
                    match moved {
                        Ok(()) => {
                            this.current = to.clone();
                            this.choosing = false;
                            let _ = this.shell.update(cx, |shell, cx| shell.relocated(cx));
                            if !this.first {
                                let text = t!(
                                    "Moved! Your library lives in {path} now",
                                    path = to.display()
                                );
                                window.push_notification(ui::toast::done(text), cx);
                            }
                            this.finish(window, cx);
                        }
                        Err(err) => window.push_notification(
                            ui::toast::failed_because(t!("Your library stayed where it was"), err),
                            cx,
                        ),
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// The first start goes on to offer the game someone already plays.
    fn finish(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.first {
            window.close_dialog(cx);
            Shell::welcome(self.shell.clone(), window, cx);
        }
    }

    /// What picking `target` would mean, line by line, and whether it can go ahead.
    fn verdict(&self) -> (Vec<(String, Tone)>, bool) {
        let Some(target) = &self.target else {
            return (
                vec![(t!("Pick a drive or a folder.").into(), Tone::Plain)],
                false,
            );
        };
        let to = drives::of(&self.drives, target);
        let note = to
            .and_then(|d| note(d.kind).map(|text| (text.to_owned(), tone(d.kind))))
            .into_iter();
        if *target == self.current {
            return if self.first {
                (note.collect(), true)
            } else {
                (
                    vec![(t!("Your library is already here.").into(), Tone::Plain)],
                    false,
                )
            };
        }
        if let Some(problem) = &self.problem {
            return (vec![(problem.clone(), Tone::Bad)], false);
        }
        let from = drives::of(&self.drives, &self.current);
        let same = matches!((to, from), (Some(a), Some(b)) if a.mount == b.mount);
        let mut lines = Vec::new();
        let mut ok = true;
        match (self.size, to) {
            (None, _) => {
                lines.push((t!("Counting what your library takes.").into(), Tone::Plain));
                ok = false;
            }
            _ if same => lines.push((
                t!("Same drive, so the move is instant.").into(),
                Tone::Plain,
            )),
            (Some((0, _)), Some(d)) if d.free < GAME => lines.push((
                t!(
                    "Only {free} free here, and a game build takes about 6 GB.",
                    free = bytes(d.free)
                ),
                Tone::Warn,
            )),
            (Some((need, _)), Some(d)) if need > d.free => {
                lines.push((
                    t!(
                        "Not enough room: it needs {need}, and only {free} is free.",
                        need = bytes(need),
                        free = bytes(d.free)
                    ),
                    Tone::Bad,
                ));
                ok = false;
            }
            (Some((need, _)), Some(d)) if need > 0 => {
                let left = d.free - need;
                lines.push((
                    t!(
                        "Needs {need}, and leaves {free} free.",
                        need = bytes(need),
                        free = bytes(left)
                    ),
                    if left < LOW { Tone::Warn } else { Tone::Plain },
                ));
            }
            _ => {}
        }
        lines.extend(note);
        (lines, ok)
    }

    /// Where the library is now, what it holds, and the way to change it.
    fn summary(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let mut meta = div().flex().flex_wrap().gap_x_4().gap_y_1();
        meta = match self.size {
            Some((size, count)) => meta
                .child(Meta::new(IconName::Layers, instances(count)))
                .child(Meta::new(IconName::Folder, bytes(size))),
            None => meta.child(Meta::new(IconName::LoaderCircle, t!("Counting"))),
        };
        if let Some(drive) = drives::of(&self.drives, &self.current) {
            meta = meta.child(Meta::new(
                icon(drive.kind),
                t!(
                    "{free} free on {drive}",
                    free = bytes(drive.free),
                    drive = drive.name
                ),
            ));
        }
        let dir = self.current.clone();
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.current.display().to_string()),
            )
            .child(meta)
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("library-open")
                            .ghost()
                            .cursor_pointer()
                            .icon(IconName::FolderOpen)
                            .label(t!("Open folder"))
                            .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                    )
                    .child(
                        FancyButton::new("library-change", t!("Change location"))
                            .small()
                            .icon(IconName::HardDrive)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.choosing = true;
                                this.stage(this.current.clone());
                                cx.notify();
                            })),
                    ),
            )
    }

    fn tile(&self, ix: usize, picked: bool, here: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let drive = &self.drives[ix];
        let theme = cx.theme();
        let used = 1. - drive.free as f32 / drive.total.max(1) as f32;
        let fill = if drive.free * 10 < drive.total {
            theme.danger
        } else if picked {
            theme.primary
        } else {
            theme.muted_foreground
        };
        // Selected is the accent at 25%; hovering it raises the alpha instead of going grey.
        let (bg, hover) = if picked {
            (theme.primary.opacity(0.25), theme.primary.opacity(0.32))
        } else {
            (theme.group_box, ui::hovered(theme.group_box, cx))
        };
        let tag = |text: &'static str| {
            div()
                .flex_none()
                .px_1p5()
                .rounded(px(4.))
                .bg(theme.muted)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        div()
            .id(("drive", ix))
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(if picked { theme.primary } else { theme.border })
            .bg(bg)
            .hover(move |style| style.bg(hover))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(crate::settings::radio(picked, cx))
                    .child(Glyph::new(icon(drive.kind)).size_4())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(drive.name.clone()),
                    )
                    .when(here, |this| this.child(tag(t!("Current"))))
                    .children(badge(drive.kind).map(tag)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(drive.mount.display().to_string()),
            )
            .child(
                div()
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .bg(theme.border)
                    .child(div().h_full().w(relative(used)).rounded_full().bg(fill)),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(t!(
                "{free} free of {total}",
                free = bytes(drive.free),
                total = bytes(drive.total)
            )))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.pick(ix);
                cx.notify();
            }))
    }

    fn chooser(&self, cx: &mut Context<Self>) -> Div {
        let at = |path: &std::path::Path| drives::of(&self.drives, path).map(|d| d.mount.clone());
        let picked = self.target.as_deref().and_then(at);
        let here = at(&self.current);
        // Where it goes comes in first, where the summary and its button were, and the drives
        // below it one after another.
        let reduce = cx.reduce_motion();
        let mut tiles = Vec::new();
        for (ix, drive) in self.drives.iter().enumerate() {
            let is = |mount: &Option<PathBuf>| mount.as_ref() == Some(&drive.mount);
            let tile = self.tile(ix, is(&picked), is(&here), cx);
            tiles.push(ui::enter_nth(tile, ("drive-in", ix), ix + 1, reduce));
        }
        let (lines, ok) = self.verdict();
        let actions = self.actions(ok, cx);
        let theme = cx.theme();
        let color = |tone: Tone| match tone {
            Tone::Plain => theme.muted_foreground,
            Tone::Warn => ui::warning_text(cx),
            Tone::Bad => theme.danger,
        };
        let destination = div()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .min_w_0()
                            .child(div().text_xs().text_color(theme.muted_foreground).child(
                                if self.first {
                                    t!("Location")
                                } else {
                                    t!("New location")
                                },
                            ))
                            .child(div().text_sm().font_weight(FontWeight::SEMIBOLD).child(
                                match &self.target {
                                    Some(target) => target.display().to_string(),
                                    None => t!("None yet").into(),
                                },
                            )),
                    )
                    .child(
                        Button::new("library-browse")
                            .ghost()
                            .cursor_pointer()
                            .icon(IconName::FolderOpen)
                            .label(t!("Choose folder"))
                            .on_click(cx.listener(|this, _, _, cx| this.browse(cx))),
                    ),
            )
            // What it means beside the button that does it, so both stay in sight.
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_4()
                    // Room for its two lines whether or not they are there: picking another
                    // drive mustn't move the tiles below under the pointer.
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .min_h(px(36.))
                            .justify_end()
                            .gap_1()
                            .children(lines.into_iter().map(|(text, tone)| {
                                div().text_xs().text_color(color(tone)).child(text)
                            })),
                    )
                    .child(actions),
            );
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(ui::enter_nth(destination, "destination-in", 0, reduce))
            .child(div().grid().grid_cols(2).gap_3().children(tiles))
    }

    fn actions(&self, ok: bool, cx: &mut Context<Self>) -> Div {
        let label = if self.first {
            t!("Use this location")
        } else {
            t!("Move library")
        };
        div()
            .flex()
            .flex_none()
            .gap_2()
            .when(!self.first, |this| {
                this.child(
                    Button::new("library-cancel")
                        .ghost()
                        .cursor_pointer()
                        .label(t!("Cancel"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.choosing = false;
                            cx.notify();
                        })),
                )
            })
            .child(
                FancyButton::new("library-confirm", label)
                    .small()
                    .icon(IconName::Check)
                    .disabled(!ok)
                    .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
            )
    }

    fn progress(&self, done: u64, total: u64, cx: &App) -> Div {
        let theme = cx.theme();
        let t = if total == 0 {
            0.
        } else {
            done as f32 / total as f32
        };
        div()
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.group_box)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t!("Moving your library")),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(t!(
                "Hang tight! This stays open until every file is across."
            )))
            .child(
                div()
                    .h(px(6.))
                    .w_full()
                    .rounded_full()
                    .bg(theme.border)
                    .child(
                        div()
                            .h_full()
                            .w(relative(t))
                            .rounded_full()
                            .bg(theme.primary),
                    ),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(t!(
                "{done} of {total}",
                done = bytes(done),
                total = bytes(total)
            )))
    }
}

impl Render for Picker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((done, total)) = self.moving {
            self.progress(done, total, cx).into_any_element()
        } else if self.choosing {
            self.chooser(cx).into_any_element()
        } else {
            // Back from choosing (Cancel, or a move that finished) it comes in again.
            ui::enter(self.summary(cx), "library-summary").into_any_element()
        }
    }
}

/// Asks where the library goes, on a first start with nothing in it yet.
pub fn first_start(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| Picker::new(shell, true, cx));
    window.open_dialog(cx, move |dialog, _, cx| {
        let muted = cx.theme().muted_foreground;
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(t!("Where should lata keep your instances?"))
            // Pinned at the top: its height changes with the picked folder, and centered it
            // would slide the button out from under the pointer.
            .margin_top(px(72.))
            .w(px(640.))
            // One choice to make, and the suggestion is fine: no way out but picking.
            .close_button(false)
            .overlay_closable(false)
            .keyboard(false)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .child(div().text_sm().text_color(muted).child(
                        t!("Game files and mods take a lot of room. Pick a drive with space; you can change it later in Settings."),
                    ))
                    .child(view.clone()),
            )
    });
}

fn instances(count: usize) -> String {
    tn!(count, "{n} instance", "{n} instances")
}

fn icon(kind: Kind) -> IconName {
    match kind {
        Kind::Removable => IconName::Usb,
        _ => IconName::HardDrive,
    }
}

fn badge(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Nvme => Some("NVMe"),
        Kind::Ssd => Some("SSD"),
        Kind::Hdd => Some("HDD"),
        Kind::Removable => Some("USB"),
        Kind::Unknown => None,
    }
}

/// What the kind of drive means for the game.
fn note(kind: Kind) -> Option<&'static str> {
    match kind {
        Kind::Nvme => Some(t!(
            "NVMe is the fastest kind of drive: worlds and mods load quickest here."
        )),
        Kind::Ssd => Some(t!("An SSD: worlds and mods load fast here.")),
        Kind::Hdd => Some(t!(
            "A hard drive: lots of room, but worlds and mods load slower than on an SSD."
        )),
        Kind::Removable => Some(t!(
            "A removable drive: keep it plugged in or your instances won't show up."
        )),
        Kind::Unknown => None,
    }
}

fn tone(kind: Kind) -> Tone {
    match kind {
        Kind::Removable => Tone::Warn,
        _ => Tone::Plain,
    }
}
