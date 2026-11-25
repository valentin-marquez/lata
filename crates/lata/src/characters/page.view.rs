//! The Characters page: every shared character as a card, and the editor to make and change
//! them. The lists the editor offers come from one installed game version, picked here.

use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::spring;
use gpui_kit::component::button::Button;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::menu::{ContextMenuExt, DropdownMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::catalog::Catalog;
use super::character::{self, Character};
use super::editor;
use super::presets::Kind;
use crate::instance::Instance;
use crate::ui::{self, Empty, FancyButton, Meta};

const CARD: f32 = 268.;
/// How many traits a card names before it just counts the rest.
const TRAITS: usize = 5;

/// An installed game version the lists can come from.
#[derive(Clone)]
struct Version {
    build: u32,
    game: PathBuf,
    name: String,
}

pub struct Characters {
    /// `None` until the store is read; then every character, or why they couldn't be read.
    characters: Option<Result<Vec<Character>, String>>,
    /// Installed versions, newest first.
    versions: Vec<Version>,
    /// The one the lists come from.
    build: Option<u32>,
    catalog: Option<Arc<Catalog>>,
}

impl Characters {
    pub fn new(instances: Vec<Instance>, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            characters: None,
            versions: Vec::new(),
            build: None,
            catalog: None,
        };
        this.reload(instances, cx);
        this
    }

    /// Reads the shared characters again (a game may have saved some since), and the versions
    /// installed.
    pub fn reload(&mut self, instances: Vec<Instance>, cx: &mut Context<Self>) {
        let mut versions: Vec<Version> = Vec::new();
        for instance in &instances {
            let (Some(build), Some(game)) = (instance.build, instance.game()) else {
                continue;
            };
            if game.is_dir() && !versions.iter().any(|v| v.build == build) {
                let name = instance.build_name().unwrap_or_else(|| build.to_string());
                versions.push(Version { build, game, name });
            }
        }
        versions.sort_by_key(|v| std::cmp::Reverse(v.build));
        self.versions = versions;
        self.read(cx);
        let keep = self
            .build
            .filter(|b| self.versions.iter().any(|v| v.build == *b));
        match keep.or_else(|| self.versions.first().map(|v| v.build)) {
            Some(build) if Some(build) != self.build || self.catalog.is_none() => {
                self.pick(build, cx)
            }
            Some(_) => {}
            None => {
                self.build = None;
                self.catalog = None;
            }
        }
    }

    fn read(&mut self, cx: &mut Context<Self>) {
        let read = super::shared(Kind::Outfits)
            .and_then(|outfits| Ok(character::list(&outfits, &super::shared(Kind::Builds)?)));
        self.characters = Some(read.map_err(|err| err.to_string()));
        cx.notify();
    }

    /// Takes the lists from `build`, read in the background.
    fn pick(&mut self, build: u32, cx: &mut Context<Self>) {
        let Some(game) = self
            .versions
            .iter()
            .find(|v| v.build == build)
            .map(|v| v.game.clone())
        else {
            return;
        };
        self.build = Some(build);
        // No editing with the last version's lists while this one's load.
        self.catalog = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let catalog = crate::runtime::blocking(move || Catalog::load(&game))
                .await
                .ok()
                .flatten();
            let _ = this.update(cx, |this, cx| {
                // Another version was picked meanwhile.
                if this.build == Some(build) {
                    this.catalog = catalog.map(Arc::new);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn all(&self) -> &[Character] {
        match &self.characters {
            Some(Ok(all)) => all,
            _ => &[],
        }
    }

    /// Opens the editor on `character` (a new one when `None`).
    fn edit(&self, character: Option<Character>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        let editing = character.as_ref().map(|c| c.name.clone());
        let taken = self
            .all()
            .iter()
            .map(|c| c.name.clone())
            .filter(|name| Some(name) != editing.as_ref())
            .collect();
        let this = cx.entity().downgrade();
        editor::open(
            character,
            taken,
            catalog,
            move |character, old, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.save(&character, old.as_deref(), window, cx)
                });
            },
            window,
            cx,
        );
    }

    /// Writes `character` into the store, in place of `old`. Read fresh first: a game that
    /// exited since may have saved others.
    fn save(
        &mut self,
        character: &Character,
        old: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let written = super::shared(Kind::Outfits).and_then(|mut outfits| {
            let mut builds = super::shared(Kind::Builds)?;
            character.save(old, &mut outfits, &mut builds);
            super::keep(&outfits)?;
            super::keep(&builds)
        });
        if let Err(err) = written {
            window.push_notification(
                ui::toast::failed_because("Hmm, that character didn't save", err),
                cx,
            );
        }
        self.read(cx);
    }

    fn duplicate(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.read(cx);
        let Some(mut copy) = self.all().iter().find(|c| c.name == name).cloned() else {
            return;
        };
        copy.name = character::unique(name, self.all());
        self.save(&copy, None, window, cx);
    }

    fn delete(&mut self, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let gone = Character {
            name: name.to_owned(),
            look: None,
            build: None,
        };
        self.save(&gone, None, window, cx);
    }

    fn confirm_delete(&self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let (this, name) = (this.clone(), name.clone());
            dialog
                .entrance(DialogEntrance::FadeSlide)
                .title(format!("Delete {name}?"))
                .w(px(420.))
                .child("It goes from every instance too, the next time each one starts.")
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .pt_4()
                        .child(
                            ui::secondary("character-keep", "Keep")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            FancyButton::new("character-delete", "Delete")
                                .small()
                                .danger()
                                .icon(IconName::Trash)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ =
                                        this.update(cx, |this, cx| this.delete(&name, window, cx));
                                }),
                        ),
                )
        });
    }

    /// The version picker, styled like the Library's sort; only with more than one installed.
    fn versions(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.versions.len() < 2 {
            return None;
        }
        let current = self.build;
        let label = self
            .versions
            .iter()
            .find(|v| Some(v.build) == current)
            .map(|v| format!("Lists from {}", v.name))
            .unwrap_or_default();
        let versions = self.versions.clone();
        let this = cx.entity().downgrade();
        Some(
            Button::new("characters-version")
                .outline()
                .h(px(36.))
                .cursor_pointer()
                .icon(IconName::Layers)
                .label(label)
                .dropdown_menu(move |menu, _, _| {
                    versions.iter().fold(menu, |menu, version| {
                        let (this, build) = (this.clone(), version.build);
                        menu.item(
                            PopupMenuItem::new(version.name.clone())
                                .checked(current == Some(build))
                                .on_click(move |_, _, cx| {
                                    let _ = this.update(cx, |this, cx| this.pick(build, cx));
                                }),
                        )
                    })
                }),
        )
    }

    fn card(
        &self,
        character: &Character,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let key = character.name.clone();
        let hovered = window.use_keyed_state(
            SharedString::from(format!("character-hover-{key}")),
            cx,
            |_, _| false,
        );
        let surface: f32 = spring(
            SharedString::from(format!("character-surface-{key}")),
            if *hovered.read(cx) { 1. } else { 0. },
            ui::SPRING_FAST,
            window,
            cx,
        );
        let pressed = window.use_keyed_state(
            SharedString::from(format!("character-pressed-{key}")),
            cx,
            |_, _| false,
        );
        // Pressing shrinks the face to 98%, like the Library's cards.
        let press: f32 = spring(
            SharedString::from(format!("character-press-{key}")),
            if *pressed.read(cx) { 1. } else { 0. },
            ui::SPRING_PRESS,
            window,
            cx,
        );
        let catalog = self.catalog.clone();
        let look = character.look.as_ref();
        let full_name = look
            .map(|l| {
                let (first, last) = l.names();
                format!("{first} {last}").trim().to_owned()
            })
            .filter(|n| !n.is_empty() && *n != character.name);
        let swatches = div()
            .flex()
            .flex_none()
            .gap_1()
            .children(
                look.and_then(|l| l.skin())
                    .map(|c| editor::dot(c, 16., theme.border)),
            )
            .children(
                look.and_then(|l| l.hair().1)
                    .map(|c| editor::dot(c, 16., theme.border)),
            );
        let badge = |text: SharedString| {
            div()
                .px_1p5()
                .py_0p5()
                .rounded(px(6.))
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .child(text)
        };
        let profession = character.build.as_ref().map(|build| {
            let name = catalog
                .as_ref()
                .and_then(|c| c.profession(&build.profession))
                .map_or(build.profession.clone(), |p| p.name.clone());
            Meta::new(IconName::Briefcase, name)
        });
        let traits = character.build.as_ref().map(|build| {
            let names = build.traits.iter().take(TRAITS).map(|id| {
                let name = catalog
                    .as_ref()
                    .and_then(|c| c.trait_(id))
                    .map_or(id.clone(), |t| t.name.clone());
                badge(name.into())
            });
            let more = build.traits.len().saturating_sub(TRAITS);
            div()
                .flex()
                .flex_wrap()
                .gap_1()
                .children(names)
                .when(more > 0, |this| {
                    this.child(badge(format!("+{more}").into()))
                })
        });
        let pieces = look.map(|l| l.pieces().len()).unwrap_or(0);
        let clothes = (pieces > 0).then(|| {
            Meta::new(
                IconName::Shirt,
                match pieces {
                    1 => "1 piece of clothing".to_owned(),
                    n => format!("{n} pieces of clothing"),
                },
            )
        });
        let (inset_x, inset_y) = (px(CARD * 0.01 * press), px(2. * press));
        let (edit, copy, gone) = (character.clone(), key.clone(), key.clone());
        let editable = self.catalog.is_some();
        let danger = theme.danger;
        let this = cx.entity().downgrade();
        div()
            .id(SharedString::from(format!("character-{key}")))
            .relative()
            .flex()
            .flex_col()
            .gap_3()
            .w(px(CARD))
            .p_4()
            .when(editable, |this| this.cursor_pointer())
            .on_hover(move |is_hovered, window, cx| {
                hovered.update(cx, |h, _| *h = *is_hovered);
                window.refresh();
            })
            .on_mouse_down(MouseButton::Left, {
                let pressed = pressed.clone();
                move |_, window, cx| {
                    pressed.update(cx, |p, _| *p = true);
                    window.refresh();
                }
            })
            .on_mouse_up(MouseButton::Left, {
                let pressed = pressed.clone();
                move |_, window, cx| {
                    pressed.update(cx, |p, _| *p = false);
                    window.refresh();
                }
            })
            .on_mouse_up_out(MouseButton::Left, move |_, window, cx| {
                pressed.update(cx, |p, _| *p = false);
                window.refresh();
            })
            .child(
                div()
                    .absolute()
                    .top(inset_y)
                    .bottom(inset_y)
                    .left(inset_x)
                    .right(inset_x)
                    .rounded(theme.radius_lg)
                    .bg(ui::mix(
                        theme.group_box,
                        ui::hovered(theme.group_box, cx),
                        surface,
                    ))
                    .border_1()
                    .border_color(theme.border),
            )
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
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
                                    .child(character.name.clone()),
                            )
                            .children(full_name.map(|name| {
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(name)
                            })),
                    )
                    .child(swatches),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .children(profession)
                    .children(clothes),
            )
            .children(traits)
            .on_click(cx.listener({
                let edit = edit.clone();
                move |this, _, window, cx| this.edit(Some(edit.clone()), window, cx)
            }))
            .context_menu(move |menu, _, _| {
                let on = |run: fn(&mut Characters, &str, &mut Window, &mut Context<Characters>),
                          name: String| {
                    let this = this.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        let _ = this.update(cx, |this, cx| run(this, &name, window, cx));
                    }
                };
                let editing = edit_item(this.clone(), edit.clone());
                menu.item(editing.disabled(!editable))
                    .item(
                        PopupMenuItem::new("Duplicate")
                            .icon(IconName::Copy)
                            .on_click(on(Characters::duplicate, copy.clone())),
                    )
                    .separator()
                    .item(
                        ui::danger_item("Delete", IconName::Trash, danger).on_click({
                            let (this, name) = (this.clone(), gone.clone());
                            move |_, window, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.confirm_delete(name.clone(), window, cx)
                                });
                            }
                        }),
                    )
            })
            .into_any_element()
    }
}

fn edit_item(this: WeakEntity<Characters>, character: Character) -> PopupMenuItem {
    PopupMenuItem::new("Edit")
        .icon(IconName::Pencil)
        .on_click(move |_, window, cx| {
            let _ = this.update(cx, |this, cx| {
                this.edit(Some(character.clone()), window, cx)
            });
        })
}

impl Render for Characters {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let count = self.all().len();
        let editable = self.catalog.is_some();
        let header = div()
            .flex()
            .items_center()
            .gap_3()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child("Characters"),
            )
            .when(count > 0, |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(count.to_string()),
                )
            })
            .child(div().flex_1())
            .children(self.versions(cx))
            .child(
                FancyButton::new("characters-new", "New character")
                    .small()
                    .icon(IconName::Plus)
                    .disabled(!editable)
                    .on_click(cx.listener(|this, _, window, cx| this.edit(None, window, cx))),
            );
        let about = div()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(if self.versions.is_empty() {
                "Every instance shares these. Install a game version to make and edit them here."
            } else {
                "Every instance shares these: save one in any game, or make it here, and it shows up on the character screen of all of them."
            });
        let body = match &self.characters {
            None => div(),
            Some(Err(why)) => div().flex().flex_1().child(Empty::new(
                IconName::TriangleAlert,
                "Couldn't read your characters",
                SharedString::from(why.clone()),
            )),
            Some(Ok(all)) if all.is_empty() => div().flex().flex_1().child(Empty::new(
                IconName::UserRound,
                "No characters yet",
                "Make one here, or save one on the game's character screen.",
            )),
            Some(Ok(all)) => {
                let all = all.clone();
                let cards: Vec<AnyElement> = all
                    .iter()
                    .map(|character| self.card(character, window, cx))
                    .collect();
                div()
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap_4()
                    .children(cards)
            }
        };
        div()
            .id("characters")
            .flex()
            .flex_col()
            .size_full()
            .gap_5()
            .p_6()
            .overflow_y_scroll()
            .child(div().flex().flex_col().gap_1().child(header).child(about))
            .child(ui::enter(body, "characters-body"))
    }
}
