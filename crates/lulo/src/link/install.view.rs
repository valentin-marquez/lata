//! The dialog that asks where a mod from a `lulo://` link goes: the mod as the Workshop shows
//! it, the instance to install it into (the one used last is already selected) and a button to
//! confirm. A link never installs anything on its own; this is where the user decides.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::instance::Instance;
use crate::instance::mod_add::card;
use crate::instance::mods::{self, Listing};
use crate::shell::Shell;
use crate::ui::{self, FancyButton};

const WIDTH: f32 = 580.;

pub fn open(
    found: Listing,
    instances: Vec<Instance>,
    shell: WeakEntity<Shell>,
    window: &mut Window,
    cx: &mut App,
) {
    let chosen = super::preselect(&instances, crate::settings::install_target().as_deref());
    let handle = window.window_handle();
    let view = cx.new(|_| {
        let mut view = InstallLink {
            found,
            instances,
            shell: shell.clone(),
            chosen: None,
            already: false,
        };
        if let Some(ix) = chosen {
            view.select(ix);
        }
        view
    });
    // However the dialog closes (installed, cancelled, Escape, outside click), the next link
    // that was waiting gets its turn.
    cx.observe_release(&view, move |_, cx| {
        let _ = shell.update(cx, |shell, cx| shell.link_closed(handle, cx));
    })
    .detach();
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title("Install mod")
            .centered()
            .w(px(WIDTH))
            .child(view.clone())
    });
}

struct InstallLink {
    found: Listing,
    instances: Vec<Instance>,
    shell: WeakEntity<Shell>,
    chosen: Option<usize>,
    /// Whether the chosen instance already has this mod, so installing it brings its current
    /// version.
    already: bool,
}

/// Whether `instance` has the Workshop item `id`, as lulo installed it.
fn has(instance: &Instance, id: u64) -> bool {
    mods::list(instance)
        .iter()
        .any(|installed| installed.workshop == Some(id))
}

impl InstallLink {
    fn select(&mut self, ix: usize) {
        self.chosen = Some(ix);
        self.already = has(&self.instances[ix], self.found.item.id);
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.chosen else { return };
        let instance = self.instances[ix].clone();
        let id = self.found.item.id;
        // The next link offers this instance first.
        crate::settings::set_install_target(&instance.id);
        let (shell, handle) = (self.shell.clone(), window.window_handle());
        window.close_dialog(cx);
        // After this update ends: the shell repaints views as the download starts.
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| shell.add_mod(instance, id, handle, cx));
        });
    }

    /// The instance picker: what is chosen, and the others to choose from.
    fn picker(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();
        let chosen = self.chosen;
        let names: Vec<SharedString> = self.instances.iter().map(|i| i.name.clone()).collect();
        let label = chosen.map_or("Choose an instance".into(), |ix| names[ix].clone());
        div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("Install in"),
            )
            .child(
                Button::new("link-instance")
                    .outline()
                    .flex_1()
                    .h(px(36.))
                    .cursor_pointer()
                    .icon(IconName::Box)
                    .label(label)
                    .dropdown_menu(move |mut menu, _, _| {
                        for (ix, name) in names.iter().enumerate() {
                            let this = this.clone();
                            menu = menu.item(
                                PopupMenuItem::new(name.clone())
                                    .checked(chosen == Some(ix))
                                    .on_click(move |_, _, cx| {
                                        this.update(cx, |view, cx| {
                                            view.select(ix);
                                            cx.notify();
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
    }
}

impl Render for InstallLink {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let chosen = self.chosen.map(|ix| &self.instances[ix]);
        let note = |icon: IconName, text: String| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(Icon::new(icon).size_4().flex_none())
                .child(text)
        };
        let ready = chosen.is_some();
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(ui::enter(card(&self.found, chosen, cx), "link-mod"))
            .when(self.instances.is_empty(), |this| {
                this.child(note(
                    IconName::Box,
                    "There is no instance yet. Create one, then open the link again.".into(),
                ))
            })
            .when(!self.instances.is_empty(), |this| {
                this.child(self.picker(cx))
            })
            .children(chosen.filter(|_| self.already).map(|instance| {
                note(
                    IconName::Check,
                    format!(
                        "{} already has it. Installing brings its current version.",
                        instance.name
                    ),
                )
            }))
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        ui::secondary("link-cancel", "Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .when(ready, |this| {
                        this.child(
                            FancyButton::new(
                                "link-install",
                                if self.already { "Update" } else { "Install" },
                            )
                            .small()
                            .icon(IconName::Download)
                            .on_click(cx.listener(|this, _, window, cx| this.install(window, cx))),
                        )
                    }),
            )
    }
}
