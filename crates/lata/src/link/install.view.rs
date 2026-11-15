//! The dialog that asks where a mod or a collection from a `lata://` link goes: what the
//! Workshop shows of it, the instance to install it into (the one used last is already
//! selected) and a button to confirm. A collection can also become a new instance. A link
//! never installs anything on its own; this is where the user decides.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Found;
use crate::instance::Instance;
use crate::instance::mod_add::{PICTURE, card};
use crate::instance::mods::{self, Bundle};
use crate::shell::Shell;
use crate::ui::{self, FancyButton, Meta};

const WIDTH: f32 = 580.;

pub fn open(
    found: Found,
    instances: Vec<Instance>,
    shell: WeakEntity<Shell>,
    window: &mut Window,
    cx: &mut App,
) {
    let title = match &found {
        Found::Mod(_) => "Install mod",
        Found::Collection(_) => "Install collection",
    };
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
            .title(title)
            .centered()
            .w(px(WIDTH))
            .child(view.clone())
    });
}

struct InstallLink {
    found: Found,
    instances: Vec<Instance>,
    shell: WeakEntity<Shell>,
    chosen: Option<usize>,
    /// Whether the chosen instance already has this mod, so installing it brings its current
    /// version. Never for a collection: its mods are just brought up to date.
    already: bool,
}

/// Whether `instance` has the Workshop item `id`, as lata installed it.
fn has(instance: &Instance, id: u64) -> bool {
    mods::list(instance)
        .iter()
        .any(|installed| installed.workshop == Some(id))
}

impl InstallLink {
    fn select(&mut self, ix: usize) {
        self.chosen = Some(ix);
        self.already = match &self.found {
            Found::Mod(found) => has(&self.instances[ix], found.item.id),
            Found::Collection(_) => false,
        };
    }

    /// The collection's mods made for `instance`'s build line; the others stay out, as when
    /// an instance is created from a collection.
    fn fitting(found: &Bundle, instance: &Instance) -> Vec<u64> {
        let (fit, _) = found.split(mods::is_modern(instance));
        fit.iter().map(|item| item.id).collect()
    }

    fn install(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.chosen else { return };
        let instance = self.instances[ix].clone();
        let found = self.found.clone();
        // The next link offers this instance first.
        crate::settings::set_install_target(&instance.id);
        let (shell, handle) = (self.shell.clone(), window.window_handle());
        window.close_dialog(cx);
        // After this update ends: the shell repaints views as the download starts.
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| match found {
                Found::Mod(found) => shell.add_mod(instance, found.item.id, handle, cx),
                Found::Collection(found) => {
                    let items = Self::fitting(&found, &instance);
                    let name = Some(found.title.into());
                    shell.add_mods(instance, items, name, false, handle, cx)
                }
            });
        });
    }

    /// Opens the create dialog. For a mod with no instance to go into, the shell offers it
    /// again once the instance exists; a collection becomes the new instance itself.
    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (shell, found) = (self.shell.clone(), self.found.clone());
        window.close_dialog(cx);
        // After this update ends, so the create dialog opens once this one is closing.
        window.defer(cx, move |window, cx| match found {
            Found::Mod(found) => {
                let _ = shell.update(cx, |shell, _| shell.link_wait(found));
                Shell::create(shell, window, cx);
            }
            Found::Collection(found) => Shell::create_from(shell, Some(found), window, cx),
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
        // How many of a collection's mods the chosen instance takes.
        let fit = match (&self.found, chosen) {
            (Found::Collection(found), Some(instance)) => {
                Some(Self::fitting(found, instance).len())
            }
            _ => None,
        };
        let ready = chosen.is_some() && fit != Some(0);
        let collection = matches!(self.found, Found::Collection(_));
        let card = match &self.found {
            Found::Mod(found) => card(found, chosen, cx),
            Found::Collection(found) => bundle_card(found, cx),
        };
        let install = match fit {
            Some(1) => "Install 1 mod".into(),
            Some(count) => format!("Install {count} mods").into(),
            None if self.already => SharedString::from("Update"),
            None => "Install".into(),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(ui::enter(card, "link-mod"))
            .when(self.instances.is_empty(), |this| {
                this.child(note(
                    IconName::Box,
                    if collection {
                        "No instances yet. Make one out of this collection.".into()
                    } else {
                        "No instances yet. Make one and this mod goes right in.".into()
                    },
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
            .children(chosen.filter(|_| fit == Some(0)).map(|instance| {
                note(
                    IconName::TriangleAlert,
                    format!(
                        "None of its mods are made for {}'s game version.",
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
                    // A collection can always start an instance of its own instead.
                    .when(collection && !self.instances.is_empty(), |this| {
                        this.child(
                            ui::secondary("link-new", "New instance")
                                .icon(IconName::Plus)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.create(window, cx)),
                                ),
                        )
                    })
                    .when(self.instances.is_empty(), |this| {
                        this.child(
                            FancyButton::new("link-create", "New instance")
                                .small()
                                .icon(IconName::Plus)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.create(window, cx)),
                                ),
                        )
                    })
                    .when(ready, |this| {
                        this.child(
                            FancyButton::new("link-install", install)
                                .small()
                                .icon(IconName::Download)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.install(window, cx)),
                                ),
                        )
                    }),
            )
    }
}

/// The collection as the Workshop describes it, in the mod card's shape: its picture, its name
/// and how many mods it brings.
fn bundle_card(found: &Bundle, cx: &App) -> Div {
    let theme = cx.theme();
    let picture = match &found.picture {
        Some(path) => div().flex_none().size(px(PICTURE)).child(
            img(path.clone())
                .size_full()
                .rounded(theme.radius)
                .object_fit(ObjectFit::Cover),
        ),
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(PICTURE))
            .rounded(theme.radius)
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .child(Icon::new(IconName::Layers).size_8()),
    };
    let count = match found.mods.len() {
        1 => "1 mod".to_owned(),
        many => format!("{many} mods"),
    };
    div()
        .flex()
        .gap_4()
        .p_3()
        .rounded(theme.radius_lg)
        .bg(theme.group_box)
        .border_1()
        .border_color(theme.border)
        .child(picture)
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_lg()
                        .font_weight(FontWeight::BOLD)
                        .truncate()
                        .child(found.title.clone()),
                )
                .child(Meta::new(IconName::Puzzle, count)),
        )
}
