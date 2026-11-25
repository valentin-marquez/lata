//! The character editor: a page in place of the Characters grid. The character stands large
//! in the middle, and the shell's right panel holds what changes it, a section per part of the
//! game's character screen. Its lists come from one installed build ([`Catalog`]).

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon as Glyph, IndexPath, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::catalog::{Catalog, Choice};
use super::character::{self, Build, Character, Look, Rgb};
use super::page::Characters;
use super::preview::Preview;
use crate::ui::{self, FancyButton};

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Appearance,
    Clothes,
    Build,
}

impl Section {
    const ALL: [Section; 3] = [Section::Appearance, Section::Clothes, Section::Build];

    fn label(self) -> &'static str {
        match self {
            Section::Appearance => t!("Appearance"),
            Section::Clothes => t!("Clothes"),
            // The game asks for traits on its profession screen too; "Build" already names a
            // game build in the translations.
            Section::Build => t!("Profession"),
        }
    }
}

/// What a dropdown row tells the editor when the mouse enters or leaves it: its id, and which.
type Hover = Rc<dyn Fn(&str, bool, &mut App)>;

/// An entry in one of the editor's dropdowns.
#[derive(Clone)]
struct Pick {
    id: String,
    name: SharedString,
    /// Told when the mouse enters or leaves this row, with its id: the editor shows it before
    /// it is chosen. Only the hair list has one.
    hover: Option<Hover>,
    /// Points it gives (a profession), shown beside it.
    cost: Option<i32>,
}

impl SearchableListItem for Pick {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn tracks_hover(&self) -> bool {
        self.hover.is_some()
    }

    // The whole row, padding included (vendor/gpui-component), not only the name drawn below.
    fn on_hover(&self, hovered: bool, cx: &mut App) {
        if let Some(hover) = &self.hover {
            hover(&self.id, hovered, cx);
        }
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .flex()
            .w_full()
            .justify_between()
            .gap_4()
            .child(self.name.clone())
            .children(self.cost.map(|cost| {
                div()
                    .text_color(points_color(cost, theme))
                    .child(points(cost))
            }))
    }

    fn value(&self) -> &String {
        &self.id
    }
}

/// Which dropdown an event came from.
#[derive(Clone, Copy)]
enum Which {
    Hair,
    Beard,
    Profession,
}

pub struct Editor {
    characters: WeakEntity<Characters>,
    catalog: Arc<Catalog>,
    section: Section,
    /// The name it had when the editor opened; `None` for a new character.
    old: Option<String>,
    /// What it was when the editor opened, to tell whether anything changed since.
    original: Character,
    taken: Vec<String>,
    name: Entity<InputState>,
    first: Entity<InputState>,
    last: Entity<InputState>,
    look: Option<Look>,
    build: Option<Build>,
    hair: Option<Entity<SelectState<Vec<Pick>>>>,
    /// The hair style the mouse is over in the open list, shown in the preview without being
    /// chosen. It goes away when the list closes.
    hover_hair: Option<String>,
    beard: Option<Entity<SelectState<Vec<Pick>>>>,
    profession: Option<Entity<SelectState<Vec<Pick>>>>,
    preview: Entity<Preview>,
    /// Why the last Save didn't go through; the draft stays for another try.
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl Editor {
    /// An editor on `character`, or on a new one when it is `None`. `taken` are the names of
    /// the others, which this one can't take.
    pub fn new(
        characters: WeakEntity<Characters>,
        character: Option<Character>,
        taken: Vec<String>,
        catalog: Arc<Catalog>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fresh = character.is_none();
        let character = character.unwrap_or_else(|| Character {
            name: String::new(),
            look: Some(first_look(&catalog)),
            build: Some(Build {
                profession: catalog
                    .professions
                    .first()
                    .map(|p| p.id.clone())
                    .unwrap_or_default(),
                traits: Vec::new(),
            }),
        });
        let (first, last) = character.look.as_ref().map(Look::names).unwrap_or_default();
        let input =
            |value: String, placeholder: &'static str, window: &mut Window, cx: &mut App| {
                cx.new(|cx| {
                    InputState::new(window, cx)
                        .placeholder(placeholder)
                        .default_value(value)
                })
            };
        // As tall as the page leaves it at the window's size when it opens: a portrait, about
        // three wide to four tall.
        // ponytail: fixed once made; a window resized while editing crops or pads it.
        let tall = (window.viewport_size().height - px(230.)).clamp(px(360.), px(900.));
        let preview = cx.new(|cx| {
            Preview::new(
                catalog.media.clone(),
                catalog.skins.clone(),
                size(tall * 0.75, tall),
                cx,
            )
        });
        let mut editor = Self {
            characters,
            name: input(character.name.clone(), t!("Kate the nurse"), window, cx),
            first: input(first, t!("Kate"), window, cx),
            last: input(last, t!("Smith"), window, cx),
            catalog,
            section: Section::Appearance,
            old: (!fresh).then(|| character.name.clone()),
            original: character.clone(),
            taken,
            look: character.look,
            build: character.build,
            hair: None,
            hover_hair: None,
            beard: None,
            profession: None,
            preview,
            error: None,
            _subscriptions: Vec::new(),
        };
        // Compared the way Save would write it, so opening and closing isn't a change.
        editor.original = editor.current(cx);
        editor.lists(window, cx);
        editor
    }

    /// Makes the dropdowns for what the character is now: the hair styles depend on gender.
    fn lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.clear();
        self.hover_hair = None;
        let catalog = self.catalog.clone();
        let look = self
            .look
            .as_ref()
            .map(|l| (l.female(), l.hair().0, l.beard().to_owned()));
        let profession = self.build.as_ref().map(|b| b.profession.clone());
        self.hair = look.as_ref().map(|(female, hair, _)| {
            self.select(Which::Hair, catalog.hair(*female), hair, false, window, cx)
        });
        self.beard = look
            .as_ref()
            .filter(|(female, _, _)| !female)
            .map(|(_, _, beard)| {
                self.select(Which::Beard, &catalog.beards, beard, false, window, cx)
            });
        self.profession = profession.map(|current| {
            self.select(
                Which::Profession,
                &catalog.professions,
                &current,
                true,
                window,
                cx,
            )
        });
    }

    fn select(
        &mut self,
        which: Which,
        list: &[Choice],
        current: &str,
        costs: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SelectState<Vec<Pick>>> {
        // Only the hair styles are previewed under the mouse: the others change what the
        // preview doesn't show.
        let hover: Option<Hover> = matches!(which, Which::Hair).then(|| {
            let editor = cx.entity().downgrade();
            Rc::new(move |id: &str, entered: bool, cx: &mut App| {
                // Later, not now: the list can say so while the editor is being drawn.
                let (editor, id) = (editor.clone(), id.to_owned());
                cx.defer(move |cx| {
                    let _ = editor.update(cx, |this, cx| {
                        if entered {
                            this.hover_hair = Some(id);
                        } else if this.hover_hair.as_deref() == Some(id.as_str()) {
                            this.hover_hair = None;
                        }
                        cx.notify();
                    });
                });
            }) as Hover
        });
        let mut list: Vec<&Choice> = list.iter().collect();
        if costs {
            // By the number shown, the most points first; the order it came in breaks ties.
            list.sort_by_key(|c| std::cmp::Reverse(c.cost));
        }
        let picks: Vec<Pick> = list
            .into_iter()
            .map(|choice| Pick {
                id: choice.id.clone(),
                name: choice.name.clone().into(),
                hover: hover.clone(),
                cost: costs.then_some(choice.cost),
            })
            .collect();
        let at = picks
            .iter()
            .position(|p| p.id.eq_ignore_ascii_case(current))
            .map(IndexPath::new);
        let state = cx.new(|cx| SelectState::new(picks, at, window, cx).searchable(true));
        self._subscriptions.push(cx.subscribe_in(
            &state,
            window,
            move |this, _, event: &SelectEvent<Vec<Pick>>, _, cx| {
                if let SelectEvent::Confirm(Some(id)) = event {
                    this.picked(which, id.clone());
                    cx.notify();
                }
            },
        ));
        // However the list closes (a pick, Escape, a click outside), what was only previewed
        // goes back to what is chosen.
        if matches!(which, Which::Hair) {
            self._subscriptions.push(cx.subscribe_in(
                &state,
                window,
                |this, _, _: &DismissEvent, _, cx| {
                    this.hover_hair = None;
                    cx.notify();
                },
            ));
        }
        state
    }

    fn picked(&mut self, which: Which, id: String) {
        match which {
            Which::Hair => {
                // What was only previewed is the choice now.
                self.hover_hair = None;
                if let Some(look) = &mut self.look {
                    let color = look.hair().1.unwrap_or([0., 0., 0.]);
                    look.set_hair(&id, color);
                }
            }
            Which::Beard => {
                if let Some(look) = &mut self.look {
                    look.set_beard(&id);
                }
            }
            Which::Profession => {
                if let Some(build) = &mut self.build {
                    build.profession = id;
                }
                // The game drops the traits the new profession's own rule out.
                let ruled = self.ruled_out();
                if let Some(build) = &mut self.build {
                    build
                        .traits
                        .retain(|t| !ruled.iter().any(|r| r.eq_ignore_ascii_case(t)));
                }
            }
        }
    }

    /// The name typed, cleaned, and why it can't be saved under it, if it can't. An empty
    /// name only keeps Save off: a new character starts without one, and that isn't a mistake.
    fn name(&self, cx: &App) -> (String, Option<&'static str>) {
        let name = character::clean(&self.name.read(cx).value());
        let problem = if name.is_empty() {
            Some("")
        } else if self.taken.contains(&name) {
            Some(t!("Another character already has that name"))
        } else {
            None
        };
        (name, problem)
    }

    /// The character as Save would write it now.
    fn current(&self, cx: &App) -> Character {
        let mut look = self.look.clone();
        if let Some(look) = &mut look {
            look.set_names(&self.first.read(cx).value(), &self.last.read(cx).value());
        }
        Character {
            name: self.name(cx).0,
            look,
            build: self.build.clone(),
        }
    }

    fn dirty(&self, cx: &App) -> bool {
        self.current(cx) != self.original
    }

    /// Points left: the profession's, minus what the traits cost. The game won't start a
    /// character below zero.
    fn points(&self) -> Option<i32> {
        let build = self.build.as_ref()?;
        let profession = self.catalog.profession(&build.profession)?.cost;
        let traits: i32 = build
            .traits
            .iter()
            .filter_map(|t| self.catalog.trait_(t))
            .map(|t| t.cost)
            .sum();
        Some(profession - traits)
    }

    /// Writes the character. The page closes the editor only once it is written; when it
    /// isn't, the editor stays open with the draft and says why.
    fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.name(cx).1.is_some() {
            return false;
        }
        let character = self.current(cx);
        let old = self.old.clone();
        let written = self
            .characters
            .update(cx, |characters, cx| {
                characters.commit(&character, old.as_deref(), cx)
            })
            .map_err(|err| err.to_string())
            .and_then(|written| written);
        match written {
            Ok(()) => true,
            Err(why) => {
                self.error = Some(why);
                cx.notify();
                false
            }
        }
    }

    fn appearance(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let Some(look) = &self.look else {
            return self.no_look(cx);
        };
        let female = look.female();
        let genders = div().flex().gap_2().children(
            [("Female", t!("Female"), true), ("Male", t!("Male"), false)].map(
                |(id, label, value)| {
                    chip(
                        SharedString::from(id),
                        SharedString::from(label),
                        female == value,
                        &theme,
                        cx,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if let Some(look) = &mut this.look
                            && look.female() != value
                        {
                            look.set_female(value);
                            // A style only the other gender has would load as nothing.
                            let (style, color) = look.hair();
                            let styles = this.catalog.hair(value);
                            if !styles.iter().any(|s| s.id == style)
                                && let Some(first) = styles.first()
                            {
                                look.set_hair(&first.id, color.unwrap_or([0., 0., 0.]));
                            }
                            this.lists(window, cx);
                            cx.notify();
                        }
                    }))
                },
            ),
        );
        let skin = look.skin();
        let skins = swatches(
            "skin",
            &self.catalog.skins,
            skin,
            24.,
            &theme,
            cx,
            |look, color| look.set_skin(color),
        );
        let (style, hair) = look.hair();
        let hair_colors = swatches(
            "hair-color",
            &self.catalog.hair_colors,
            hair,
            20.,
            &theme,
            cx,
            move |look, color| look.set_hair(&style, color),
        );
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(div().flex_1().min_w_0().child(field(
                        t!("First name"),
                        "",
                        Input::new(&self.first),
                        cx,
                    )))
                    .child(div().flex_1().min_w_0().child(field(
                        t!("Last name"),
                        "",
                        Input::new(&self.last),
                        cx,
                    ))),
            )
            .child(field(t!("Gender"), "", genders, cx))
            .child(field(t!("Skin"), "", skins, cx))
            .child(field(
                t!("Hair"),
                "",
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .children(self.hair.as_ref().map(|state| {
                        Select::new(state)
                            .menu_max_h(px(280.))
                            .accessibility_label(t!("Hair style"))
                    }))
                    .child(hair_colors),
                cx,
            ))
            .children(self.beard.as_ref().map(|state| {
                field(
                    t!("Beard"),
                    "",
                    Select::new(state)
                        .menu_max_h(px(280.))
                        .accessibility_label(t!("Beard")),
                    cx,
                )
            }))
    }

    fn no_look(&self, cx: &mut Context<Self>) -> Div {
        missing(
            t!("No look saved under this name, just a profession and traits."),
            t!("Add a look"),
            cx.listener(|this, _, window, cx| {
                this.look = Some(first_look(&this.catalog));
                this.lists(window, cx);
                cx.notify();
            }),
            cx,
        )
    }

    fn clothes(&self, cx: &mut Context<Self>) -> Div {
        let Some(look) = &self.look else {
            return self.no_look(cx);
        };
        let theme = cx.theme();
        let hover = ui::hovered(theme.title_bar, cx);
        let rows = look.pieces().into_iter().enumerate().map(|(ix, piece)| {
            div()
                .id(("piece", ix))
                .flex()
                .items_center()
                .gap_3()
                .px_2()
                .py_2()
                .rounded(theme.radius)
                .hover(|s| s.bg(hover))
                .child(match piece.tint {
                    Some(tint) => dot(tint, 14., theme.border),
                    None => div().flex_none().size(px(14.)),
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .truncate()
                                .text_sm()
                                .child(self.catalog.item(&piece.item)),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(piece.slot),
                        ),
                )
        });
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("Clothes are picked in the game for now. Change them on its character screen, save the look, and it shows up here.")),
            )
            .child(div().flex().flex_col().children(rows))
    }

    fn build(&self, cx: &mut Context<Self>) -> Div {
        let (Some(build), Some(state)) = (&self.build, &self.profession) else {
            return missing(
                t!("No profession or traits saved under this name, just a look."),
                t!("Add a profession"),
                cx.listener(|this, _, window, cx| {
                    this.build = Some(Build {
                        profession: this
                            .catalog
                            .professions
                            .first()
                            .map(|p| p.id.clone())
                            .unwrap_or_default(),
                        traits: Vec::new(),
                    });
                    this.lists(window, cx);
                    cx.notify();
                }),
                cx,
            );
        };
        let theme = cx.theme().clone();
        let about = self
            .catalog
            .profession(&build.profession)
            .is_none()
            .then(|| {
                div().text_xs().text_color(ui::warning_text(cx)).child(t!(
                    "\"{profession}\" isn't in this game version, maybe it comes from a mod.",
                    profession = build.profession
                ))
            });
        let profession = field(
            t!("Profession"),
            t!("Some give you points to spend on traits, some cost them"),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    Select::new(state)
                        .menu_max_h(px(320.))
                        .accessibility_label(t!("Profession")),
                )
                .children(about),
            cx,
        );
        let has = |id: &str| build.traits.iter().any(|t| t.eq_ignore_ascii_case(id));
        let ruled = self.ruled_out();
        let free = |id: &str| !ruled.iter().any(|r| r.eq_ignore_ascii_case(id));
        let group = |title: &'static str, good: bool, cx: &mut Context<Self>| {
            let mut shown: Vec<_> = self
                .catalog
                .traits
                .iter()
                .filter(|t| (t.cost >= 0) == good)
                .collect();
            // Cheapest first, as the game lists them; the order it came in breaks ties.
            shown.sort_by_key(|t| t.cost.abs());
            let rows = shown
                .into_iter()
                .map(|t| {
                    let id = t.id.clone();
                    let row = trait_row(
                        SharedString::from(format!("trait-{id}")),
                        t.name.clone().into(),
                        Some(-t.cost),
                        has(&id),
                        free(&id),
                        &theme,
                        cx,
                    );
                    if free(&id) {
                        row.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle(&id);
                            cx.notify();
                        }))
                    } else {
                        row
                    }
                })
                .collect::<Vec<_>>();
            field(title, "", div().flex().flex_col().children(rows), cx)
        };
        // Saved by another version or a mod: shown so they can be dropped.
        let unknown: Vec<String> = build
            .traits
            .iter()
            .filter(|t| self.catalog.trait_(t).is_none())
            .cloned()
            .collect();
        let others = (!unknown.is_empty()).then(|| {
            field(
                t!("Not in this game version"),
                t!("From another version or a mod. Click one to drop it."),
                div()
                    .flex()
                    .flex_col()
                    .children(unknown.into_iter().map(|id| {
                        trait_row(
                            SharedString::from(format!("unknown-{id}")),
                            id.clone().into(),
                            None,
                            true,
                            true,
                            &theme,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle(&id);
                            cx.notify();
                        }))
                    })),
                cx,
            )
        });
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(profession)
            .child(group(t!("Good traits"), true, cx))
            .child(group(t!("Bad traits"), false, cx))
            .children(others)
    }

    /// Points left, kept in view above the traits while they scroll.
    fn balance(&self, cx: &mut Context<Self>) -> Option<Div> {
        let theme = cx.theme().clone();
        self.points().map(|left| {
            let (text, color) = if left < 0 {
                (
                    tn!(
                        -left,
                        "{n} points short, the game won't start it like this",
                        "{n} points short, the game won't start it like this"
                    ),
                    ui::warning_text(cx),
                )
            } else {
                (
                    tn!(left, "{n} point left", "{n} points left"),
                    theme.foreground,
                )
            };
            div()
                .px_3()
                .py_2()
                .rounded(theme.radius)
                .bg(theme.muted)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .text_color(color)
                .child(text)
        })
    }

    /// Traits the profession's own traits rule out (a veteran can't be brave).
    fn ruled_out(&self) -> Vec<String> {
        self.build
            .as_ref()
            .and_then(|b| self.catalog.profession(&b.profession))
            .map(|p| p.exclusive.clone())
            .unwrap_or_default()
    }

    /// Takes or drops a trait; taking one drops those it can't go with.
    fn toggle(&mut self, id: &str) {
        let Some(build) = &mut self.build else { return };
        let before = build.traits.len();
        build.traits.retain(|t| !t.eq_ignore_ascii_case(id));
        if build.traits.len() != before {
            return;
        }
        if let Some(chosen) = self.catalog.trait_(id) {
            build
                .traits
                .retain(|t| !chosen.exclusive.iter().any(|e| e.eq_ignore_ascii_case(t)));
        }
        build.traits.push(id.to_owned());
    }

    /// The shell's right panel while this editor is open: the sections and their controls.
    fn panel(&self, cx: &mut Context<Self>) -> Div {
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::tab::{Tab, TabBar};
        let selected = Section::ALL
            .iter()
            .position(|s| *s == self.section)
            .unwrap_or(0);
        let tabs = TabBar::new("character-sections")
            .pill()
            .w_full()
            .selected_index(selected)
            .children(Section::ALL.map(|s| Tab::new().label(s.label()).flex_1().cursor_pointer()))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.section = Section::ALL[*ix];
                cx.notify();
            }))
            .small();
        let content = match self.section {
            Section::Appearance => self.appearance(cx),
            Section::Clothes => self.clothes(cx),
            Section::Build => self.build(cx),
        };
        div()
            .flex()
            .flex_col()
            .flex_none()
            .w(crate::discover::WIDTH)
            .h_full()
            .p_3()
            .gap_3()
            .child(ui::tab_tray(tabs, cx))
            .when(self.section == Section::Build, |this| {
                this.children(self.balance(cx))
            })
            .child(
                div()
                    .id("character-section")
                    .flex_1()
                    .min_h_0()
                    .px_1()
                    .pb_3()
                    .overflow_y_scroll()
                    .child(ui::enter(content, self.section.label())),
            )
    }
}

/// Leaves the editor, then runs `then`. With changes not saved yet it asks first: Save,
/// Discard or Keep editing. Always runs after the current event, so it can be called from
/// inside any view's handler.
pub fn leave(
    editor: Entity<Editor>,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    window.defer(cx, move |window, cx| {
        if !editor.read(cx).dirty(cx) {
            close(&editor, cx);
            then(window, cx);
            return;
        }
        // The dialog is built again on every frame; whichever button wins takes `then`.
        let then = Rc::new(RefCell::new(Some(then)));
        window.open_dialog(cx, move |dialog, _, _| {
            let (save_editor, discard_editor) = (editor.clone(), editor.clone());
            let (save_then, discard_then) = (then.clone(), then.clone());
            dialog
                .entrance(DialogEntrance::FadeSlide)
                .title(t!("Save your changes?"))
                .w(px(440.))
                .child(t!(
                    "This character has changes that aren't saved yet. Leaving without saving drops them."
                ))
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .pt_4()
                        .child(
                            ui::secondary("character-keep-editing", t!("Keep editing"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            ui::secondary("character-discard", t!("Discard"))
                                .icon(IconName::Trash)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    close(&discard_editor, cx);
                                    if let Some(then) = discard_then.borrow_mut().take() {
                                        then(window, cx);
                                    }
                                }),
                        )
                        .child(
                            FancyButton::new("character-save-leave", t!("Save"))
                                .small()
                                .icon(IconName::Check)
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    if save_editor.update(cx, |editor, cx| editor.save(cx))
                                        && let Some(then) = save_then.borrow_mut().take()
                                    {
                                        then(window, cx);
                                    }
                                }),
                        ),
                )
        });
    });
}

/// Back to the grid, dropping the draft.
fn close(editor: &Entity<Editor>, cx: &mut App) {
    let characters = editor.read(cx).characters.clone();
    let _ = characters.update(cx, |characters, cx| characters.close(cx));
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The preview shows the hair style under the mouse in the open list, as the pick would
        // set it (same color), without choosing it.
        let mut look = self.look.clone();
        if let (Some(look), Some(style)) = (&mut look, &self.hover_hair) {
            let color = look.hair().1.unwrap_or([0., 0., 0.]);
            look.set_hair(style, color);
        }
        self.preview.update(cx, |p, _| p.show(look.as_ref()));
        let theme = cx.theme().clone();
        let (_, problem) = self.name(cx);
        let me = cx.entity();
        let leave_now = move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
            leave(me.clone(), |_, _| {}, window, cx)
        };
        let back = Button::new("character-back")
            .ghost()
            .cursor_pointer()
            .icon(IconName::ChevronLeft)
            .label(t!("Characters"))
            .on_click(leave_now.clone());
        let save = FancyButton::new(
            "character-save",
            if self.old.is_some() {
                t!("Save")
            } else {
                t!("Create character")
            },
        )
        .small()
        .icon(IconName::Check)
        .disabled(problem.is_some())
        .on_click(cx.listener(|this, _, _, cx| {
            this.error = None;
            this.save(cx);
        }));
        let actions = div()
            .flex()
            .items_center()
            .gap_2()
            .child(ui::secondary("character-cancel", t!("Cancel")).on_click(leave_now))
            .child(
                // The wrapper carries the tooltip: the button itself is off while it applies.
                div()
                    .id("character-save-tip")
                    .when(problem == Some(""), |this| {
                        this.tooltip(|window, cx| {
                            Tooltip::new(t!("Give the character a name first")).build(window, cx)
                        })
                    })
                    .child(save),
            );
        let note = match (&self.error, problem) {
            (Some(why), _) => Some(
                div()
                    .flex()
                    .flex_col()
                    .text_sm()
                    .child(
                        div()
                            .text_color(theme.danger)
                            .font_weight(FontWeight::MEDIUM)
                            .child(t!("Hmm, that character didn't save")),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(SharedString::from(why.clone())),
                    ),
            ),
            (None, Some(problem)) if !problem.is_empty() => {
                Some(div().text_sm().text_color(theme.danger).child(problem))
            }
            // A new character starts without a name and that isn't a mistake, so this says
            // what Save is waiting for in the muted tone, not as an error.
            (None, Some(_)) => Some(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("Give the character a name first")),
            ),
            _ => None,
        };
        div()
            .id("character-editor")
            .flex()
            .flex_col()
            .size_full()
            .gap_4()
            .p_6()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(back)
                    .child(actions),
            )
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_4()
                    .child(
                        div()
                            .flex_1()
                            .max_w(px(320.))
                            .min_w_0()
                            .child(required_field(
                                t!("Preset name"),
                                "",
                                Input::new(&self.name),
                                cx,
                            )),
                    )
                    .child(div().flex_1().min_w_0().pb_1().children(note)),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .overflow_hidden()
                    .rounded(theme.radius_lg)
                    .bg(theme.group_box)
                    .border_1()
                    .border_color(theme.border)
                    .child(self.preview.clone()),
            )
    }
}

/// The editor's controls, drawn in the shell's right panel.
pub struct Panel(Entity<Editor>);

impl Panel {
    pub fn new(editor: Entity<Editor>, cx: &mut Context<Self>) -> Self {
        cx.observe(&editor, |_, _, cx| cx.notify()).detach();
        Self(editor)
    }
}

impl Render for Panel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.0.update(cx, |editor, cx| editor.panel(cx))
    }
}

/// A trait to take or drop: a full-width row, so taking one doesn't move the others.
fn trait_row(
    id: SharedString,
    name: SharedString,
    cost: Option<i32>,
    selected: bool,
    free: bool,
    theme: &gpui_kit::component::Theme,
    cx: &App,
) -> Stateful<Div> {
    let hover = if selected {
        theme.primary.opacity(0.35)
    } else {
        ui::hovered(theme.title_bar, cx)
    };
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(32.))
        .px_2()
        .rounded(theme.radius)
        .text_sm()
        .when(selected, |this| this.bg(theme.primary.opacity(0.25)))
        .map(|this| {
            if free {
                this.cursor_pointer().hover(|s| s.bg(hover))
            } else {
                // Its profession rules it out: shown, but it can't be taken.
                this.opacity(0.5)
            }
        })
        .child(div().flex_none().size_4().when(selected, |this| {
            this.child(Glyph::new(IconName::Check).size_4())
        }))
        .child(div().flex_1().min_w_0().truncate().child(name))
        .children(cost.map(|cost| {
            div()
                .flex_none()
                .text_color(points_color(cost, theme))
                .child(points(cost))
        }))
}

/// A new look: the game's first skin color and hair color, and its first female hair style
/// that isn't bald.
fn first_look(catalog: &Catalog) -> Look {
    let style = catalog
        .female_hair
        .iter()
        .find(|s| s.id != "Bald")
        .map_or("Bob", |s| s.id.as_str());
    Look::new(
        true,
        catalog.skins.first().copied().unwrap_or([1., 0.91, 0.72]),
        style,
        catalog
            .hair_colors
            .first()
            .copied()
            .unwrap_or([0.2, 0.15, 0.1]),
    )
}

/// "+4", "-2", "0".
fn points(n: i32) -> String {
    if n > 0 {
        format!("+{n}")
    } else {
        n.to_string()
    }
}

/// As the game paints them: green for what costs points (the good), red for what gives them
/// (the bad), muted for none.
fn points_color(n: i32, theme: &gpui_kit::component::Theme) -> Hsla {
    match n.signum() {
        1 => theme.danger,
        -1 => theme.success,
        _ => theme.muted_foreground,
    }
}

pub fn color(rgb: Rgb) -> Hsla {
    Rgba {
        r: rgb[0],
        g: rgb[1],
        b: rgb[2],
        a: 1.,
    }
    .into()
}

/// A round swatch of `rgb`.
pub fn dot(rgb: Rgb, size: f32, border: Hsla) -> Div {
    div()
        .flex_none()
        .size(px(size))
        .rounded_full()
        .bg(color(rgb))
        .border_1()
        .border_color(border)
}

/// The colors the game offers, the one in use ringed in the accent. A saved color that isn't
/// among them (picked from the game's full picker) shows first, ringed.
fn swatches(
    key: &'static str,
    colors: &[Rgb],
    current: Option<Rgb>,
    size: f32,
    theme: &gpui_kit::component::Theme,
    cx: &mut Context<Editor>,
    set: impl Fn(&mut Look, Rgb) + Clone + 'static,
) -> Div {
    let near = |a: Rgb, b: Rgb| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.01);
    let mut all: Vec<Rgb> = colors.to_vec();
    if let Some(current) = current
        && !all.iter().any(|c| near(*c, current))
    {
        all.insert(0, current);
    }
    div()
        .flex()
        .flex_wrap()
        .gap_1()
        .children(all.into_iter().enumerate().map(|(ix, rgb)| {
            let chosen = current.is_some_and(|c| near(c, rgb));
            let set = set.clone();
            div()
                .id((key, ix))
                .p(px(2.))
                .rounded_full()
                .border_2()
                .border_color(if chosen {
                    theme.primary
                } else {
                    transparent_black()
                })
                .cursor_pointer()
                .when(!chosen, |this| this.hover(|s| s.border_color(theme.border)))
                .child(dot(rgb, size, theme.border))
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(look) = &mut this.look {
                        set(look, rgb);
                        cx.notify();
                    }
                }))
        }))
}

/// A choice that is on or off, like the channel chips of Create instance.
fn chip(
    id: SharedString,
    label: impl IntoElement,
    selected: bool,
    theme: &gpui_kit::component::Theme,
    cx: &App,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_1p5()
        .h(px(32.))
        .px_3()
        .rounded(theme.radius)
        .border_1()
        .text_sm()
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.bg(theme.primary.opacity(0.25))
                    .border_color(theme.primary)
                    .hover(|s| s.bg(theme.primary.opacity(0.35)))
            } else {
                this.bg(theme.background)
                    .border_color(theme.border)
                    .hover(|s| s.bg(ui::hovered(theme.background, cx)))
            }
        })
        .when(selected, |this| {
            this.child(Glyph::new(IconName::Check).size_4())
        })
        .child(label)
}

/// A labelled control, with an optional line under the label.
fn field(title: &'static str, line: &'static str, control: impl IntoElement, cx: &App) -> Div {
    labelled(title, false, line, control, cx)
}

/// Like [`field`], for what must be filled in: a small red asterisk after the title.
fn required_field(
    title: &'static str,
    line: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    labelled(title, true, line, control, cx)
}

fn labelled(
    title: &'static str,
    required: bool,
    line: &'static str,
    control: impl IntoElement,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .flex()
                        .gap_1()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title)
                        .when(required, |this| {
                            this.child(div().text_color(cx.theme().danger).child("*"))
                        }),
                )
                .when(!line.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(line),
                    )
                }),
        )
        .child(control)
}

/// What shows where half of a character is missing, with a button to add it.
fn missing(
    text: &'static str,
    action: &'static str,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_start()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .child(
            ui::secondary("character-add-half", action)
                .icon(IconName::Plus)
                .on_click(on_click),
        )
}
