//! The Mods tab: the instance's mods in a table (mod, version, actions) with search, sort and a
//! filter above, and a bar of actions for the rows that are checked (Modrinth App's content
//! tab).

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Instance;
use super::java;
use super::mods::{self, Mod};
use crate::shell::Shell;
use crate::ui::{self, Empty, FancyButton, toast};

/// Side of a mod's picture in its row.
const ICON: Pixels = px(40.);
/// Height of the toolbar's controls: the primary button's small size.
const CONTROL: Pixels = px(36.);
/// Width of the Version and Actions columns; Mod takes the rest.
const VERSION: Pixels = px(190.);
const ACTIONS: Pixels = px(168.);
/// Smallest width of the search field, so its placeholder always reads.
const SEARCH: f32 = 240.;
/// The toolbar's two groups at their least; when both don't fit side by side, the second wraps.
const QUERY: f32 = 560.;
const ACTIONS_ROW: f32 = 392.;

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Name,
    EnabledFirst,
    Added,
    Updated,
}

impl Sort {
    fn label(self) -> &'static str {
        match self {
            Sort::Name => t!("Name"),
            // "On first" was read as install order.
            Sort::EnabledFirst => t!("Enabled first"),
            Sort::Added => t!("Recently installed"),
            Sort::Updated => t!("Recently updated"),
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    On,
    Off,
    /// A newer Workshop version is out: found without scrolling the whole list.
    Updates,
}

impl Filter {
    fn label(self) -> &'static str {
        match self {
            Filter::All => t!("All mods"),
            Filter::On => t!("On"),
            Filter::Off => t!("Off"),
            Filter::Updates => t!("Updates available"),
        }
    }
}

/// Where asking Steam for newer versions stands.
#[derive(Clone, Copy, PartialEq)]
enum Check {
    Running,
    Done,
    /// Offline or Steam down: the table works the same, without update marks.
    Failed,
}

pub struct Mods {
    instance: Instance,
    /// Read from disk when the tab opens and after every change.
    mods: Vec<Mod>,
    /// Names of the mods made for a newer game than the instance runs, read with `mods`.
    too_new: Vec<String>,
    /// What undoing the last update would put back ([`mods::undo`]): item and version.
    undo: Vec<(u64, u64)>,
    /// Where the instance stands with Java mods.
    java: java::State,
    /// What [`mods::stamp`] said when `mods` was read; the same stamp means nothing new.
    /// `None` until the first read is in.
    stamp: Option<mods::Stamp>,
    /// A read is running in the background; `stale` asks for one more after it.
    loading: bool,
    stale: bool,
    search: Entity<InputState>,
    sort: Sort,
    filter: Filter,
    /// Folders of the checked rows.
    selected: HashSet<String>,
    /// The current manifest of each installed Workshop item; empty until Steam answers.
    latest: HashMap<u64, u64>,
    check: Check,
    shell: WeakEntity<Shell>,
    /// The table's scroll position, eased by `ui::smooth_scroll`.
    scroll: UniformListScrollHandle,
    /// The tab's width at the last frame: the selection bar drops its labels when narrow.
    width: Pixels,
    _search: Subscription,
}

impl Mods {
    pub fn new(
        instance: Instance,
        shell: WeakEntity<Shell>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(t!("Search mods")));
        let on_search = cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                cx.notify();
            }
        });
        let mut mods = Self {
            latest: HashMap::new(),
            check: Check::Running,
            // Read in the background: a collection is hundreds of `mod.info` files, and
            // reading them here froze the window on the way into the instance.
            stamp: None,
            loading: false,
            stale: false,
            mods: Vec::new(),
            too_new: Vec::new(),
            undo: Vec::new(),
            java: java::State::None,
            instance,
            search,
            sort: Sort::Name,
            filter: Filter::All,
            selected: HashSet::new(),
            shell,
            scroll: UniformListScrollHandle::new(),
            width: px(f32::MAX),
            _search: on_search,
        };
        mods.check_updates(cx);
        mods.refresh(cx);
        mods
    }

    /// Asks Steam for the current version of the instance's Workshop mods.
    fn check_updates(&mut self, cx: &mut Context<Self>) {
        self.check = Check::Running;
        cx.notify();
        let instance = self.instance.clone();
        cx.spawn(async move |this, cx| {
            let found = mods::latest(&instance).await;
            let _ = this.update(cx, |this, cx| {
                match found {
                    Ok(latest) => {
                        this.latest = latest;
                        this.check = Check::Done;
                    }
                    Err(_) => this.check = Check::Failed,
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Reads the mods from disk again when they changed; the shell calls it on every step of
    /// a download. The read goes to the background (a collection is hundreds of `mod.info`
    /// files) and only one runs at a time.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let stamp = mods::stamp(&self.instance);
        if Some(stamp) == self.stamp {
            return;
        }
        if self.loading {
            self.stale = true;
            return;
        }
        self.loading = true;
        let instance = self.instance.clone();
        // The first read is the page loading: the bar under the title bar says so.
        let first = self.stamp.is_none().then(|| self.shell.clone());
        cx.spawn(async move |this, cx| {
            if let Some(shell) = &first {
                let _ = shell.update(cx, |shell, cx| shell.set_loading(true, cx));
            }
            let found = crate::runtime::blocking(move || {
                let undo = mods::undo(&instance);
                let list = mods::list(&instance);
                let java = java::state(&instance, &list);
                (list, mods::too_new(&instance), undo, java)
            })
            .await;
            if let Some(shell) = &first {
                let _ = shell.update(cx, |shell, cx| shell.set_loading(false, cx));
            }
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                if let Ok((found, too_new, undo, java)) = found {
                    this.mods = found;
                    this.too_new = too_new;
                    this.undo = undo;
                    this.java = java;
                    this.stamp = Some(stamp);
                    // A mod that is gone can't stay checked.
                    this.selected
                        .retain(|folder| this.mods.iter().any(|m| m.folder == *folder));
                    cx.notify();
                }
                if std::mem::take(&mut this.stale) {
                    this.refresh(cx);
                }
            });
        })
        .detach();
    }

    /// The Workshop item of `item` when it has a newer version than the one installed. One
    /// kept at a version on purpose has no update to offer.
    fn update_of(&self, item: &Mod) -> Option<u64> {
        let id = item.workshop.filter(|_| !item.held)?;
        let newest = *self.latest.get(&id)?;
        (newest != mods::GONE && Some(newest) != item.manifest).then_some(id)
    }

    /// Whether the Workshop no longer has `item`'s source: Steam was asked about it and said so
    /// (removed, or hidden by its author). The copy here is then the only one lata can reach.
    /// A mod added after the last check isn't in `latest` at all, which says nothing.
    fn gone(&self, item: &Mod) -> bool {
        item.workshop
            .is_some_and(|id| self.latest.get(&id) == Some(&mods::GONE))
    }

    /// Downloads the current version of `items` through the download manager.
    fn update(&self, items: Vec<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let (instance, shell, handle) = (
            self.instance.clone(),
            self.shell.clone(),
            window.window_handle(),
        );
        // After this update ends: the shell repaints this view as the download starts.
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.update_mods(instance, items, handle, cx)
            });
        });
    }

    /// Puts back the versions the last update replaced (all, or only those of `only`), through
    /// the download manager.
    fn undo(&self, only: Option<Vec<u64>>, window: &mut Window, cx: &mut Context<Self>) {
        let (instance, shell, handle) = (
            self.instance.clone(),
            self.shell.clone(),
            window.window_handle(),
        );
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| shell.undo_mods(instance, only, handle, cx));
        });
    }

    fn shown(&self, cx: &App) -> Vec<&Mod> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        let mut shown: Vec<&Mod> = self
            .mods
            .iter()
            .filter(|item| match self.filter {
                Filter::All => true,
                Filter::On => item.enabled,
                Filter::Off => !item.enabled,
                Filter::Updates => self.update_of(item).is_some(),
            })
            .filter(|item| {
                query.is_empty()
                    || [&item.name, &item.id, &item.author]
                        .iter()
                        .any(|text| text.to_lowercase().contains(&query))
            })
            .collect();
        // `mods` is already by name; the sort is stable, so names break ties.
        match self.sort {
            Sort::Name => {}
            Sort::EnabledFirst => shown.sort_by_key(|item| !item.enabled),
            Sort::Added => shown.sort_by_key(|item| std::cmp::Reverse(item.added)),
            Sort::Updated => shown.sort_by_key(|item| std::cmp::Reverse(item.updated)),
        }
        shown
    }

    fn toggle(&mut self, id: &str, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        match mods::set_enabled(&self.instance, id, on) {
            Ok(missing) if !missing.is_empty() => window.push_notification(
                toast::heads_up(t!("It needs mods you don't have yet"), missing.join(", ")),
                cx,
            ),
            Ok(_) => {}
            Err(err) => window.push_notification(
                toast::failed_because(t!("Hmm, that switch didn't stick"), err),
                cx,
            ),
        }
        self.refresh(cx);
    }

    fn remove(&mut self, folder: &str, window: &mut Window, cx: &mut Context<Self>) {
        match mods::remove(&self.instance, folder) {
            Ok(()) => window.push_notification(toast::done(t!("Poof! That mod is gone")), cx),
            Err(err) => window.push_notification(
                toast::failed_because(t!("That mod wouldn't budge"), err),
                cx,
            ),
        }
        self.refresh(cx);
    }

    fn toolbar(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let this = cx.entity().downgrade();
        let (sort, filter) = (self.sort, self.filter);
        let (instance, shell) = (self.instance.clone(), self.shell.clone());
        // Each item once: an item can hold several mods.
        let mut outdated: Vec<u64> = self.mods.iter().filter_map(|m| self.update_of(m)).collect();
        outdated.sort_unstable();
        outdated.dedup();
        let pick = |id: &'static str, icon: IconName, label: &'static str| {
            // The Library's sort and group buttons, so both toolbars read the same.
            Button::new(id)
                .outline()
                .h(CONTROL)
                .cursor_pointer()
                .icon(icon)
                .label(label)
        };
        div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            // Two groups that wrap whole: finding mods (search, sort, filter) and changing the
            // instance (load order, updates, add). When both don't fit on one line, the second
            // drops below, instead of the search shrinking to nothing.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .flex_basis(px(QUERY))
                    .min_w(px(QUERY))
                    .child(
                        // The input's own frame only comes in 32 or 44 px, so ours carries the border
                        // at the toolbar's height and the input sits inside it bare.
                        div()
                            .flex_1()
                            .min_w(px(SEARCH))
                            .flex()
                            .items_center()
                            .gap_1()
                            .h(CONTROL)
                            .pl_3()
                            .rounded(theme.radius)
                            .bg(theme.background)
                            .border_1()
                            .border_color(theme.input)
                            .child(
                                Icon::new(IconName::Search)
                                    .size_4()
                                    .text_color(theme.muted_foreground),
                            )
                            .child(Input::new(&self.search).appearance(false).cleanable(true)),
                    )
                    .child(
                        pick("mods-sort", IconName::ArrowDownWideNarrow, sort.label())
                            .dropdown_menu({
                                let this = this.clone();
                                move |mut menu, _, _| {
                                    for option in
                                        [Sort::Name, Sort::EnabledFirst, Sort::Added, Sort::Updated]
                                    {
                                        let this = this.clone();
                                        menu = menu.item(
                                            PopupMenuItem::new(option.label())
                                                .checked(option == sort)
                                                .on_click(move |_, _, cx| {
                                                    let _ = this.update(cx, |mods, cx| {
                                                        mods.sort = option;
                                                        cx.notify();
                                                    });
                                                }),
                                        );
                                    }
                                    menu
                                }
                            }),
                    )
                    .child(
                        pick("mods-filter", IconName::Layers, filter.label()).dropdown_menu(
                            move |mut menu, _, _| {
                                for option in
                                    [Filter::All, Filter::On, Filter::Off, Filter::Updates]
                                {
                                    let this = this.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(option.label())
                                            .checked(option == filter)
                                            .on_click(move |_, _, cx| {
                                                let _ = this.update(cx, |mods, cx| {
                                                    mods.filter = option;
                                                    cx.notify();
                                                });
                                            }),
                                    );
                                }
                                menu
                            },
                        ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .flex_1()
                    .flex_basis(px(ACTIONS_ROW))
                    .min_w(px(ACTIONS_ROW))
                    // With two mods on there is an order to speak of.
                    .when(self.mods.iter().filter(|m| m.enabled).count() > 1, |this| {
                        let (instance, mods) = (self.instance.clone(), cx.entity().downgrade());
                        this.child(
                            pick("mods-order", IconName::ListOrdered, t!("Load order")).on_click(
                                move |_, window, cx| {
                                    let mods = mods.clone();
                                    super::order::open(
                                        instance.clone(),
                                        move |_, cx| {
                                            let _ = mods.update(cx, |mods, cx| mods.refresh(cx));
                                        },
                                        window,
                                        cx,
                                    )
                                },
                            ),
                        )
                    })
                    // Load order on the left, updates and adding on the right.
                    .child(div().flex_1())
                    // Always there, saying where the check for updates stands, so its spot
                    // doesn't appear out of nowhere when Steam answers.
                    .when(!self.undo.is_empty(), |this| {
                        this.child(
                            Button::new("mods-undo")
                                .ghost()
                                .h(CONTROL)
                                .cursor_pointer()
                                .icon(Icon::empty().path("icons/undo-2.svg"))
                                .label(t!("Undo update"))
                                .tooltip(t!("Put back the versions from before the last update"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.undo(None, window, cx)),
                                ),
                        )
                    })
                    .child(self.updates(outdated, cx))
                    .child(
                        FancyButton::new("mod-add", t!("Install mod"))
                            .small()
                            .icon(IconName::Plus)
                            .on_click(move |_, window, cx| {
                                super::mod_add::open(instance.clone(), shell.clone(), window, cx)
                            }),
                    ),
            )
    }

    /// The mods the game skips for being newer than it, above the table: they don't show in it.
    /// Turns Java mods on for the instance and reads the tab again. Turning them off is in
    /// the instance's settings: once on there is nothing left to decide here.
    fn turn_on_java(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let note = match super::store::set_java_mods(&self.instance.dir, true) {
            Ok(()) => toast::done(t!("Java mods are on! Each one asks you before it loads")),
            Err(err) => toast::failed_because(t!("Hmm, that didn't save"), err),
        };
        window.push_notification(note, cx);
        self.refresh(cx);
    }

    /// Asks before turning Java mods on.
    fn confirm_java(&self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        super::java_view::confirm(window, cx, move |window, cx| {
            let _ = view.update(cx, |this, cx| this.turn_on_java(window, cx));
        });
    }

    /// Where the instance stands with Java mods, with the one thing to do about it: add the
    /// loader they need, or turn them on. Nothing when it has none or they are already on.
    fn java_note(&self, cx: &mut Context<Self>) -> Option<Div> {
        let (title, detail, action): (String, String, AnyElement) = match &self.java {
            java::State::None | java::State::On => return None,
            java::State::NeedsLoader(names) => {
                let (instance, shell) = (self.instance.clone(), self.shell.clone());
                (
                    tn!(
                        names.len(),
                        "{n} Java mod needs ZombieBuddy to run",
                        "{n} Java mods need ZombieBuddy to run"
                    ),
                    names.join(", "),
                    FancyButton::new("java-add-loader", t!("Install ZombieBuddy"))
                        .small()
                        .icon(IconName::Plus)
                        .on_click(move |_, window, cx| {
                            let (instance, handle) = (instance.clone(), window.window_handle());
                            let _ = shell.update(cx, |shell, cx| {
                                shell.add_mod(instance, java::LOADER, handle, cx)
                            });
                        })
                        .into_any_element(),
                )
            }
            java::State::Off => (
                t!("Java mods are off in this instance").to_owned(),
                t!(
                    "ZombieBuddy is here, but it only loads Java mods when the game starts with \
                 it hooked in."
                )
                .to_owned(),
                FancyButton::new("java-on", t!("Turn on"))
                    .small()
                    .on_click(cx.listener(|this, _, window, cx| this.confirm_java(window, cx)))
                    .into_any_element(),
            ),
        };
        let theme = cx.theme();
        Some(
            div()
                .flex()
                .items_center()
                .gap_3()
                .p_3()
                .rounded(theme.radius_lg)
                .bg(theme.primary.opacity(0.1))
                .text_sm()
                .child(
                    Icon::new(IconName::Info)
                        .size_4()
                        .flex_none()
                        .text_color(theme.primary),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                        .child(div().text_color(theme.muted_foreground).child(detail)),
                )
                .child(div().flex_none().child(action)),
        )
    }

    /// Says which mods the Workshop no longer has, so nobody removes one thinking it can come
    /// back: there is nowhere to download it from again.
    fn gone_note(&self, cx: &App) -> Option<Div> {
        let mut gone: Vec<&str> = self
            .mods
            .iter()
            .filter(|item| self.gone(item))
            .map(|item| item.name.as_str())
            .collect();
        gone.dedup();
        if gone.is_empty() {
            return None;
        }
        let theme = cx.theme();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(theme.radius_lg)
                .bg(theme.warning.opacity(0.12))
                .text_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui::warning_text(cx))
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(tn!(
                            gone.len(),
                            "Heads up! {n} mod left the Workshop, your copy is the only one",
                            "Heads up! {n} mods left the Workshop, your copies are the only ones"
                        )),
                )
                .child(div().pl_6().child(gone.join(", ")))
                .child(div().pl_6().text_color(theme.muted_foreground).child(t!(
                    "They still work, but can't update or change version. Removing one is for good."
                ))),
        )
    }

    fn too_new_note(&self, cx: &App) -> Option<Div> {
        if self.too_new.is_empty() {
            return None;
        }
        let theme = cx.theme();
        Some(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_3()
                .rounded(theme.radius_lg)
                .bg(theme.warning.opacity(0.12))
                .text_sm()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(ui::warning_text(cx))
                        .child(Icon::new(IconName::TriangleAlert).size_4().flex_none())
                        .child(tn!(
                            self.too_new.len(),
                            "Heads up! {n} mod needs a newer game, update this instance",
                            "Heads up! {n} mods need a newer game, update this instance"
                        )),
                )
                .child(div().pl_6().child(self.too_new.join(", "))),
        )
    }

    /// The updates button: checking (a spinning icon), how many to update, up to date, or
    /// couldn't check. The last two check again on a click.
    fn updates(&self, outdated: Vec<u64>, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let button = Button::new("mods-updates")
            .outline()
            .h(CONTROL)
            .cursor_pointer();
        match (self.check, outdated.len()) {
            // Nothing to press while Steam is asked: disabled, with the arrow cursor.
            (Check::Running, _) => button
                .disabled(true)
                .cursor_default()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            Icon::new(IconName::LoaderCircle)
                                .size_4()
                                .text_color(theme.muted_foreground)
                                .with_animation(
                                    "mods-checking",
                                    // Indeterminate, so it loops, eased like the page loader.
                                    Animation::new(std::time::Duration::from_millis(1000))
                                        .repeat()
                                        .with_easing(ease_in_out),
                                    |icon, t| icon.rotate(percentage(t)),
                                ),
                        )
                        .child(t!("Checking for updates")),
                )
                .into_any_element(),
            (Check::Done, 0) => button
                .icon(IconName::Check)
                .label(t!("Up to date"))
                .tooltip(t!("Check again"))
                .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx)))
                .into_any_element(),
            (Check::Done, count) => {
                let label = tn!(count, "Update {n} mod", "Update {n} mods");
                button
                    .icon(IconName::Download)
                    .label(label)
                    .text_color(theme.primary)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.update(outdated.clone(), window, cx)
                    }))
                    .into_any_element()
            }
            (Check::Failed, _) => button
                .icon(Icon::empty().path("icons/history.svg"))
                .label(t!("Couldn't check"))
                .tooltip(t!("Steam didn't answer. Click to try again"))
                .on_click(cx.listener(|this, _, _, cx| this.check_updates(cx)))
                .into_any_element(),
        }
    }

    /// The checked mods, in the list's order.
    fn chosen(&self) -> Vec<&Mod> {
        self.mods
            .iter()
            .filter(|m| self.selected.contains(&m.folder))
            .collect()
    }

    fn toggle_chosen(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<String> = self.chosen().iter().map(|m| m.id.clone()).collect();
        match mods::set_enabled_all(&self.instance, &ids, on) {
            Ok(missing) if !missing.is_empty() => window.push_notification(
                toast::heads_up(t!("Some need mods you don't have yet"), missing.join(", ")),
                cx,
            ),
            Ok(_) => {}
            Err(err) => window.push_notification(
                toast::failed_because(t!("Hmm, those switches didn't stick"), err),
                cx,
            ),
        }
        self.refresh(cx);
    }

    fn remove_chosen(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let folders: Vec<String> = self.chosen().iter().map(|m| m.folder.clone()).collect();
        let failed = folders
            .iter()
            .find_map(|folder| mods::remove(&self.instance, folder).err());
        match failed {
            None => window.push_notification(
                toast::done(match folders.len() {
                    1 => t!("Poof! That mod is gone").to_owned(),
                    many => t!("Poof! {many} mods are gone", many = many),
                }),
                cx,
            ),
            Some(err) => {
                window.push_notification(toast::failed_because(t!("A mod wouldn't budge"), err), cx)
            }
        }
        self.refresh(cx);
    }

    /// The table's first row: check all, and the column names.
    fn header(&self, shown: &[Mod], cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let all = !shown.is_empty() && shown.iter().all(|m| self.selected.contains(&m.folder));
        let folders: Vec<String> = shown.iter().map(|m| m.folder.clone()).collect();
        let name = |text: &'static str| {
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap_3()
            .h(px(44.))
            .px_4()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .child(
                Checkbox::new("mods-all")
                    .cursor_pointer()
                    .checked(all)
                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                        for folder in &folders {
                            if *on {
                                this.selected.insert(folder.clone());
                            } else {
                                this.selected.remove(folder);
                            }
                        }
                        cx.notify();
                    })),
            )
            .child(name(t!("Mod")).flex_1())
            .child(name(t!("Version")).w(VERSION))
            .child(name(t!("Actions")).w(ACTIONS).flex().justify_end())
    }

    fn row(&self, item: &Mod, last: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let checked = self.selected.contains(&item.folder);
        let picture = match &item.icon {
            Some(path) => div().flex_none().size(ICON).child(
                img(path.clone())
                    .size_full()
                    .rounded(px(8.))
                    .object_fit(ObjectFit::Cover)
                    // Pictures decode as rows scroll into view.
                    .with_loading(|| {
                        Skeleton::new()
                            .size_full()
                            .rounded(px(8.))
                            .into_any_element()
                    }),
            ),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(ICON)
                .rounded(px(8.))
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::Puzzle).size_4()),
        };
        let two = |top: String, bottom: String| {
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .truncate()
                        .child(top),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(bottom),
                )
        };
        let author = if item.author.is_empty() {
            item.id.clone()
        } else {
            t!("by {author}", author = item.author)
        };
        // A mod read from `common/`, or a Build 41 one, isn't tied to one version folder.
        let version = item.version.clone().unwrap_or(t!("Any version").into());
        let key = |what: &str| SharedString::from(format!("{what}-{}", item.folder));
        let (id, folder, pick) = (item.id.clone(), item.folder.clone(), item.folder.clone());
        let (dir, mod_id, workshop) = (item.dir.clone(), item.id.clone(), item.workshop);
        let update = self.update_of(item);
        let (held, installed, gone) = (item.held, item.manifest, self.gone(item));
        let (title, instance, shell) = (
            SharedString::from(item.name.clone()),
            self.instance.clone(),
            self.shell.clone(),
        );
        let view_shell = shell.clone();
        let versions = move |window: &mut Window, cx: &mut App| {
            if let Some(workshop) = workshop {
                super::versions::open(
                    instance.clone(),
                    (workshop, title.clone(), installed),
                    shell.clone(),
                    window,
                    cx,
                );
            }
        };
        let held_versions = versions.clone();
        div()
            // A uniform list doesn't stretch its rows like a flex column did.
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .when(!last, |row| row.border_b_1().border_color(theme.border))
            .when(checked, |row| row.bg(theme.primary.opacity(0.1)))
            // The table clips with a plain rectangle, so its last row rounds its own corners.
            .when(last, |row| row.rounded_b(theme.radius_lg - px(1.)))
            .child(
                Checkbox::new(key("check"))
                    .cursor_pointer()
                    .checked(checked)
                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                        if *on {
                            this.selected.insert(pick.clone());
                        } else {
                            this.selected.remove(&pick);
                        }
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_3()
                    .child(picture)
                    .child(two(item.name.clone(), author)),
            )
            .child(two(version, item.folder.clone()).w(VERSION))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .w(ACTIONS)
                    .when_some(update, |this, update| {
                        this.child(
                            Button::new(key("update"))
                                .ghost()
                                .cursor_pointer()
                                .icon(IconName::Download)
                                .text_color(theme.primary)
                                .tooltip(t!("Update available"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.update(vec![update], window, cx)
                                })),
                        )
                    })
                    // Kept at an older version on purpose: says so where its update would be.
                    .when(held, |this| {
                        this.child(
                            Button::new(key("held"))
                                .ghost()
                                .cursor_pointer()
                                .icon(Icon::empty().path("icons/history.svg"))
                                .tooltip(t!("Staying on an older version. Click to change it"))
                                .on_click(move |_, window, cx| held_versions(window, cx)),
                        )
                    })
                    .child(
                        Switch::new(key("on"))
                            .cursor_pointer()
                            .checked(item.enabled)
                            .on_click(cx.listener(move |this, on: &bool, window, cx| {
                                this.toggle(&id, *on, window, cx)
                            })),
                    )
                    .child(
                        Button::new(key("remove"))
                            .ghost()
                            .cursor_pointer()
                            .icon(IconName::Trash)
                            .tooltip(t!("Remove"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.remove(&folder, window, cx)
                            })),
                    )
                    .child(
                        Button::new(key("more"))
                            .ghost()
                            .cursor_pointer()
                            .icon(IconName::EllipsisVertical)
                            .dropdown_menu(move |menu, _, _| {
                                let (dir, mod_id) = (dir.clone(), mod_id.clone());
                                let (versions, view_shell) = (versions.clone(), view_shell.clone());
                                // Grouped like the instance menus: the mod on the Workshop,
                                // then its files here, then Steam outside lata.
                                menu.when_some(workshop.filter(|_| !gone), |menu, workshop| {
                                    menu.item(
                                        PopupMenuItem::new(t!("View in Workshop"))
                                            .icon(IconName::Compass)
                                            .on_click(move |_, window, cx| {
                                                Shell::view_mod(
                                                    view_shell.clone(),
                                                    workshop,
                                                    window,
                                                    cx,
                                                )
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(t!("Change version"))
                                            .icon(Icon::empty().path("icons/history.svg"))
                                            .on_click(move |_, window, cx| versions(window, cx)),
                                    )
                                    .separator()
                                })
                                .item(
                                    PopupMenuItem::new(t!("Open folder"))
                                        .icon(IconName::FolderOpen)
                                        .on_click(move |_, _, cx| cx.open_with_system(&dir)),
                                )
                                .item(
                                    PopupMenuItem::new(t!("Copy mod id"))
                                        .icon(IconName::Copy)
                                        .on_click(move |_, window, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                mod_id.clone(),
                                            ));
                                            window.push_notification(
                                                toast::done(t!("Copied! Paste it anywhere")),
                                                cx,
                                            );
                                        }),
                                )
                                .when_some(workshop, |menu, workshop| {
                                    menu.separator().item(
                                        PopupMenuItem::new(t!("Open on Steam"))
                                            .icon(IconName::ExternalLink)
                                            .on_click(move |_, _, cx| {
                                                cx.open_url(&format!(
                                                    "https://steamcommunity.com/sharedfiles/filedetails/?id={workshop}"
                                                ))
                                            }),
                                    )
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    /// What to do with the checked rows, floating under the table while any is. Where the
    /// labels don't fit in `room`, only the icons show, each with its name as a tooltip.
    fn selection(&self, room: Pixels, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let chosen = self.chosen();
        let (any_off, any_on) = (
            chosen.iter().any(|m| !m.enabled),
            chosen.iter().any(|m| m.enabled),
        );
        // Each item once: an item can hold several mods. Held mods stay out, like in
        // "Update all".
        let mut outdated: Vec<u64> = chosen.iter().filter_map(|m| self.update_of(m)).collect();
        outdated.sort_unstable();
        outdated.dedup();
        let mut undoable: Vec<u64> = chosen
            .iter()
            .filter_map(|m| m.workshop)
            .filter(|id| self.undo.iter().any(|(undo, _)| undo == id))
            .collect();
        undoable.sort_unstable();
        undoable.dedup();
        // ponytail: widths measured by eye at Inter 14 px in English; another language adds its
        // labels' extra length, guessed per character (wide ones for CJK). Measure the shaped
        // text if a language still overflows.
        let guess = |text: &str| -> f32 {
            text.chars()
                .map(|c| if c >= '\u{2e80}' { 14. } else { 7.5 })
                .sum()
        };
        let longer = |english: &str, shown: &str| (guess(shown) - guess(english)).max(0.);
        let needed = 600.
            + longer("Clear", t!("Clear"))
            + longer("Turn on", t!("Turn on"))
            + longer("Turn off", t!("Turn off"))
            + longer("Remove", t!("Remove"))
            + longer(
                "2 mods selected",
                &tn!(2, "{n} mod selected", "{n} mods selected"),
            )
            + if outdated.is_empty() {
                0.
            } else {
                100. + longer("Update", t!("Update"))
            }
            + if undoable.is_empty() {
                0.
            } else {
                140. + longer("Undo update", t!("Undo update"))
            };
        let compact = room < px(needed);
        let count = match (chosen.len(), compact) {
            (many, false) => tn!(many, "{n} mod selected", "{n} mods selected"),
            (many, true) => t!("{many} selected", many = many),
        };
        let act = |id: &'static str, label: &'static str, icon: Icon, enabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .gap_2()
                .h(px(36.))
                .map(|this| {
                    if compact {
                        this.w(px(36.))
                    } else {
                        this.px_3()
                    }
                })
                .rounded(theme.radius)
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .child(icon.size_4())
                .when(!compact, |this| this.child(label))
                .when(compact, |this| {
                    this.tooltip(move |window, cx| Tooltip::new(label).build(window, cx))
                })
                .map(|this| {
                    if enabled {
                        this.cursor_pointer().hover(|style| style.bg(theme.muted))
                    } else {
                        this.opacity(0.5)
                    }
                })
        };
        let rule = || {
            div()
                .flex_none()
                .w(px(1.))
                .h(px(24.))
                .mx_1()
                .bg(theme.border)
        };
        div()
            .occlude()
            .flex()
            .items_center()
            .gap_1()
            .max_w_full()
            .h(px(56.))
            .pl_5()
            .pr_2()
            .rounded(theme.radius_lg * 1.5)
            .bg(theme.tokens.popover)
            .border_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(
                div()
                    .flex_none()
                    .pr_2()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(count),
            )
            .child(
                act(
                    "selection-clear",
                    t!("Clear"),
                    Icon::new(IconName::Close),
                    true,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.selected.clear();
                    cx.notify();
                })),
            )
            .child(rule())
            .child(
                act(
                    "selection-on",
                    t!("Turn on"),
                    Icon::new(IconName::Check),
                    any_off,
                )
                .when(any_off, |this| {
                    this.on_click(
                        cx.listener(|this, _, window, cx| this.toggle_chosen(true, window, cx)),
                    )
                }),
            )
            .child(
                act(
                    "selection-off",
                    t!("Turn off"),
                    Icon::new(IconName::Minus),
                    any_on,
                )
                .when(any_on, |this| {
                    this.on_click(
                        cx.listener(|this, _, window, cx| this.toggle_chosen(false, window, cx)),
                    )
                }),
            )
            .when(!outdated.is_empty() || !undoable.is_empty(), |this| {
                this.child(rule())
            })
            .when(!outdated.is_empty(), |this| {
                this.child(
                    act(
                        "selection-update",
                        t!("Update"),
                        Icon::new(IconName::Download),
                        true,
                    )
                    .text_color(theme.primary)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.update(outdated.clone(), window, cx)
                    })),
                )
            })
            .when(!undoable.is_empty(), |this| {
                this.child(
                    act(
                        "selection-undo",
                        t!("Undo update"),
                        Icon::empty().path("icons/undo-2.svg"),
                        true,
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.undo(Some(undoable.clone()), window, cx)
                    })),
                )
            })
            .child(rule())
            .child(
                act(
                    "selection-remove",
                    t!("Remove"),
                    Icon::new(IconName::Trash),
                    true,
                )
                .text_color(theme.danger)
                .on_click(cx.listener(|this, _, window, cx| this.remove_chosen(window, cx))),
            )
    }
}

impl Render for Mods {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = self.scroll.0.borrow().base_handle.clone();
        ui::smooth_scroll("mods-scroll", &base, window, cx);
        // Still reading: the shell's bar shows it, and "No mods yet" would be a lie.
        if self.stamp.is_none() {
            return div().flex_1().into_any_element();
        }
        let shown: Vec<Mod> = self.shown(cx).into_iter().cloned().collect();
        // No mods yet: the page says so and offers the one thing to do. A search, a sort and a
        // filter with nothing to work on would only be noise.
        if self.mods.is_empty() {
            let (instance, shell) = (self.instance.clone(), self.shell.clone());
            // The same panel as the Worlds tab, so the tabs don't jump when switching.
            return div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap_2()
                .children(self.too_new_note(cx))
                .children(self.gone_note(cx))
                .children(self.java_note(cx))
                .child(
                    Empty::new(
                        IconName::Puzzle,
                        t!("No mods yet"),
                        t!("Install one from the Workshop and it lands here."),
                    )
                    .action(
                        FancyButton::new("mod-add-first", t!("Install mod"))
                            .small()
                            .icon(IconName::Plus)
                            .on_click(move |_, window, cx| {
                                super::mod_add::open(instance.clone(), shell.clone(), window, cx)
                            }),
                    ),
                )
                .into_any_element();
        }
        let list = if shown.is_empty() {
            Empty::new(
                IconName::Search,
                t!("No mods match"),
                t!("Try another name, or change the filter."),
            )
            .into_any_element()
        } else {
            div()
                .flex()
                .flex_col()
                .min_h_0()
                .rounded(cx.theme().radius_lg)
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .overflow_hidden()
                .child(self.header(&shown, cx))
                // Only the rows in view are built: a collection brings hundreds of mods, each
                // with a picture to decode.
                .child(
                    uniform_list(
                        "mods-list",
                        shown.len(),
                        cx.processor(|this, range: Range<usize>, _, cx| {
                            let shown = this.shown(cx);
                            let last = shown.len().saturating_sub(1);
                            let picked: Vec<(usize, Mod)> = range
                                .filter_map(|ix| shown.get(ix).map(|m| (ix, (*m).clone())))
                                .collect();
                            picked
                                .iter()
                                .map(|(ix, item)| this.row(item, *ix == last, cx))
                                .collect()
                        }),
                    )
                    // Rows times their height, capped by the room left: the table hugs a short
                    // list and scrolls a long one. The default sizing gives it no height at all.
                    .with_sizing_behavior(ListSizingBehavior::Infer)
                    .track_scroll(&self.scroll)
                    // Scroll room past the last row while the selection bar floats over the
                    // table, so that row can still come out from under it.
                    .when(!self.selected.is_empty(), |list| list.pb(px(80.)))
                    .min_h_0(),
                )
                .into_any_element()
        };
        let on = self.mods.iter().filter(|m| m.enabled).count();
        // Floats over the bottom of the table without taking room from it.
        let bar = (!self.selected.is_empty()).then(|| {
            div()
                .absolute()
                .bottom(px(16.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(ui::enter(self.selection(self.width, cx), "mods-selection"))
        });
        // The tab's own width, for `compact`; read after layout, so a resize shows one frame
        // late. A notify while painting is dropped, hence the next frame.
        let this = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    if this.width != bounds.size.width {
                        this.width = bounds.size.width;
                        cx.on_next_frame(window, |_, _, cx| cx.notify());
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(self.toolbar(cx))
            .children(self.too_new_note(cx))
            .children(self.gone_note(cx))
            .children(self.java_note(cx))
            .when(!self.mods.is_empty(), |this| {
                // How many show (the search and filter apply), and how many the game loads.
                let total = self.mods.len();
                let count = if shown.len() == total {
                    tn!(total, "{n} mod", "{n} mods")
                } else {
                    tn!(
                        total,
                        "{shown} of {n} mod",
                        "{shown} of {n} mods",
                        shown = shown.len()
                    )
                };
                this.child(
                    div()
                        .flex()
                        .justify_between()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(count)
                        .child(t!("{on} enabled", on = on)),
                )
            })
            .child(measure)
            .child(list)
            .children(bar)
            .into_any_element()
    }
}
