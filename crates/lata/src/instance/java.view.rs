//! The question asked before Java mods are turned on for an instance: unlike Lua mods, they
//! can do anything on the computer, so it is the user's call, said plainly, each time.

use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::*;

use crate::ui::{self, FancyButton};

/// Opens the dialog; `yes` runs when the user chooses to turn them on.
pub fn confirm(
    window: &mut Window,
    cx: &mut App,
    yes: impl Fn(&mut Window, &mut App) + Clone + 'static,
) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let yes = yes.clone();
        let muted = cx.theme().muted_foreground;
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title("Turn on Java mods?")
            .w(px(460.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .text_sm()
                    .child(
                        "Java mods change the game itself, and can do anything on your \
                         computer that you can: read your files, use the network, run \
                         programs. Only turn this on for mods you trust.",
                    )
                    .child(div().text_color(muted).child(
                        "lata starts the game with ZombieBuddy hooked in, in this instance \
                         only. ZombieBuddy then asks you about every Java mod before loading \
                         it. You can turn this off in the instance's settings any time.",
                    )),
            )
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .pt_4()
                    .child(
                        ui::secondary("java-keep-off", "Keep off")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        FancyButton::new("java-turn-on", "Turn on")
                            .small()
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                yes(window, cx);
                            }),
                    ),
            )
    });
}
