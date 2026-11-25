//! "Create instance" dialog, in two steps: pick how to start, then set the picture, name and
//! game version.

use gpui_kit::assets::IconName;
use gpui_kit::component::IndexPath;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::hover_card::HoverCard;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use gpui_kit::component::{ActiveTheme, Icon as Glyph, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use std::path::PathBuf;
use std::rc::Rc;

use super::branches::{self, Branch, Choice};
use super::existing::{self, Existing};
use super::install::size;
use super::mods::{self, Bundle};
use super::picture::{self, Picker};
use super::{Instance, store};
use crate::ui::{self, FancyButton, Loader, Meta};

/// The step the dialog is on.
#[derive(Clone, Copy, PartialEq)]
enum Step {
    Choose,
    Custom,
    Collection,
}

/// A collection's name and the Workshop items to install from it.
pub type Mods = Option<(SharedString, Vec<u64>)>;

type Created = Rc<dyn Fn(Instance, Mods, &mut Window, &mut App)>;
type Bring = Rc<dyn Fn(Existing, &mut Window, &mut App)>;
type Import = Rc<dyn Fn(PathBuf, &mut Window, &mut App)>;

/// What is known about the pasted collection link.
enum Lookup {
    /// Nothing usable typed yet.
    Empty,
    Loading(u64),
    Found(Box<Bundle>),
    Failed(String),
}

/// Opens the dialog. `created` runs after a new instance is written under `root`, with the
/// mods to install into it when it started from a collection. With `collection` (a link
/// brought it) the dialog opens on that collection, already found. `bring` takes over when
/// the game someone already plays is chosen as the start, and `import` when a `.lata` is.
/// What the dialog starts from, besides a blank instance.
pub enum Start {
    /// A collection's link, looked up already.
    Collection(Bundle),
    /// A mod: its title, and whether it runs on Build 42 only (`true`) or Build 41 only
    /// (`false`). Only versions of that build are offered.
    Mod(String, bool),
}

pub fn open(
    root: PathBuf,
    start: Option<Start>,
    bring: impl Fn(Existing, &mut Window, &mut App) + 'static,
    import: impl Fn(PathBuf, &mut Window, &mut App) + 'static,
    created: impl Fn(Instance, Mods, &mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let created: Created = Rc::new(created);
    let view = cx.new(|cx| {
        let mut create = Create::new(root, created, window, cx);
        create.bring = Some(Rc::new(bring));
        create.import = Some(Rc::new(import));
        create.load(window, cx);
        match start {
            Some(Start::Collection(found)) => create.start_from(found, window, cx),
            // Made for a mod: straight to picking its version, among the ones it runs on.
            Some(Start::Mod(title, modern)) => {
                create.step = Step::Custom;
                create.made_for = Some((title, modern));
            }
            None => {}
        }
        create
    });
    window.open_dialog(cx, move |dialog, _, _| {
        // Fades in settling from just above, and back out (the default drops from the window top).
        let dialog = dialog.entrance(DialogEntrance::FadeSlide);
        dialog
            .title(t!("Create instance"))
            .centered()
            .w(px(620.))
            .child(view.clone())
    });
}

struct Create {
    root: PathBuf,
    created: Created,
    bring: Option<Bring>,
    import: Option<Import>,
    /// The game played without lata, when its folder has something to bring.
    existing: Option<Existing>,
    step: Step,
    /// Whether the last step change went forward; steps slide in from that side.
    forward: bool,
    /// `None` while loading from Steam.
    branches: Option<Result<Vec<Branch>, String>>,
    /// Every version on offer, by channel; filled once the branches are in.
    choices: Vec<Choice>,
    /// The channel picked, and the version picked (an index into `choices`).
    channel: String,
    chosen: usize,
    /// The version dropdown, for the channel picked.
    select: Option<Entity<SelectState<Vec<Row>>>>,
    _select: Option<Subscription>,
    /// The picture so far, beside the name field.
    picture: Entity<Picker>,
    name: Entity<InputState>,
    /// The name used when the field is left empty; it shows as the placeholder.
    suggested: &'static str,
    link: Entity<InputState>,
    collection: Lookup,
    /// The mod it is made for, and whether that one runs on Build 42 only.
    made_for: Option<(String, bool)>,
    _link: Subscription,
}

impl Create {
    fn new(root: PathBuf, created: Created, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let suggested = super::suggest();
        let link = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://steamcommunity.com/sharedfiles/filedetails/?id=")
        });
        let on_link = cx.subscribe_in(&link, window, |this, _, event: &InputEvent, window, cx| {
            if let InputEvent::Change = event {
                this.typed(window, cx);
            }
        });
        let name = cx.new(|cx| InputState::new(window, cx).placeholder(suggested));
        Self {
            root,
            created,
            bring: None,
            import: None,
            existing: existing::home().and_then(|dir| existing::find(&dir)),
            step: Step::Choose,
            forward: true,
            branches: None,
            choices: Vec::new(),
            channel: String::new(),
            chosen: 0,
            select: None,
            _select: None,
            picture: cx.new(|_| Picker::new(None, false).with_name(name.clone())),
            name,
            suggested,
            link,
            collection: Lookup::Empty,
            made_for: None,
            _link: on_link,
        }
    }

    /// Looks the collection up as soon as the link holds an id, so pasting is all it takes.
    fn typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.link.read(cx).value();
        let Some(id) = steam::workshop::parse_id(&typed) else {
            self.collection = if typed.trim().is_empty() {
                Lookup::Empty
            } else {
                Lookup::Failed(t!("Hmm, that doesn't look like a Workshop link").into())
            };
            return cx.notify();
        };
        let known = match &self.collection {
            Lookup::Loading(now) => *now == id,
            Lookup::Found(found) => found.id == id,
            _ => false,
        };
        if known {
            return;
        }
        self.collection = Lookup::Loading(id);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let found = mods::collection(id).await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Another link was pasted meanwhile: this answer is stale.
                if !matches!(this.collection, Lookup::Loading(now) if now == id) {
                    return;
                }
                this.collection = match found {
                    Ok(found) => {
                        this.adopt(&found, window, cx);
                        Lookup::Found(Box::new(found))
                    }
                    Err(why) => Lookup::Failed(why),
                };
                cx.notify();
            });
        })
        .detach();
    }

    /// Starts on the collection step with `found` in place, as if its link had been pasted and
    /// looked up.
    fn start_from(&mut self, found: Bundle, window: &mut Window, cx: &mut Context<Self>) {
        self.step = Step::Collection;
        self.adopt(&found, window, cx);
        let link = format!(
            "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
            found.id
        );
        // Set before the field: its change looks the link up, and finds it already known.
        self.collection = Lookup::Found(Box::new(found));
        self.link
            .update(cx, |field, cx| field.set_value(link, window, cx));
    }

    /// The instance takes the collection's name (as the one used when the field is left
    /// empty) and its picture; both can still be changed.
    fn adopt(&mut self, found: &Bundle, window: &mut Window, cx: &mut Context<Self>) {
        let title = found.title.clone();
        self.name
            .update(cx, |name, cx| name.set_placeholder(title, window, cx));
        if let Some(file) = &found.picture {
            self.picture.update(cx, |picker, cx| {
                picker.icon = super::Icon::Image {
                    file: file.to_string_lossy().into_owned(),
                };
                cx.notify();
            });
        }
    }

    /// Whether the chosen game version is Build 42 or newer.
    fn modern(&self) -> bool {
        self.choices.get(self.chosen).is_none_or(branches::modern)
    }

    /// Picks `channel` and its newest version.
    fn pick_channel(&mut self, channel: String, window: &mut Window, cx: &mut Context<Self>) {
        self.chosen = self
            .choices
            .iter()
            .position(|c| c.channel == channel)
            .unwrap_or(0);
        self.channel = channel;
        let rows: Vec<Row> = self
            .choices
            .iter()
            .enumerate()
            .filter(|(_, c)| c.channel == self.channel)
            .map(|(ix, c)| Row {
                ix,
                name: c.name.clone().into(),
                latest: c.latest,
                date: c.date.map(|at| branches::date(at).into()),
                accent: cx.theme().primary,
            })
            .collect();
        let state = cx.new(|cx| SelectState::new(rows, Some(IndexPath::new(0)), window, cx));
        self._select = Some(cx.subscribe_in(
            &state,
            window,
            |this, _, event: &SelectEvent<Vec<Row>>, _, cx| {
                if let SelectEvent::Confirm(Some(ix)) = event {
                    this.chosen = *ix;
                    cx.notify();
                }
            },
        ));
        self.select = Some(state);
        cx.notify();
    }

    /// The collection's name and the items that fit the chosen version, once it is found
    /// and has some.
    fn mods(&self) -> Mods {
        let Lookup::Found(found) = &self.collection else {
            return None;
        };
        let (fit, _) = found.split(self.modern());
        (!fit.is_empty()).then(|| {
            (
                found.title.clone().into(),
                fit.iter().map(|item| item.id).collect(),
            )
        })
    }

    fn go(&mut self, step: Step, cx: &mut Context<Self>) {
        self.forward = step != Step::Choose;
        self.step = step;
        cx.notify();
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.branches = None;
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = branches::fetch().await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.choices = match &result {
                    Ok(list) => branches::choices(list),
                    Err(_) => Vec::new(),
                };
                if let Some((_, modern)) = &this.made_for {
                    this.choices
                        .retain(|choice| branches::modern(choice) == *modern);
                }
                this.branches = Some(result);
                // Stable first: the newest stable version is where most people start.
                let first = this.choices.first().map(|c| c.channel.clone());
                if let Some(channel) = first {
                    this.pick_channel(channel, window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = self.name.read(cx).value().trim().to_owned();
        let mods = self.mods();
        if self.step == Step::Collection && mods.is_none() {
            return;
        }
        let name = match (typed.is_empty(), &mods) {
            (false, _) => typed,
            (true, Some((collection, _))) => collection.to_string(),
            (true, None) => self.suggested.to_owned(),
        };
        let Some(choice) = self.choices.get(self.chosen) else {
            return;
        };
        let made = store::create(&self.root, &name, &choice.branch).and_then(|mut instance| {
            // An older version installs by its manifests; the branch stays its update channel.
            if let Some(target) = &choice.target {
                store::set_target(&instance.dir, Some(target.clone()))?;
            }
            let icon = picture::keep(&self.picture.read(cx).icon, &instance.dir)?;
            store::set_icon(&instance.dir, Some(icon.clone()))?;
            instance.icon = Some(icon);
            Ok(instance)
        });
        match made {
            Ok(instance) => {
                window.close_dialog(cx);
                (self.created)(instance, mods, window, cx);
            }
            Err(err) => window.push_notification(
                crate::ui::toast::failed_because(t!("Well, that instance didn't happen"), err),
                cx,
            ),
        }
    }

    /// Step 1: one row per way to start.
    fn choose(&self, cx: &mut Context<Self>) -> Div {
        let options = [
            (
                Step::Custom,
                IconName::Box,
                t!("Custom setup"),
                t!("Start from scratch with a game version you pick."),
                true,
            ),
            (
                Step::Collection,
                IconName::Compass,
                t!("From a Workshop collection"),
                t!("Paste a collection link and get its mods installed."),
                true,
            ),
        ];
        // Its own dialog: the worlds found decide the game version, so there is no form here.
        let existing = self
            .existing
            .clone()
            .zip(self.bring.clone())
            .map(|(game, bring)| {
                option(
                    options.len(),
                    IconName::Globe,
                    t!("From the game you already play"),
                    t!("Bring your worlds, their mods and your settings over."),
                    true,
                    cx,
                )
                .on_click(move |_, window, cx| {
                    let (game, bring) = (game.clone(), bring.clone());
                    window.close_dialog(cx);
                    // After this update ends, so the next dialog opens once this one is closing.
                    window.defer(cx, move |window, cx| bring(game, window, cx));
                })
            });
        let pack = option(
            options.len() + 1,
            IconName::Download,
            t!("Import a .lata"),
            t!("Open a shared pack with its build, mods and settings."),
            true,
            cx,
        )
        .on_click(cx.listener(|this, _, window, cx| this.pick_pack(window, cx)));
        div()
            .flex()
            .flex_col()
            .gap_3()
            .children(existing)
            .children(options.into_iter().enumerate().map(
                |(ix, (step, icon, title, line, enabled))| {
                    option(ix, icon, title, line, enabled, cx).when(enabled, |this| {
                        this.on_click(cx.listener(move |this, _, _, cx| this.go(step, cx)))
                    })
                },
            ))
            .child(pack)
    }

    /// Asks the OS for a `.lata` and hands it over, closing this dialog.
    fn pick_pack(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(t!("Import").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let Some(import) = this.import.clone() else {
                    return;
                };
                window.close_dialog(cx);
                window.defer(cx, move |window, cx| import(path, window, cx));
            });
        })
        .detach();
    }

    /// Game version: one chip per Steam branch, the chosen one filled.
    fn versions(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme().clone();
        let list = match &self.branches {
            None => {
                return div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label(t!("Game version")))
                    .child(Loader)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(t!("Loading versions from Steam")),
                    );
            }
            Some(Err(err)) => {
                return titled(
                    div()
                        .flex()
                        .items_center()
                        .gap_3()
                        .text_sm()
                        .child(
                            div()
                                .flex_1()
                                .text_color(theme.muted_foreground)
                                .child(t!("Couldn't reach Steam: {err}", err = err)),
                        )
                        .child(
                            ui::secondary("retry", t!("Retry"))
                                .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
                        ),
                );
            }
            Some(Ok(list)) => list,
        };
        let mut channels: Vec<&str> = Vec::new();
        for choice in &self.choices {
            if !channels.contains(&choice.channel.as_str()) {
                channels.push(&choice.channel);
            }
        }
        let chips = channels.into_iter().map(|channel| {
            let selected = channel == self.channel;
            let pick = channel.to_owned();
            div()
                .id(SharedString::from(format!("channel-{channel}")))
                .flex()
                .items_center()
                .gap_1p5()
                .h(px(34.))
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
                    } else {
                        this.bg(theme.background)
                            .border_color(theme.border)
                            .hover(|s| s.bg(crate::ui::hovered(theme.background, cx)))
                    }
                })
                .when(selected, |this| {
                    this.child(Glyph::new(IconName::Check).size_4())
                })
                .child(crate::i18n::tr(channel).to_owned())
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.channel != pick {
                        this.pick_channel(pick.clone(), window, cx);
                    }
                }))
        });
        let Some(chosen) = self.choices.get(self.chosen) else {
            return titled(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("Steam has no versions to offer right now")),
            );
        };
        let live = list.iter().find(|b| b.name == chosen.branch);
        // What the pick means for updates, in a line under the dropdown.
        let about = match (&chosen.target, live) {
            (Some(_), _) => t!("An older version. lata lets you know when a newer one is out, updating is up to you.").to_owned(),
            (None, Some(b)) if branches::fallback(&b.name) => {
                t!("Steam keeps this one on its own branch, so it won't get updates.").to_owned()
            }
            (None, Some(b)) => match &b.description {
                Some(text) if branches::described(text).is_none() => text.clone(),
                _ if b.name == "public" => t!("Current release").to_owned(),
                _ => String::new(),
            },
            (None, None) => String::new(),
        };
        // Swaps in place when the pick changes: a short fade and a few px of travel. Its line
        // is always there, so the footer never moves.
        let from = 4.;
        // A slot two lines tall, the text at its top: one line or two, the note starts in the
        // same place and nothing under it moves.
        let note = div().h(px(32.)).child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(about)
                .with_animation(
                    ElementId::Name(format!("version-about-{}", self.chosen).into()),
                    Animation::new(crate::ui::ENTER).with_easing(crate::ui::ease_out_quint),
                    move |this, t| this.relative().top(px(from * (1. - t))).opacity(t),
                ),
        );
        div()
            .flex()
            .flex_col()
            .gap_6()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label(t!("Channel")))
                    .child(div().flex().flex_wrap().gap_2().children(chips))
                    .children(self.made_for.as_ref().map(|(title, modern)| {
                        let build = if *modern { "Build 42" } else { "Build 41" };
                        div().text_xs().text_color(theme.muted_foreground).child(t!(
                            "Only {build} versions here: {title} runs on that one.",
                            build = build,
                            title = title
                        ))
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(label(t!("Game version")))
                    .children(self.select.as_ref().map(|state| {
                        Select::new(state)
                            .menu_max_h(px(280.))
                            .accessibility_label(t!("Game version"))
                    }))
                    .child(note),
            )
    }

    /// What the pasted collection holds for the chosen version, under its link. Every state
    /// takes the same height, so the form below never jumps.
    fn summary(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let frame = div()
            .flex()
            .flex_col()
            .justify_center()
            .gap_2()
            .h(SUMMARY)
            .px_4()
            .rounded(theme.radius_lg)
            .bg(theme.group_box)
            .border_1()
            .border_color(theme.border);
        let note = |icon: IconName, text: SharedString, color: Hsla| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_w_0()
                .text_sm()
                .text_color(color)
                .child(Glyph::new(icon).size_4().flex_none())
                .child(div().min_w_0().truncate().child(text))
        };
        match &self.collection {
            Lookup::Empty => frame
                .items_center()
                .child(note(
                    IconName::Link,
                    t!("Paste a collection link and what's in it shows up here").into(),
                    theme.muted_foreground,
                ))
                .into_any_element(),
            Lookup::Loading(_) => frame
                .items_center()
                .child(note(
                    IconName::Search,
                    t!("Asking Steam what's in it").into(),
                    theme.muted_foreground,
                ))
                .child(div().w(px(160.)).child(Loader))
                .into_any_element(),
            Lookup::Failed(why) => frame
                .items_center()
                .child(note(
                    IconName::TriangleAlert,
                    why.clone().into(),
                    theme.danger,
                ))
                .into_any_element(),
            Lookup::Found(found) => {
                let modern = self.modern();
                let (fit, other) = found.split(modern);
                let total: u64 = fit.iter().map(|item| item.size).sum();
                let build = if modern { "Build 41" } else { "Build 42" };
                let facts = div()
                    .flex()
                    .items_center()
                    .gap_4()
                    .min_w_0()
                    .map(|this| {
                        if fit.is_empty() {
                            this.child(note(
                                IconName::TriangleAlert,
                                t!("Nothing in it runs on this version").into(),
                                theme.danger,
                            ))
                        } else {
                            this.child(Meta::new(
                                IconName::Puzzle,
                                tn!(fit.len(), "{n} mod", "{n} mods"),
                            ))
                            .child(Meta::new(IconName::Download, size(total)))
                        }
                    })
                    .when(!other.is_empty(), |this| {
                        this.child(left_out(&other, build, cx))
                    });
                let card = frame
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .truncate()
                            .child(found.title.clone()),
                    )
                    .child(facts);
                ui::enter(card, "collection-found").into_any_element()
            }
        }
    }

    /// Back on the left and Create on the right, always there; Create waits, dimmed, until
    /// there is something to create.
    fn footer(&self, ready: bool, cx: &mut Context<Self>) -> Div {
        div()
            .flex()
            .justify_between()
            .child(
                ui::secondary("back", t!("Back"))
                    .icon(IconName::ChevronLeft)
                    .on_click(cx.listener(|this, _, _, cx| this.go(Step::Choose, cx))),
            )
            .child(
                FancyButton::new("create", t!("Create instance"))
                    .small()
                    .icon(IconName::Plus)
                    .disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.create(window, cx))),
            )
    }
}

fn option(
    ix: usize,
    icon: IconName,
    title: &'static str,
    line: &'static str,
    enabled: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(ix)
        .flex()
        .items_center()
        .gap_3()
        .p_4()
        .rounded(theme.radius_lg)
        .bg(theme.group_box)
        .border_1()
        .border_color(theme.border)
        .map(|this| {
            if enabled {
                this.cursor_pointer()
                    .hover(|s| s.bg(crate::ui::hovered(theme.group_box, cx)))
            } else {
                this.opacity(0.5)
            }
        })
        .child(
            div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(44.))
                .rounded(theme.radius)
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                // Muted like every other icon tile (the rail's, a mod's placeholder): the title
                // carries the weight, not the glyph.
                .child(Glyph::new(icon).size_5().text_color(theme.muted_foreground)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(line),
                ),
        )
}

/// Height of the collection summary in every state: its title and one line of facts.
const SUMMARY: Pixels = px(80.);

/// How many of the collection's mods stay out for being made for `build` alone, underlined;
/// hovering lists them, each one opening its Workshop page.
fn left_out(other: &[&steam::workshop::Item], build: &'static str, cx: &App) -> HoverCard {
    let color = ui::warning_text(cx);
    let items: Rc<Vec<(u64, String)>> = Rc::new(
        other
            .iter()
            .map(|item| (item.id, item.title.clone()))
            .collect(),
    );
    let count = items.len();
    HoverCard::new("left-out")
        .open_delay(std::time::Duration::from_millis(300))
        .trigger(
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1p5()
                .text_sm()
                .text_color(color)
                .cursor_pointer()
                .child(Glyph::new(IconName::TriangleAlert).size_4())
                .child(
                    div()
                        .underline()
                        .child(tn!(count, "{n} left out", "{n} left out")),
                ),
        )
        .content(move |_, _, cx| {
            let theme = cx.theme();
            let rows = items.iter().enumerate().map(|(ix, (id, title))| {
                let url = format!("https://steamcommunity.com/sharedfiles/filedetails/?id={id}");
                div()
                    .id(("left-out", ix))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.muted))
                    .child(div().flex_1().min_w_0().truncate().child(title.clone()))
                    .child(
                        Glyph::new(IconName::ExternalLink)
                            .size_3p5()
                            .text_color(theme.muted_foreground),
                    )
                    .on_click(move |_, _, cx| cx.open_url(&url))
            });
            div()
                .flex()
                .flex_col()
                .gap_1()
                .w(px(300.))
                .child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("For {build} only, so they stay out", build = build)),
                )
                .child(
                    div()
                        .id("left-out-list")
                        .flex()
                        .flex_col()
                        .max_h(px(220.))
                        .overflow_y_scroll()
                        .children(rows),
                )
        })
}

/// A field's label over its content.
fn titled(content: Div) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_2()
        .child(label(t!("Game version")))
        .child(content)
}

fn label(text: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

impl Render for Create {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Page side-by-side: each step slides 8 px in from the side it was reached from.
        let from = if self.forward { 8. } else { -8. };
        let step = ElementId::Name(format!("step-{}", self.step as u8).into());
        let body = match self.step {
            Step::Choose => self.choose(cx),
            Step::Custom | Step::Collection => {
                let ready = matches!(self.branches, Some(Ok(_)))
                    && (self.step != Step::Collection || self.mods().is_some());
                let versions = self.versions(cx);
                // From a collection the link comes first: it fills in the picture and the
                // name, and the version sits under its summary, whose warning depends on it.
                let body = div().flex().flex_col().gap_6();
                match self.step {
                    Step::Collection => body
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_2()
                                .child(label(t!("Collection link")))
                                .child(Input::new(&self.link))
                                .child(self.summary(cx)),
                        )
                        .child(versions)
                        .child(self.picture.clone()),
                    _ => body.child(self.picture.clone()).child(versions),
                }
                .child(self.footer(ready, cx))
            }
        };
        body.with_animation(
            step,
            Animation::new(crate::ui::ENTER).with_easing(crate::ui::ease_out_quint),
            move |body, t| body.relative().left(px(from * (1. - t))).opacity(t),
        )
    }
}

/// A version in the dropdown.
#[derive(Clone)]
struct Row {
    /// Its index in the dialog's `choices`.
    ix: usize,
    name: SharedString,
    latest: bool,
    date: Option<SharedString>,
    /// The theme's accent, for the "Latest" tag (the trigger draws it without a context).
    accent: Hsla,
}

impl Row {
    fn tag(&self) -> Div {
        div()
            .px_1p5()
            .rounded(px(4.))
            .text_xs()
            .font_weight(FontWeight::MEDIUM)
            .bg(self.accent.opacity(0.15))
            .text_color(self.accent)
            .child(t!("Latest"))
    }
}

impl SearchableListItem for Row {
    type Value = usize;

    fn title(&self) -> SharedString {
        match self.latest {
            true => t!("{name} (latest)", name = self.name).into(),
            false => self.name.clone(),
        }
    }

    fn display_title(&self) -> Option<AnyElement> {
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.name.clone())
                .when(self.latest, |this| this.child(self.tag()))
                .into_any_element(),
        )
    }

    fn render(&self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        div()
            .flex()
            .flex_1()
            .items_center()
            .gap_2()
            .child(self.name.clone())
            .when(self.latest, |this| this.child(self.tag()))
            .child(div().flex_1())
            .children(
                self.date
                    .clone()
                    .map(|date| div().text_xs().text_color(muted).child(date)),
            )
    }

    fn value(&self) -> &usize {
        &self.ix
    }
}
