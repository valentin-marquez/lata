//! Title bar: logo, an update when there is one, the download manager, and which instance is
//! running.

use gpui_kit::component::{ActiveTheme, Icon, TitleBar};
use gpui_kit::*;

use crate::assets::LOGO;

/// Taller than gpui-component's 34px default so the header carries the window.
pub const HEIGHT: Pixels = px(48.);
/// Height of the two pills on the right: a small button's, so they read as controls.
pub const PILL: Pixels = px(36.);

#[derive(IntoElement)]
pub struct Topbar {
    /// Name of the game running, if any.
    pub running: Option<SharedString>,
    /// The label and icon of the update button, when a newer lata is out or ready.
    pub update: Option<(SharedString, gpui_kit::assets::IconName)>,
    pub shell: WeakEntity<super::Shell>,
    pub downloads: super::downloads::Pill,
}

impl RenderOnce for Topbar {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();

        let status = self.status(theme);
        let shell = self.shell;
        // The one accent in the bar while it is there: it is the thing to do next.
        let update = self.update.map(|(label, icon)| {
            crate::ui::FancyButton::new("update", label)
                .small()
                .icon(icon)
                .on_click(move |_, window, cx| super::Shell::update_lata(shell.clone(), window, cx))
        });
        TitleBar::new().h(HEIGHT).child(
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_between()
                .pr_3()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        // The brand takes the bar's height, as Modrinth's does: mark and name
                        // read at a glance instead of sitting there as a small label.
                        .child(img(LOGO).size(px(32.)))
                        .child(
                            div()
                                .text_size(px(24.))
                                .font_family(crate::theme::WORDMARK)
                                .font_weight(FontWeight::BOLD)
                                .child("lata"),
                        )
                        // Every piece of lata carries the name plus its own word (the browser
                        // extension is "lata link"); this one is the launcher.
                        .child(
                            div()
                                .pt_1()
                                .text_base()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(cx.theme().muted_foreground)
                                .child("launcher"),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .children(update)
                        .child(discord(cx.theme()))
                        .child(self.downloads)
                        .child(status),
                ),
        )
    }
}

/// Discord's own blue, for its mark only.
const DISCORD: u32 = 0x5865F2;

/// The way into the community, always in sight: where people ask for help and say what broke.
fn discord(theme: &gpui_kit::component::Theme) -> impl IntoElement + use<> {
    let blue = rgb(DISCORD);
    div()
        .id("discord-pill")
        .flex()
        .items_center()
        .gap_2()
        .h(PILL)
        .px_3p5()
        .rounded_full()
        .border_1()
        .border_color(theme.border)
        .text_sm()
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(|style| style.border_color(blue))
        .child(
            Icon::empty()
                .path("icons/discord.svg")
                .size_4()
                .text_color(blue),
        )
        .child("Discord")
        .on_click(|_, _, cx| cx.open_url(super::DISCORD_INVITE))
}

impl Topbar {
    fn status(&self, theme: &gpui_kit::component::Theme) -> impl IntoElement + use<> {
        div()
            .flex()
            .items_center()
            .gap_2()
            .h(PILL)
            .px_3p5()
            .rounded_full()
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(match self.running {
                Some(_) => Icon::empty()
                    .path("icons/gamepad.svg")
                    .size_4()
                    .text_color(theme.primary),
                None => Icon::empty()
                    .path("icons/moon-sleep.svg")
                    .size_4()
                    .text_color(theme.muted_foreground),
            })
            .child(match &self.running {
                Some(name) => format!("Running {name}"),
                None => "Nothing running".into(),
            })
    }
}
