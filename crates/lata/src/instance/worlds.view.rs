//! The Worlds tab: the instance's saves, in the Mods table's style, the last played first.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::worlds::World;
use crate::ui::{Empty, Meta};

/// Side of a world's screenshot in its row; the game saves them square.
const THUMB: Pixels = px(48.);

pub fn worlds(worlds: &[World], cx: &App) -> AnyElement {
    if worlds.is_empty() {
        return Empty::new(
            IconName::Globe,
            "No worlds yet",
            "Start a game and its save shows up here.",
        )
        .into_any_element();
    }
    let theme = cx.theme();
    let last = worlds.len() - 1;
    div()
        .id("worlds")
        .flex()
        .flex_col()
        .min_h_0()
        .overflow_y_scroll()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .children(
            worlds
                .iter()
                .enumerate()
                .map(|(ix, world)| row(ix, world, ix == last, cx)),
        )
        .into_any_element()
}

fn row(ix: usize, world: &World, last: bool, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let thumb = match &world.thumb {
        Some(path) => div().flex_none().size(THUMB).child(
            img(path.clone())
                .size_full()
                .rounded(px(8.))
                .object_fit(ObjectFit::Cover)
                .with_loading(|| {
                    Skeleton::new()
                        .size_full()
                        .rounded(px(8.))
                        .into_any_element()
                }),
        ),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(THUMB)
            .rounded(px(8.))
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Globe).size_4()),
    };
    let saved = super::store::now().saturating_sub(world.saved);
    let dir = world.dir.clone();
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .when(!last, |row| row.border_b_1().border_color(theme.border))
        .child(thumb)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(world.title().to_owned()),
                )
                .child(
                    div()
                        .flex()
                        .gap_4()
                        .child(Meta::new(IconName::Gamepad, world.mode.clone()))
                        // The title is the given name: say who plays it too.
                        .children(
                            world
                                .character
                                .clone()
                                .filter(|character| character != world.title())
                                .map(|character| Meta::new(IconName::CircleUser, character)),
                        )
                        .child(Meta::new(
                            IconName::Clock,
                            format!("Played {}", super::ago(saved)),
                        )),
                ),
        )
        .child(
            Button::new(("world-open", ix))
                .ghost()
                .cursor_pointer()
                .icon(IconName::FolderOpen)
                .tooltip("Open folder")
                .on_click(move |_, _, cx| cx.open_with_system(&dir)),
        )
}
