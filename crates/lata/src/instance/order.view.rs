//! "Load order" dialog: the instance's active mods in the order the game loads them, to drag
//! into place or sort by what each one asks for, and under them the maps, only when two
//! of them fight over a cell (the game offers its map order just then too). Saving rewrites
//! the order in `mods/default.txt`.

use gpui_kit::assets::IconName;
use gpui_kit::base::Disableable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Instance;
use super::maps::{self, Map};
use super::mods::{self, Mod};
use crate::ui::{self, FancyButton, toast};

/// What runs once a new order is written.
type Saved = Box<dyn Fn(&mut Window, &mut App)>;

/// Which of the dialog's two lists a row is in.
#[derive(Clone, Copy, PartialEq)]
enum List {
    Mods,
    Maps,
}

/// A row being dragged; it draws itself under the pointer.
#[derive(Clone)]
struct Dragged {
    list: List,
    from: usize,
    name: SharedString,
}

impl Render for Dragged {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_3()
            .py_2()
            .rounded(theme.radius)
            .bg(theme.popover)
            .border_1()
            .border_color(theme.primary)
            .shadow_md()
            .text_sm()
            .font_weight(FontWeight::MEDIUM)
            .child(self.name.clone())
    }
}

/// Opens the dialog for `instance`; `saved` runs once a new order is written.
pub fn open(
    instance: Instance,
    saved: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let view = cx.new(|_| Order::new(instance, Box::new(saved)));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title("Load order")
            .centered()
            .w(px(560.))
            .child(view.clone())
    });
}

struct Order {
    instance: Instance,
    /// The active mods, first to load on top.
    mods: Vec<Mod>,
    /// Their maps, the one that wins a shared cell on top.
    maps: Vec<Map>,
    saved: Saved,
}

impl Order {
    fn new(instance: Instance, saved: Saved) -> Self {
        let active = mods::active(&instance);
        let installed = mods::list(&instance);
        // In the file's order; ids with no mod behind them have nothing to show.
        let mods: Vec<Mod> = active
            .mods
            .iter()
            .filter_map(|id| installed.iter().find(|m| m.id == *id).cloned())
            .collect();
        let refs: Vec<&Mod> = mods.iter().collect();
        // ponytail: read once; the maps of a mod don't change while this is open, and their
        // grouping only follows the mod order loosely.
        let maps = maps::in_order(&refs, mods::is_modern(&instance), &active.maps);
        Self {
            instance,
            mods,
            maps,
            saved,
        }
    }

    /// Whether any two maps share a cell, which is when their order matters.
    fn maps_matter(&self) -> bool {
        self.maps.iter().any(|map| !map.conflicts.is_empty())
    }

    fn shift(&mut self, list: List, from: usize, to: usize, cx: &mut Context<Self>) {
        match list {
            List::Mods if from < self.mods.len() && to < self.mods.len() => {
                let moved = self.mods.remove(from);
                self.mods.insert(to, moved);
            }
            List::Maps if from < self.maps.len() && to < self.maps.len() => {
                let moved = self.maps.remove(from);
                self.maps.insert(to, moved);
            }
            _ => {}
        }
        cx.notify();
    }

    fn auto(&mut self, cx: &mut Context<Self>) {
        let order = mods::auto_sort(&self.mods);
        self.mods
            .sort_by_key(|m| order.iter().position(|id| *id == m.id));
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.mods.iter().map(|m| m.id.clone()).collect();
        let maps: Vec<String> = self.maps.iter().map(|map| map.name.clone()).collect();
        let maps = self.maps_matter().then_some(maps.as_slice());
        match mods::set_order(&self.instance, &ids, maps) {
            Ok(()) => {
                window.close_dialog(cx);
                window
                    .push_notification(toast::done("Order saved! The game loads it that way"), cx);
                (self.saved)(window, cx);
            }
            Err(err) => {
                window.push_notification(toast::failed_because("That order didn't stick", err), cx)
            }
        }
    }

    /// One row of either list: a grip to drag it by, what it is, a note under it, and arrows
    /// for whoever would rather not drag.
    fn row(
        &self,
        list: List,
        ix: usize,
        count: usize,
        name: SharedString,
        note: Option<(SharedString, bool)>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let key = |part: &'static str| {
            let list = if list == List::Mods { "mod" } else { "map" };
            SharedString::from(format!("order-{list}-{part}-{ix}"))
        };
        let step = |part: &'static str, icon: IconName, to: Option<usize>, tip: &'static str| {
            Button::new(key(part))
                .ghost()
                .xsmall()
                .icon(icon)
                .tooltip(tip)
                .disabled(to.is_none())
                .when_some(to, |button, to| {
                    button
                        .cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| this.shift(list, ix, to, cx)))
                })
        };
        let dragged = Dragged {
            list,
            from: ix,
            name: name.clone(),
        };
        let hover = ui::hovered(theme.background, cx);
        let accent = theme.primary;
        div()
            .id(key("row"))
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .when(ix + 1 < count, |row| {
                row.border_b_1().border_color(theme.border)
            })
            .cursor_grab()
            .hover(move |row| row.bg(hover))
            .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
            // Only a row of the same list can land here; the line shows where it would go.
            .drag_over::<Dragged>(move |row, dragged, _, _| {
                if dragged.list == list {
                    row.border_t_2().border_color(accent)
                } else {
                    row
                }
            })
            .on_drop(cx.listener(move |this, dragged: &Dragged, _, cx| {
                if dragged.list == list {
                    this.shift(list, dragged.from, ix, cx);
                }
            }))
            .child(
                Icon::new(IconName::GripVertical)
                    .size_4()
                    .flex_none()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(24.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child((ix + 1).to_string()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .truncate()
                            .child(name),
                    )
                    .children(note.map(|(text, wrong)| {
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(if wrong {
                                ui::warning_text(cx)
                            } else {
                                theme.muted_foreground
                            })
                            .child(text)
                    })),
            )
            .child(step(
                "up",
                IconName::ChevronUp,
                ix.checked_sub(1),
                "Move up",
            ))
            .child(step(
                "down",
                IconName::ChevronDown,
                (ix + 1 < count).then_some(ix + 1),
                "Move down",
            ))
            .into_any_element()
    }

    fn list(&self, id: &'static str, rows: Vec<AnyElement>, cx: &Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .flex_col()
            .max_h(px(300.))
            .overflow_y_scroll()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .children(rows)
    }
}

impl Render for Order {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let refs: Vec<&Mod> = self.mods.iter().collect();
        // The game refuses an order with a mod above one it requires; so does Save.
        let wrong = mods::out_of_order(&refs).map(|(m, late)| (m.id.clone(), late.name.clone()));
        let count = self.mods.len();
        let mods: Vec<AnyElement> = self
            .mods
            .iter()
            .enumerate()
            .map(|(ix, m)| {
                let note = match &wrong {
                    Some((id, late)) if *id == m.id => {
                        (format!("Needs {late} above it").into(), true)
                    }
                    _ => (m.id.clone().into(), false),
                };
                (ix, SharedString::from(m.name.clone()), note)
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(ix, name, note)| self.row(List::Mods, ix, count, name, Some(note), cx))
            .collect();
        let maps_matter = self.maps_matter();
        let map_count = self.maps.len();
        let maps: Vec<AnyElement> = if maps_matter {
            self.maps
                .iter()
                .enumerate()
                .map(|(ix, map)| {
                    let note = (!map.conflicts.is_empty()).then(|| {
                        let with = map.conflicts.join(", ");
                        (format!("Shares cells with {with}").into(), true)
                    });
                    (ix, SharedString::from(map.name.clone()), note)
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|(ix, name, note)| self.row(List::Maps, ix, map_count, name, note, cx))
                .collect()
        } else {
            Vec::new()
        };
        let theme = cx.theme();
        let hint = |text: &'static str| {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(hint(
                "The game loads these from the top down, so a mod goes under the ones it \
                 needs. Drag them around, or let Auto sort it out.",
            ))
            .child(self.list("order-mods", mods, cx))
            .when(maps_matter, |this| {
                this.child(hint(
                    "Some of your maps want the same spot. Where they overlap, the one on \
                     top wins.",
                ))
                .child(self.list("order-maps", maps, cx))
            })
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(
                        ui::secondary("order-auto", "Auto")
                            .icon(IconName::WandSparkles)
                            .on_click(cx.listener(|this, _, _, cx| this.auto(cx))),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .child(
                                ui::secondary("order-cancel", "Cancel")
                                    .on_click(|_, window, cx| window.close_dialog(cx)),
                            )
                            .child(
                                FancyButton::new("order-save", "Save order")
                                    .small()
                                    .icon(IconName::Check)
                                    .disabled(wrong.is_some())
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            ),
                    ),
            )
    }
}
