//! Screenshots taken in any instance, in one place: by day, newest first, and big in a viewer
//! that covers the window.

#[path = "shots.model.rs"]
mod shots;

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use gpui_kit::assets::IconName;
use gpui_kit::base::spring;
use gpui_kit::component::button::Button;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon as Glyph, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::instance::{Instance, store};
use crate::ui::{self, Empty, FancyButton};
use shots::Shot;

/// A tile's width; its picture is 16:9 under it. Three fit beside the news panel at 1280 px.
const TILE: f32 = 196.;
/// The viewer's bars above and below the picture.
const CHROME: f32 = 140.;
/// The viewer's arrows beside the picture, with their gaps and padding.
const ARROWS: f32 = 136.;
/// The viewer opens like a modal (transitions.dev): 250 ms in from 96%, 150 ms back out to
/// it, on ease-out-quint (their cubic-bezier(0.22, 1, 0.36, 1)).
const OPEN: std::time::Duration = std::time::Duration::from_millis(250);
const OUT: std::time::Duration = std::time::Duration::from_millis(150);
const SCALE: f32 = 0.96;
/// Stepping to the next picture slides it like a page side by side: 200 ms over 8 px.
const STEP: std::time::Duration = std::time::Duration::from_millis(200);
const TRAVEL: f32 = 8.;

pub struct Screenshots {
    instances: Vec<Instance>,
    /// `None` until the first look at the folders is done.
    shots: Option<Vec<Shot>>,
    /// Small copies for the grid, by screenshot, as they are made.
    thumbs: HashMap<PathBuf, PathBuf>,
    /// The instance shown, by id; `None` shows them all.
    filter: Option<SharedString>,
    /// The screenshot open in the viewer, as an index into what the grid shows.
    viewing: Option<usize>,
    /// The viewer's Delete asks once more, in place.
    confirming: bool,
    /// Which way the last step went (-1 back, 1 on, 0 just opened): the picture comes in
    /// from that side.
    dir: isize,
    /// Counts openings, so each one replays the viewer's entrance.
    opened: u64,
    /// The viewer is fading out; it goes once that is done.
    leaving: bool,
    /// Bumped on every look, so a slow one that finishes late is dropped.
    epoch: u64,
    focus: FocusHandle,
}

impl Screenshots {
    pub fn new(instances: Vec<Instance>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            instances: Vec::new(),
            shots: None,
            thumbs: HashMap::new(),
            filter: None,
            viewing: None,
            confirming: false,
            dir: 0,
            opened: 0,
            leaving: false,
            epoch: 0,
            focus: cx.focus_handle(),
        };
        this.reload(instances, cx);
        this
    }

    /// Looks at the folders again, then makes the small copies still missing, newest first.
    pub fn reload(&mut self, instances: Vec<Instance>, cx: &mut Context<Self>) {
        self.instances = instances.clone();
        self.epoch += 1;
        let epoch = self.epoch;
        let cache = store::data().join("cache/thumbs");
        cx.spawn(async move |this, cx| {
            let Ok(found) = crate::runtime::blocking(move || shots::list(&instances)).await else {
                return;
            };
            let paths: Vec<PathBuf> = found.iter().map(|shot| shot.path.clone()).collect();
            let fresh = this
                .update(cx, |this, cx| {
                    if this.epoch != epoch {
                        return false;
                    }
                    // An instance whose screenshots are all gone can't stay the filter.
                    if let Some(id) = &this.filter
                        && !found.iter().any(|shot| shot.instance == *id)
                    {
                        this.filter = None;
                    }
                    this.shots = Some(found);
                    this.clamp();
                    cx.notify();
                    true
                })
                .unwrap_or(false);
            if !fresh {
                return;
            }
            let (made_tx, mut made) = tokio::sync::mpsc::unbounded_channel();
            let making = crate::runtime::spawn(crate::runtime::blocking(move || {
                for path in paths {
                    if let Ok(thumb) = shots::thumb(&path, &cache) {
                        let _ = made_tx.send((path, thumb));
                    }
                }
            }));
            while let Some((path, thumb)) = made.recv().await {
                let gone = this
                    .update(cx, |this, cx| {
                        this.thumbs.insert(path, thumb);
                        cx.notify();
                        this.epoch != epoch
                    })
                    .unwrap_or(true);
                if gone {
                    break;
                }
            }
            drop(making);
        })
        .detach();
    }

    fn shown(&self) -> Vec<&Shot> {
        self.shots
            .iter()
            .flatten()
            .filter(|shot| self.filter.as_ref().is_none_or(|id| shot.instance == *id))
            .collect()
    }

    /// Keeps the viewer on a screenshot that exists, or closes it when none is left.
    fn clamp(&mut self) {
        let count = self.shown().len();
        self.viewing = self
            .viewing
            .and_then(|ix| (count > 0).then(|| ix.min(count - 1)));
        self.confirming = false;
    }

    fn open(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.viewing = Some(ix);
        self.confirming = false;
        self.leaving = false;
        self.dir = 0;
        self.opened += 1;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let count = self.shown().len();
        if let Some(ix) = self.viewing.filter(|_| !self.leaving) {
            let next = ix as isize + by;
            if (0..count as isize).contains(&next) {
                self.viewing = Some(next as usize);
                self.confirming = false;
                self.dir = by.signum();
                cx.notify();
            }
        }
    }

    /// Fades the viewer out, then drops it.
    fn close(&mut self, cx: &mut Context<Self>) {
        if self.leaving || self.viewing.is_none() {
            return;
        }
        self.leaving = true;
        let opened = self.opened;
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(OUT).await;
            let _ = this.update(cx, |this, cx| {
                // Opened again meanwhile: that one stays.
                if this.opened == opened {
                    this.viewing = None;
                    this.leaving = false;
                    this.confirming = false;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn copy(&self, path: PathBuf, window: &mut Window, cx: &mut App) {
        match std::fs::read(&path) {
            Ok(bytes) => {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                    ImageFormat::Png,
                    bytes,
                )));
                window.push_notification(ui::toast::done(t!("Copied! Paste it anywhere")), cx);
            }
            Err(err) => window.push_notification(
                ui::toast::failed_because(t!("Couldn't copy that one"), err),
                cx,
            ),
        }
    }

    fn delete(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(err) = std::fs::remove_file(&path) {
            self.confirming = false;
            window.push_notification(
                ui::toast::failed_because(t!("That screenshot wouldn't go"), err),
                cx,
            );
            cx.notify();
            return;
        }
        // The viewer stays at the same place: the next one slides into it.
        if let Some(shots) = &mut self.shots {
            shots.retain(|shot| shot.path != path);
        }
        if let Some(thumb) = self.thumbs.remove(&path) {
            let _ = std::fs::remove_file(thumb);
        }
        if let Some(id) = self.filter.clone()
            && !self.shown().iter().any(|shot| shot.instance == id)
        {
            self.filter = None;
        }
        self.dir = 1;
        self.clamp();
        cx.notify();
    }

    /// The instance filter, styled like the Library's sort: only when screenshots come from
    /// more than one instance.
    fn filter(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let mut counts: Vec<(SharedString, SharedString, usize)> = Vec::new();
        for shot in self.shots.iter().flatten() {
            match counts.iter_mut().find(|(id, _, _)| *id == shot.instance) {
                Some((_, _, count)) => *count += 1,
                None => counts.push((shot.instance.clone(), shot.name.clone(), 1)),
            }
        }
        if counts.len() < 2 {
            return None;
        }
        let total = self.shots.iter().flatten().count();
        let current = self.filter.clone();
        let label: SharedString = match &current {
            Some(id) => counts
                .iter()
                .find(|(i, _, _)| i == id)
                .map(|(_, name, _)| name.clone())
                .unwrap_or_default(),
            None => t!("All instances").into(),
        };
        let this = cx.entity().downgrade();
        Some(
            Button::new("shots-filter")
                .outline()
                .h(px(36.))
                .cursor_pointer()
                .icon(IconName::Layers)
                .label(label)
                .dropdown_menu(move |menu, _, _| {
                    let pick = |id: Option<SharedString>| {
                        let this = this.clone();
                        move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                            let _ = this.update(cx, |this, cx| {
                                this.filter = id.clone();
                                this.viewing = None;
                                cx.notify();
                            });
                        }
                    };
                    let menu = menu.item(
                        PopupMenuItem::new(t!("All instances ({total})", total = total))
                            .checked(current.is_none())
                            .on_click(pick(None)),
                    );
                    counts.iter().fold(menu, |menu, (id, name, count)| {
                        menu.item(
                            PopupMenuItem::new(format!("{name} ({count})"))
                                .checked(current.as_ref() == Some(id))
                                .on_click(pick(Some(id.clone()))),
                        )
                    })
                }),
        )
    }

    fn tile(
        &self,
        ix: usize,
        shot: &Shot,
        named: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let key = shot.path.to_string_lossy().into_owned();
        let hovered = window.use_keyed_state(
            SharedString::from(format!("shot-hover-{key}")),
            cx,
            |_, _| false,
        );
        let lit: f32 = spring(
            SharedString::from(format!("shot-lit-{key}")),
            if *hovered.read(cx) { 1. } else { 0. },
            ui::SPRING_FAST,
            window,
            cx,
        );
        let theme = cx.theme();
        let box_h = TILE * 9. / 16.;
        // The whole picture, never cropped: an ultrawide or 4:3 one gets bars instead. Sized
        // here, since GPUI's img takes its height from its width and spills out of a box.
        let (w, h) = (shot.size.0.max(1) as f32, shot.size.1.max(1) as f32);
        let fit = (TILE / w).min(box_h / h);
        let radius = theme.radius_lg;
        let picture = div()
            .flex()
            .items_center()
            .justify_center()
            .w(px(TILE))
            .h(px(box_h))
            .rounded(radius)
            .overflow_hidden()
            .bg(theme.muted)
            .children(self.thumbs.get(&shot.path).map(|thumb| {
                // Fades in as its small copy lands, instead of popping.
                img(thumb.clone())
                    .w(px(w * fit))
                    .h(px(h * fit))
                    .rounded(radius)
                    .with_animation(
                        SharedString::from(format!("shot-in-{key}")),
                        Animation::new(ui::ENTER).with_easing(ui::ease_out_quint),
                        |img, t| img.opacity(t),
                    )
            }));
        let caption = div()
            .flex()
            .justify_between()
            .gap_2()
            .px_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .children(named.then(|| shot.name.clone())),
            )
            .child(
                div()
                    .flex_none()
                    .child(shot.taken.format("%H:%M").to_string()),
            );
        div()
            .id(SharedString::from(format!("shot-{key}")))
            .flex()
            .flex_col()
            .gap_1p5()
            .p_1p5()
            .rounded(theme.radius_lg)
            .cursor_pointer()
            .bg(theme.sidebar_accent.opacity(lit))
            .on_hover(move |on, window, cx| {
                hovered.update(cx, |h, _| *h = *on);
                window.refresh();
            })
            .child(picture)
            .child(caption)
            .on_click(cx.listener(move |this, _, window, cx| this.open(ix, window, cx)))
    }

    fn viewer(&self, ix: usize, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown();
        let (total, shot) = (shown.len(), shown[ix].clone());
        let size = window.viewport_size();
        let muted = white().opacity(0.6);

        // Fit to the room between the arrows and the bars; GPUI's img keeps its shape from its
        // width, so a box alone let a 4:3 picture spill over the bar below.
        let room = (
            f32::from(size.width) - ARROWS,
            f32::from(size.height) - CHROME,
        );
        let (w, h) = (shot.size.0.max(1) as f32, shot.size.1.max(1) as f32);
        let scale = (room.0 / w).min(room.1 / h).max(0.);
        let (w, h) = (w * scale, h * scale);
        let (dir, leaving) = (self.dir as f32, self.leaving);
        let picture = div().flex().flex_1().min_w_0().justify_center().child(
            img(shot.path.clone())
                .w(px(w))
                .h(px(h))
                .rounded(px(10.))
                // Opening grows it into place and closing shrinks it back; a step slides the
                // next one in from its side.
                .with_animation(
                    SharedString::from(format!(
                        "viewer-picture-{}-{}-{}",
                        self.opened,
                        self.leaving,
                        shot.path.display()
                    )),
                    Animation::new(match (leaving, dir == 0.) {
                        (true, _) => OUT,
                        (false, true) => OPEN,
                        (false, false) => STEP,
                    })
                    .with_easing(ui::ease_out_quint),
                    move |img, t| {
                        let (grow, shift, shown) = match (leaving, dir == 0.) {
                            (true, _) => (1. - (1. - SCALE) * t, 0., 1. - t),
                            (false, true) => (SCALE + (1. - SCALE) * t, 0., t),
                            (false, false) => (1., dir * TRAVEL * (1. - t), t),
                        };
                        img.w(px(w * grow))
                            .h(px(h * grow))
                            .relative()
                            .left(px(shift))
                            .opacity(shown)
                    },
                ),
        );

        let arrow = |id: &'static str,
                     icon: IconName,
                     enabled: bool,
                     by: isize,
                     window: &mut Window,
                     cx: &mut Context<Self>| {
            glass(id, enabled, window, cx)
                .size(px(44.))
                .child(Glyph::new(icon).size_5())
                .when(enabled, |this| {
                    this.on_click(cx.listener(move |this, _, _, cx| this.step(by, cx)))
                })
        };
        let prev = arrow("shot-prev", IconName::ChevronLeft, ix > 0, -1, window, cx);
        let next = arrow(
            "shot-next",
            IconName::ChevronRight,
            ix + 1 < total,
            1,
            window,
            cx,
        );
        let close = glass("shot-close", true, window, cx)
            .size(px(36.))
            .child(Glyph::new(IconName::Close).size_4())
            .on_click(cx.listener(|this, _, _, cx| this.close(cx)));

        let (copy, folder, gone) = (
            shot.path.clone(),
            shot.path.parent().map(PathBuf::from).unwrap_or_default(),
            shot.path.clone(),
        );
        let pill = |id: &'static str,
                    icon: IconName,
                    label: &'static str,
                    window: &mut Window,
                    cx: &mut Context<Self>| {
            glass(id, true, window, cx)
                .h(px(36.))
                .px_3()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(Glyph::new(icon).size_4())
                .child(label)
        };
        let actions = if self.confirming {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("Delete it for good?")),
                )
                .child(
                    pill("shot-keep", IconName::Undo2, t!("Keep"), window, cx).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.confirming = false;
                            cx.notify();
                        }),
                    ),
                )
                .child(
                    FancyButton::new("shot-delete-yes", t!("Delete"))
                        .small()
                        .danger()
                        .icon(IconName::Trash)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.delete(gone.clone(), window, cx)
                        })),
                )
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    pill("shot-copy", IconName::Copy, t!("Copy"), window, cx).on_click(
                        cx.listener(move |this, _, window, cx| this.copy(copy.clone(), window, cx)),
                    ),
                )
                .child(
                    pill(
                        "shot-folder",
                        IconName::FolderOpen,
                        t!("Open folder"),
                        window,
                        cx,
                    )
                    .on_click(move |_, _, cx| cx.open_with_system(&folder)),
                )
                .child(
                    pill("shot-delete", IconName::Trash, t!("Delete"), window, cx).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.confirming = true;
                            cx.notify();
                        }),
                    ),
                )
        };

        let viewer = div()
            .id("shot-viewer")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                match event.keystroke.key.as_str() {
                    "left" => this.step(-1, cx),
                    "right" => this.step(1, cx),
                    "escape" => this.close(cx),
                    _ => {}
                }
            }))
            .occlude()
            .w(size.width)
            .h(size.height)
            .flex()
            .flex_col()
            // Dark in both themes: a picture reads best on black, as in any photo viewer.
            .bg(rgb(0x09090b).opacity(0.94))
            .text_color(white())
            .child(div().flex().justify_end().p_3().child(close))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .child(prev)
                    .child(picture)
                    .child(next),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .px_6()
                    .py_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(shot.name.clone()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .gap_4()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(t!(
                                        "{date} at {time}",
                                        date = shot.taken.format("%b %-d, %Y"),
                                        time = shot.taken.format("%H:%M")
                                    ))
                                    .child(t!("{n} of {total}", n = ix + 1, total = total)),
                            ),
                    )
                    .child(actions),
            );
        // In with a fade over the page; out quicker, the same way.
        let viewer = if leaving {
            viewer
                .with_animation(
                    SharedString::from(format!("viewer-out-{}", self.opened)),
                    Animation::new(OUT).with_easing(ui::ease_out_quint),
                    |viewer, t| viewer.opacity(1. - t),
                )
                .into_any_element()
        } else {
            viewer
                .with_animation(
                    SharedString::from(format!("viewer-in-{}", self.opened)),
                    Animation::new(OPEN).with_easing(ui::ease_out_quint),
                    |viewer, t| viewer.opacity(t),
                )
                .into_any_element()
        };
        deferred(
            anchored()
                .position_mode(AnchoredPositionMode::Window)
                .position(point(px(0.), px(0.)))
                .child(viewer),
        )
        .with_priority(1)
    }
}

/// A light control on the viewer's dark backdrop; its fill rises on hover with a spring.
fn glass(id: &'static str, enabled: bool, window: &mut Window, cx: &mut App) -> Stateful<Div> {
    let hovered =
        window.use_keyed_state(SharedString::from(format!("{id}-hover")), cx, |_, _| false);
    let lit: f32 = spring(
        SharedString::from(format!("{id}-lit")),
        if *hovered.read(cx) && enabled { 1. } else { 0. },
        ui::SPRING_FAST,
        window,
        cx,
    );
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded_full()
        .bg(white().opacity(0.08 + 0.1 * lit))
        .map(|this| {
            if enabled {
                this.cursor_pointer()
            } else {
                this.opacity(0.35)
            }
        })
        .on_hover(move |on, window, cx| {
            hovered.update(cx, |h, _| *h = *on);
            window.refresh();
        })
}

/// "Today", "Yesterday", else the date.
fn day(date: NaiveDate) -> String {
    let today = Local::now().date_naive();
    if date == today {
        t!("Today").into()
    } else if today.pred_opt() == Some(date) {
        t!("Yesterday").into()
    } else {
        date.format("%b %-d, %Y").to_string()
    }
}

impl Render for Screenshots {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let shown: Vec<Shot> = self.shown().into_iter().cloned().collect();
        let named = self.filter.is_none()
            && shown
                .iter()
                .any(|shot| Some(&shot.instance) != shown.first().map(|s| &s.instance));
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(t!("Screenshots")),
            )
            .when(!shown.is_empty(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(shown.len().to_string()),
                )
            })
            .child(div().flex_1())
            .children(self.filter(cx));
        let body = match &self.shots {
            // A quick look; no empty state while it runs, it would be a lie.
            None => div(),
            Some(all) if all.is_empty() => div().flex().flex_1().child(Empty::new(
                IconName::Image,
                t!("No screenshots yet"),
                t!("Screenshots you take in the game show up here, from every instance."),
            )),
            Some(_) => {
                let mut days = div().flex().flex_col().gap_5();
                let mut ix = 0;
                for group in shown.chunk_by(|a, b| a.taken.date() == b.taken.date()) {
                    let mut tiles = Vec::new();
                    for shot in group {
                        tiles.push(self.tile(ix, shot, named, window, cx));
                        ix += 1;
                    }
                    days = days.child(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(day(group[0].taken.date())),
                            )
                            .child(div().flex().flex_wrap().gap_2().children(tiles)),
                    );
                }
                days
            }
        };
        let viewer = self.viewing.map(|ix| self.viewer(ix, window, cx));
        div()
            .id("screenshots")
            .flex()
            .flex_col()
            .size_full()
            .gap_5()
            .p_6()
            .overflow_y_scroll()
            .child(header)
            .child(ui::enter(body, "shots-body"))
            .children(viewer)
    }
}
