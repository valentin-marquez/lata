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
use crate::instance::mod_add::{SUMMARY, other_build, picture, summary};
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
                    shell.add_mods(
                        instance,
                        items,
                        name,
                        crate::shell::Why::Add,
                        None,
                        handle,
                        cx,
                    )
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

    /// The instance picker, full width like a select: what is chosen, and the others to
    /// choose from.
    fn picker(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let this = cx.entity();
        let chosen = self.chosen;
        let names: Vec<SharedString> = self.instances.iter().map(|i| i.name.clone()).collect();
        let (label, placeholder) = match chosen {
            Some(ix) => (names[ix].clone(), false),
            None => ("Choose an instance".into(), true),
        };
        let muted = theme.muted_foreground;
        Button::new("link-instance")
            .outline()
            .w_full()
            .h(px(40.))
            .cursor_pointer()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .child(Icon::new(IconName::Box).size_4().text_color(muted))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .when(placeholder, |this| this.text_color(muted))
                            .child(label),
                    )
                    .child(Icon::new(IconName::ChevronDown).size_4().text_color(muted)),
            )
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
            })
            .trigger_style(StyleRefinement::default().w_full())
    }
}

impl Render for InstallLink {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let chosen = self.chosen.map(|ix| &self.instances[ix]);
        let note = |icon: IconName, text: String, warn: bool| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(if warn {
                    ui::warning_text(cx)
                } else {
                    theme.muted_foreground
                })
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
        let none = self.instances.is_empty();
        let summary = match &self.found {
            Found::Mod(found) => summary(found, cx),
            Found::Collection(found) => bundle_summary(found, cx),
        };
        let install = match fit {
            Some(1) => "Install 1 mod".into(),
            Some(count) => format!("Install {count} mods").into(),
            None if self.already => SharedString::from("Update mod"),
            None => "Install mod".into(),
        };
        // What the chosen instance means for this mod: already there, the other build line,
        // or nothing of a collection fits.
        let notes: Vec<Div> = match (&self.found, chosen) {
            (Found::Mod(found), Some(instance)) => {
                let other = other_build(&found.item.tags, mods::is_modern(instance));
                [
                    self.already.then(|| {
                        note(
                            IconName::Check,
                            format!(
                                "{} already has it. Installing brings its current version.",
                                instance.name
                            ),
                            false,
                        )
                    }),
                    other.map(|build| {
                        note(
                            IconName::TriangleAlert,
                            format!("Heads up! It's for {build} only, not this instance's"),
                            true,
                        )
                    }),
                ]
                .into_iter()
                .flatten()
                .collect()
            }
            (Found::Collection(_), Some(instance)) if fit == Some(0) => vec![note(
                IconName::TriangleAlert,
                format!(
                    "None of its mods are made for {}'s game version.",
                    instance.name
                ),
                true,
            )],
            _ => Vec::new(),
        };
        let destination = if none {
            note(
                IconName::Box,
                if collection {
                    "No instances yet. Make one out of this collection.".into()
                } else {
                    "No instances yet. Make one and this mod goes right in.".into()
                },
                false,
            )
        } else {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Install in"),
                )
                .child(self.picker(cx))
                .children(notes)
        };
        div()
            .flex()
            .flex_col()
            .gap_6()
            .pt_2()
            .child(ui::enter(summary, "link-mod"))
            .child(destination)
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
                    .when(collection && !none, |this| {
                        this.child(
                            ui::secondary("link-new", "New instance")
                                .icon(IconName::Plus)
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.create(window, cx)),
                                ),
                        )
                    })
                    // The one thing to do without instances is to make one; otherwise Install
                    // is always there, waiting for a destination.
                    .map(|this| {
                        if none {
                            this.child(
                                FancyButton::new("link-create", "New instance")
                                    .small()
                                    .icon(IconName::Plus)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.create(window, cx)),
                                    ),
                            )
                        } else {
                            this.child(
                                FancyButton::new("link-install", install)
                                    .small()
                                    .icon(IconName::Download)
                                    .disabled(!ready)
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.install(window, cx)),
                                    ),
                            )
                        }
                    }),
            )
    }
}

/// The collection in brief, in the mod summary's shape: its picture, its name and how many
/// mods it brings.
fn bundle_summary(found: &Bundle, cx: &App) -> Div {
    let count = match found.mods.len() {
        1 => "1 mod".to_owned(),
        many => format!("{many} mods"),
    };
    div()
        .flex()
        .items_start()
        .gap_4()
        .child(picture(
            found.picture.as_ref(),
            IconName::Layers,
            SUMMARY,
            px(8.),
            cx,
        ))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap_1p5()
                .child(
                    div()
                        .text_base()
                        .font_weight(FontWeight::SEMIBOLD)
                        .line_clamp(2)
                        .text_ellipsis()
                        .child(found.title.clone()),
                )
                .child(Meta::new(IconName::Puzzle, count)),
        )
}
