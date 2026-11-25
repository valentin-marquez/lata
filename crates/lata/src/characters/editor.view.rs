//! The character editor: a dialog laid out like Settings, a section per part of the game's
//! character screen. Its lists come from one installed build ([`Catalog`]).

use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::assets::IconName;
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
use super::preview::Preview;
use crate::ui::{self, FancyButton};

const WIDTH: f32 = 1080.;
const HEIGHT: f32 = 560.;
const NAV: f32 = 200.;
const PREVIEW: f32 = 300.;

/// What happens on Save: the character, and its name before (`None` for a new one).
type Saved = Rc<dyn Fn(Character, Option<String>, &mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Look,
    Clothes,
    Profession,
    Traits,
}

impl Section {
    const ALL: [Section; 4] = [
        Section::Look,
        Section::Clothes,
        Section::Profession,
        Section::Traits,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Look => t!("Look"),
            Section::Clothes => t!("Clothes"),
            Section::Profession => t!("Profession"),
            Section::Traits => t!("Traits"),
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::Look => IconName::UserRound,
            Section::Clothes => IconName::Shirt,
            Section::Profession => IconName::Briefcase,
            Section::Traits => IconName::WandSparkles,
        }
    }
}

/// Opens the editor on `character`, or on a new one when it is `None`. `taken` are the names
/// of the others, which this one can't take.
pub fn open(
    character: Option<Character>,
    taken: Vec<String>,
    catalog: Arc<Catalog>,
    saved: impl Fn(Character, Option<String>, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let title = if character.is_some() {
        t!("Edit character")
    } else {
        t!("New character")
    };
    let saved: Saved = Rc::new(saved);
    let view = cx.new(|cx| Editor::new(character, taken, catalog, saved, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title(title)
            .centered()
            .w(px(WIDTH))
            .child(view.clone())
    });
}

/// An entry in one of the editor's dropdowns.
#[derive(Clone)]
struct Pick {
    id: String,
    name: SharedString,
    /// Points it gives (a profession), shown beside it.
    cost: Option<i32>,
}

impl SearchableListItem for Pick {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone()
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .flex()
            .w_full()
            .justify_between()
            .gap_4()
            .child(self.name.clone())
            .children(
                self.cost
                    .map(|cost| div().text_color(muted).child(points(cost))),
            )
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

struct Editor {
    catalog: Arc<Catalog>,
    section: Section,
    /// The name it had when the editor opened; `None` for a new character.
    old: Option<String>,
    taken: Vec<String>,
    name: Entity<InputState>,
    first: Entity<InputState>,
    last: Entity<InputState>,
    look: Option<Look>,
    build: Option<Build>,
    hair: Option<Entity<SelectState<Vec<Pick>>>>,
    beard: Option<Entity<SelectState<Vec<Pick>>>>,
    profession: Option<Entity<SelectState<Vec<Pick>>>>,
    preview: Entity<Preview>,
    saved: Saved,
    _subscriptions: Vec<Subscription>,
}

impl Editor {
    fn new(
        character: Option<Character>,
        taken: Vec<String>,
        catalog: Arc<Catalog>,
        saved: Saved,
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
        let preview = cx.new(|cx| {
            Preview::new(
                catalog.media.clone(),
                catalog.skins.clone(),
                size(px(PREVIEW), px(HEIGHT)),
                cx,
            )
        });
        let mut editor = Self {
            name: input(character.name.clone(), t!("Kate the nurse"), window, cx),
            first: input(first, t!("Kate"), window, cx),
            last: input(last, t!("Smith"), window, cx),
            catalog,
            section: Section::Look,
            old: (!fresh).then(|| character.name.clone()),
            taken,
            look: character.look,
            build: character.build,
            hair: None,
            beard: None,
            profession: None,
            preview,
            saved,
            _subscriptions: Vec::new(),
        };
        editor.lists(window, cx);
        editor
    }

    /// Makes the dropdowns for what the character is now: the hair styles depend on gender.
    fn lists(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._subscriptions.clear();
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
        let picks: Vec<Pick> = list
            .iter()
            .map(|choice| Pick {
                id: choice.id.clone(),
                name: choice.name.clone().into(),
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
        state
    }

    fn picked(&mut self, which: Which, id: String) {
        match which {
            Which::Hair => {
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

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (name, problem) = self.name(cx);
        if problem.is_some() {
            return;
        }
        let mut look = self.look.clone();
        if let Some(look) = &mut look {
            look.set_names(&self.first.read(cx).value(), &self.last.read(cx).value());
        }
        let character = Character {
            name,
            look,
            build: self.build.clone(),
        };
        window.close_dialog(cx);
        (self.saved)(character, self.old.clone(), window, cx);
    }

    fn nav(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap_1()
            .w(px(NAV))
            .p_3()
            .border_r_1()
            .border_color(theme.border)
            .children(Section::ALL.map(|section| {
                ui::nav_item(section.label(), section.icon(), section == self.section, cx).on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.section = section;
                        cx.notify();
                    }),
                )
            }))
    }

    fn look(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let name = required_field(
            t!("Name"),
            t!("What the game lists it as"),
            Input::new(&self.name),
            cx,
        );
        let Some(look) = &self.look else {
            return div().flex().flex_col().gap_6().child(name).child(missing(
                t!("No look saved under this name, just a profession and traits."),
                t!("Add a look"),
                cx.listener(|this, _, window, cx| {
                    this.look = Some(first_look(&this.catalog));
                    this.lists(window, cx);
                    cx.notify();
                }),
                cx,
            ));
        };
        let female = look.female();
        let genders = div().flex().gap_2().children(
            [("Female", t!("Female"), true), ("Male", t!("Male"), false)].map(
                |(id, label, value)| {
                    chip(
                        SharedString::from(id),
                        label.into(),
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
            28.,
            &theme,
            cx,
            |look, color| look.set_skin(color),
        );
        let (style, hair) = look.hair();
        let hair_colors = swatches(
            "hair-color",
            &self.catalog.hair_colors,
            hair,
            22.,
            &theme,
            cx,
            move |look, color| look.set_hair(&style, color),
        );
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(name)
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(div().flex_1().child(field(
                        t!("First name"),
                        "",
                        Input::new(&self.first),
                        cx,
                    )))
                    .child(div().flex_1().child(field(
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

    fn clothes(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let pieces = self.look.as_ref().map(Look::pieces).unwrap_or_default();
        let rows = pieces.into_iter().enumerate().map(|(ix, piece)| {
            div()
                .id(("piece", ix))
                .flex()
                .items_center()
                .gap_3()
                .px_3()
                .py_2()
                .rounded(theme.radius)
                .hover(|s| s.bg(theme.muted))
                .child(match piece.tint {
                    Some(tint) => dot(tint, 14., theme.border),
                    None => div().size(px(14.)),
                })
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(self.catalog.item(&piece.item)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(piece.slot),
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

    fn profession(&self, cx: &mut Context<Self>) -> Div {
        let Some(state) = &self.profession else {
            return self.no_build(cx);
        };
        let about = self
            .build
            .as_ref()
            .filter(|b| self.catalog.profession(&b.profession).is_none())
            .map(|b| {
                div().text_xs().text_color(ui::warning_text(cx)).child(t!(
                    "\"{profession}\" isn't in this game version, maybe it comes from a mod.",
                    profession = b.profession
                ))
            });
        div().flex().flex_col().gap_2().child(field(
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
        ))
    }

    fn no_build(&self, cx: &mut Context<Self>) -> Div {
        missing(
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
        )
    }

    /// Traits the profession's own traits rule out (a veteran can't be brave).
    fn ruled_out(&self) -> Vec<String> {
        self.build
            .as_ref()
            .and_then(|b| self.catalog.profession(&b.profession))
            .map(|p| p.exclusive.clone())
            .unwrap_or_default()
    }

    fn traits(&self, cx: &mut Context<Self>) -> Div {
        let Some(build) = &self.build else {
            return self.no_build(cx);
        };
        let theme = cx.theme().clone();
        let has = |id: &str| build.traits.iter().any(|t| t.eq_ignore_ascii_case(id));
        let ruled = self.ruled_out();
        let free = |id: &str| !ruled.iter().any(|r| r.eq_ignore_ascii_case(id));
        let group = |title: &'static str, good: bool, cx: &mut Context<Self>| {
            let chips = self
                .catalog
                .traits
                .iter()
                .filter(|t| (t.cost >= 0) == good)
                .map(|t| {
                    let id = t.id.clone();
                    let chip = chip(
                        SharedString::from(format!("trait-{id}")),
                        format!("{} {}", t.name, points(-t.cost)).into(),
                        has(&id),
                        &theme,
                        cx,
                    );
                    if free(&id) {
                        chip.on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle(&id);
                            cx.notify();
                        }))
                    } else {
                        // Its profession rules it out: shown, but it can't be taken.
                        chip.opacity(0.5).cursor_default()
                    }
                })
                .collect::<Vec<_>>();
            field(
                title,
                "",
                div().flex().flex_wrap().gap_2().children(chips),
                cx,
            )
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
                    .flex_wrap()
                    .gap_2()
                    .children(unknown.into_iter().map(|id| {
                        chip(
                            SharedString::from(format!("unknown-{id}")),
                            id.clone().into(),
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
            .gap_6()
            .child(group(t!("Good traits"), true, cx))
            .child(group(t!("Bad traits"), false, cx))
            .children(others)
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

    fn footer(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (_, problem) = self.name(cx);
        // A new character starts without a name and that isn't a mistake, so this says what the
        // button is waiting for in the muted tone, not as an error.
        let nameless = problem == Some("");
        let note = match (problem, self.points()) {
            (Some(problem), _) if !problem.is_empty() => {
                Some(div().text_color(theme.danger).child(problem))
            }
            (Some(_), _) => Some(
                div()
                    .text_color(theme.muted_foreground)
                    .child(t!("Give the character a name first")),
            ),
            (_, Some(left)) if left < 0 => Some(div().text_color(ui::warning_text(cx)).child(tn!(
                -left,
                "{n} points short, the game won't start it like this",
                "{n} points short, the game won't start it like this"
            ))),
            (_, Some(left)) => Some(div().text_color(theme.muted_foreground).child(tn!(
                left,
                "{n} point left",
                "{n} points left"
            ))),
            (_, None) => None,
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_6()
            .py_3()
            .border_t_1()
            .border_color(theme.border)
            .child(div().flex_1().min_w_0().text_sm().children(note))
            .child(
                ui::secondary("character-cancel", t!("Cancel"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                // The wrapper carries the tooltip: the button itself is off while it applies.
                div()
                    .id("character-save-tip")
                    .when(nameless, |this| {
                        this.tooltip(|window, cx| {
                            Tooltip::new(t!("Give the character a name first")).build(window, cx)
                        })
                    })
                    .child(
                        FancyButton::new(
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
                        .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            )
    }
}

impl Render for Editor {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::Look => self.look(cx),
            Section::Clothes => self.clothes(cx),
            Section::Profession => self.profession(cx),
            Section::Traits => self.traits(cx),
        };
        let look = self.look.clone();
        self.preview.update(cx, |p, _| p.show(look.as_ref()));
        let theme = cx.theme();
        let (border, well) = (theme.border, theme.muted);
        div()
            .flex()
            .h(px(HEIGHT))
            .border_t_1()
            .border_color(border)
            .child(self.nav(cx))
            .child(
                div()
                    .flex_none()
                    .bg(well)
                    .border_r_1()
                    .border_color(border)
                    .child(self.preview.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .id("character-content")
                            .flex_1()
                            .min_h_0()
                            .p_6()
                            .overflow_y_scroll()
                            .child(ui::enter(content, self.section.label())),
                    )
                    .child(self.footer(cx)),
            )
    }
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
    label: SharedString,
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
