//! "Bring your game" dialog: the worlds found in the game's own folder, to pick which come
//! along. It opens by itself on a first start, and from the create dialog after that.
//! Confirming makes the instances; the copy shows in the download manager.

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::existing::{Existing, Save};
use crate::shell::Shell;
use crate::ui::{self, FancyButton, Meta};

pub fn open(game: Existing, shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|_| Bring {
        // Everything comes along unless unticked.
        picked: vec![true; game.worlds.len()],
        game,
        shell,
    });
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(t!("Bring your game"))
            .centered()
            .w(px(620.))
            .child(view.clone())
    });
}

struct Bring {
    game: Existing,
    /// One flag per world of `game`.
    picked: Vec<bool>,
    shell: WeakEntity<Shell>,
}

impl Bring {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (shell, game) = (self.shell.clone(), self.game.clone());
        let plans = game.plans(&self.picked);
        window.close_dialog(cx);
        // After this update ends: the shell repaints views as the copy starts.
        window.defer(cx, move |window, cx| {
            Shell::adopt(shell, game, plans, window, cx)
        });
    }

    fn row(&self, ix: usize, save: &Save, last: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let world = &save.world;
        let saved = super::store::now().saturating_sub(world.saved);
        let toggle = cx.listener(move |this: &mut Self, on: &bool, _, cx| {
            this.picked[ix] = *on;
            cx.notify();
        });
        div()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .when(!last, |row| row.border_b_1().border_color(theme.border))
            .child(
                Checkbox::new(("bring-world", ix))
                    .cursor_pointer()
                    .checked(self.picked[ix])
                    .on_click(toggle),
            )
            .child(super::worlds_view::thumb(world, cx))
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
                            .child(Meta::new(
                                IconName::Box,
                                if save.modern { "Build 42" } else { "Build 41" },
                            ))
                            .child(Meta::new(
                                IconName::Clock,
                                t!("Played {when}", when = super::ago(saved)),
                            )),
                    ),
            )
    }
}

impl Render for Bring {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let plans = self.game.plans(&self.picked);
        let mods: usize = plans.iter().map(|plan| plan.active.mods.len()).sum();
        let last = self.game.worlds.len().saturating_sub(1);
        let rows: Vec<_> = self
            .game
            .worlds
            .iter()
            .enumerate()
            .map(|(ix, save)| self.row(ix, save, ix == last, cx).into_any_element())
            .collect();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(div().text_sm().text_color(theme.muted_foreground).child(t!(
                "Looks like you've been surviving without us. Pick what comes along: \
                         it gets its own instance here, with the mods it needs and your \
                         settings. Your game's folder stays as it is, lata only copies."
            )))
            .when(!rows.is_empty(), |this| {
                this.child(
                    div()
                        .id("bring-worlds")
                        .flex()
                        .flex_col()
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .rounded(theme.radius_lg)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .children(rows),
                )
            })
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_4()
                    .child(Meta::new(
                        IconName::Puzzle,
                        match mods {
                            0 => t!("No mods to bring").to_owned(),
                            many => tn!(many, "{n} mod", "{n} mods"),
                        },
                    ))
                    .child(Meta::new(
                        IconName::Settings,
                        t!("Settings and key bindings"),
                    ))
                    .when(plans.len() > 1, |this| {
                        this.child(Meta::new(
                            IconName::Box,
                            t!("Build 41 and Build 42 worlds get an instance each"),
                        ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        ui::secondary("bring-later", t!("Not now"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        FancyButton::new("bring", t!("Bring it in"))
                            .small()
                            .icon(IconName::Download)
                            .on_click(cx.listener(|this, _, window, cx| this.confirm(window, cx))),
                    ),
            )
    }
}
