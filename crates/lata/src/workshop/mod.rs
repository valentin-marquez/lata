//! The Workshop page: search and browse Project Zomboid's Workshop and add a mod to an instance,
//! without leaving lata (Modrinth's discover page: filters on the left, search and sort above
//! the results). Steam answers searches from anyone, but only a signed-in account can read who
//! made each mod, so the page asks to sign in first.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::spring;
use gpui_kit::component::button::Button;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Disableable, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use steam::workshop::{Item, Page, Query};

#[path = "bbcode.model.rs"]
mod bbcode;
#[path = "detail.view.rs"]
mod detail;

use crate::account::Account;
use crate::instance::{APP, branches, mods};
use crate::shell::Shell;
use crate::ui::{self, Empty, FancyButton, Meta};

/// Height of the toolbar's controls: the primary button's middle size.
const CONTROL: Pixels = px(36.);
const SIDEBAR: Pixels = px(208.);
const PICTURE: Pixels = px(88.);
/// The narrowest page that keeps the filters in their own column.
const WIDE: Pixels = px(860.);
/// Results asked for at a time; "Load more" asks for the next ones.
const PAGE: u32 = 20;
/// How long typing has to pause before the search goes out.
const PAUSE: Duration = Duration::from_millis(350);
/// Tags shown on a result; Workshop authors stack many.
const SHOWN_TAGS: usize = 5;

/// The build tags, offered apart from the rest: a mod for the other build doesn't load.
const BUILDS: [&str; 2] = ["Build 42", "Build 41"];
/// The Workshop's categories for Project Zomboid, as its authors tag them (`building` really is
/// lowercase there). Taken from the tags on its 1000 most subscribed mods.
const TAGS: [&str; 28] = [
    "Animals",
    "Audio",
    "Balance",
    "building",
    "Clothing/Armor",
    "Farming",
    "Food",
    "Framework",
    "Hardmode",
    "Interface",
    "Items",
    "Language/Translation",
    "Literature",
    "Map",
    "Military",
    "Misc",
    "Models",
    "Multiplayer",
    "Pop Culture",
    "QOL",
    "Realistic",
    "Silly/Fun",
    "Skills",
    "Textures",
    "Traits",
    "Vehicles",
    "Weapons",
    "WIP",
];

#[derive(Clone, Copy, PartialEq)]
enum Sort {
    Relevance,
    Trending,
    Popular,
    Newest,
    Updated,
}

impl Sort {
    const ALL: [Sort; 5] = [
        Sort::Relevance,
        Sort::Trending,
        Sort::Popular,
        Sort::Newest,
        Sort::Updated,
    ];

    fn label(self) -> &'static str {
        match self {
            Sort::Relevance => "Relevance",
            Sort::Trending => "Trending",
            Sort::Popular => "Most subscribed",
            Sort::Newest => "Newest",
            Sort::Updated => "Recently updated",
        }
    }

    /// What Steam is asked for. Relevance needs words to rank against; with none it is what
    /// people pick up this week.
    fn steam(self, text: &str) -> steam::workshop::Sort {
        use steam::workshop::Sort as S;
        match self {
            Sort::Relevance if !text.trim().is_empty() => S::Relevance,
            Sort::Relevance | Sort::Trending => S::Trend { days: 7 },
            Sort::Popular => S::Popular,
            Sort::Newest => S::Newest,
            Sort::Updated => S::Updated,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Fetch {
    /// A filter or the sort changed: the first page, now.
    Fresh,
    /// The search text changed: the first page, once typing pauses.
    Typed,
    /// The page after the ones shown.
    More,
}

/// One logged-on session, kept between searches: logging on takes about a second. It belongs
/// to the account in the first field.
type Session = Arc<tokio::sync::Mutex<Option<(u64, steam::Client)>>>;

pub struct Workshop {
    shell: WeakEntity<Shell>,
    search: Entity<InputState>,
    sort: Sort,
    /// `None` for mods of any build.
    build: Option<&'static str>,
    tags: Vec<&'static str>,
    /// Categories left out: no result carries any of them.
    excluded: Vec<&'static str>,
    results: Vec<Item>,
    total: u32,
    /// Where the next page starts; `None` when there is none.
    next: Option<String>,
    loading: bool,
    /// Why the last search failed, and whether it was the next page (kept results stay).
    failed: Option<(String, Fetch)>,
    /// Authors' names by SteamID, kept for the whole visit.
    authors: HashMap<u64, String>,
    pictures: HashMap<u64, PathBuf>,
    /// Items whose picture was looked for, found or not.
    pictured: HashSet<u64>,
    session: Session,
    /// The account the results were searched with; a new one searches again.
    who: Option<u64>,
    /// Filter groups folded shut, by title.
    folded: HashSet<&'static str>,
    /// The mod page open over the results, and what it asks for.
    open: Option<(Entity<detail::Detail>, Subscription)>,
    /// The search running; a new one drops it, which cancels it.
    task: Option<Task<()>>,
    scroll: ScrollHandle,
    _search: Subscription,
}

impl Workshop {
    pub fn new(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search the Workshop"));
        let on_search = cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
            if let InputEvent::Change = event {
                this.run(Fetch::Typed, cx);
            }
        });
        Self {
            shell,
            search,
            sort: Sort::Relevance,
            // Build 42 is what Steam installs today.
            build: Some(BUILDS[0]),
            tags: Vec::new(),
            excluded: Vec::new(),
            results: Vec::new(),
            total: 0,
            next: None,
            loading: false,
            failed: None,
            authors: HashMap::new(),
            pictures: HashMap::new(),
            pictured: HashSet::new(),
            session: Arc::default(),
            who: None,
            folded: HashSet::new(),
            open: None,
            task: None,
            scroll: ScrollHandle::new(),
            _search: on_search,
        }
    }

    /// Opens mod `id`'s page over the results; `seed` is what the results knew about it.
    pub fn visit(&mut self, id: u64, seed: Option<Item>, cx: &mut Context<Self>) {
        let picture = self.pictures.get(&id).cloned();
        let author = seed
            .as_ref()
            .and_then(|item| self.authors.get(&item.author).cloned());
        let (shell, session) = (self.shell.clone(), self.session.clone());
        let page = cx.new(|cx| detail::Detail::new(id, seed, picture, author, shell, session, cx));
        let asks = cx.subscribe(&page, |this: &mut Self, _, event: &detail::Event, cx| {
            match event {
                detail::Event::Back => this.open = None,
                detail::Event::Show(id) => {
                    let seed = this.results.iter().find(|item| item.id == *id).cloned();
                    this.visit(*id, seed, cx);
                }
            }
            cx.notify();
        });
        self.open = Some((page, asks));
        cx.notify();
    }

    /// Back to the results from a mod's page; whether one was open.
    pub fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.open.take().is_some();
        cx.notify();
        open
    }

    fn account(&self, cx: &App) -> Option<Account> {
        self.shell.upgrade()?.read(cx).account().cloned()
    }

    /// Asks Steam for what the filters, the sort and the search text describe.
    fn run(&mut self, how: Fetch, cx: &mut Context<Self>) {
        let Some(account) = self.account(cx) else {
            return;
        };
        let text = self.search.read(cx).value().trim().to_owned();
        let query = Query {
            app: APP,
            sort: self.sort.steam(&text),
            text,
            tags: self
                .build
                .into_iter()
                .chain(self.tags.iter().copied())
                .map(String::from)
                .collect(),
            excluded: self.excluded.iter().copied().map(String::from).collect(),
            cursor: match how {
                Fetch::More => self.next.clone(),
                _ => None,
            },
            count: PAGE,
        };
        self.loading = true;
        self.failed = None;
        cx.notify();
        let (session, shell) = (self.session.clone(), self.shell.clone());
        let known: Vec<u64> = self.authors.keys().copied().collect();
        self.task = Some(cx.spawn(async move |this, cx| {
            if how == Fetch::Typed {
                cx.background_executor().timer(PAUSE).await;
            }
            // The bar under the title bar, as any page that loads.
            let _ = shell.update(cx, |shell, cx| shell.set_loading(true, cx));
            let found = search(account, session, query, known).await;
            let _ = shell.update(cx, |shell, cx| shell.set_loading(false, cx));
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match found {
                    Ok((page, names)) => this.show(page, names, how != Fetch::More, cx),
                    Err(err) => {
                        // A new search that failed shows only why: the old results don't
                        // match the filters on screen any more.
                        if how != Fetch::More {
                            this.results.clear();
                            this.next = None;
                        }
                        this.failed = Some((err, how));
                    }
                }
                cx.notify();
            });
        }));
    }

    fn show(
        &mut self,
        page: Page,
        names: HashMap<u64, String>,
        fresh: bool,
        cx: &mut Context<Self>,
    ) {
        self.authors.extend(names);
        if fresh {
            self.results.clear();
            self.scroll.set_offset(point(px(0.), px(0.)));
        }
        // Steam's pages can overlap by an item or two.
        let new: Vec<Item> = page
            .items
            .into_iter()
            .filter(|item| self.results.iter().all(|shown| shown.id != item.id))
            .collect();
        self.fetch_pictures(&new, cx);
        self.results.extend(new);
        self.total = page.total;
        self.next = page.next;
    }

    /// Downloads the pictures of `items` (cached on disk like the trending ones), all at once.
    fn fetch_pictures(&self, items: &[Item], cx: &mut Context<Self>) {
        let wanted: Vec<(u64, Option<String>)> = items
            .iter()
            .filter(|item| !self.pictured.contains(&item.id))
            .map(|item| (item.id, item.preview.clone()))
            .collect();
        cx.spawn(async move |this, cx| {
            let found = crate::runtime::spawn(async move {
                let http = steam::http();
                let mut running = tokio::task::JoinSet::new();
                for (id, url) in wanted {
                    let http = http.clone();
                    running.spawn(async move { (id, mods::picture(&http, url.as_deref()).await) });
                }
                let mut found = Vec::new();
                while let Some(Ok(done)) = running.join_next().await {
                    found.push(done);
                }
                found
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                for (id, picture) in found {
                    this.pictured.insert(id);
                    if let Some(picture) = picture {
                        this.pictures.insert(id, picture);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// `wide` says the filters have their own column; without it they are a menu here.
    fn toolbar(&self, wide: bool, window: &Window, cx: &mut Context<Self>) -> Div {
        let filters = (!wide).then(|| self.filters_menu(cx));
        let theme = cx.theme();
        let focused = self.search.focus_handle(cx).is_focused(window);
        let (this, sort) = (cx.entity().downgrade(), self.sort);
        // Relevance with nothing typed ranks by trend: say what the order really is.
        let shown_sort = match sort {
            Sort::Relevance if self.search.read(cx).value().trim().is_empty() => "Trending",
            sort => sort.label(),
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                // The input's own frame only comes in 32 or 44 px, so ours carries the border at
                // the toolbar's height and the input sits inside it bare (as the Library's).
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_1()
                    .h(CONTROL)
                    .pl_3()
                    .rounded(theme.radius)
                    .bg(theme.background)
                    .border_1()
                    .border_color(if focused { theme.ring } else { theme.input })
                    .child(
                        Icon::new(IconName::Search)
                            .size_4()
                            .text_color(theme.muted_foreground),
                    )
                    .child(Input::new(&self.search).appearance(false).cleanable(true)),
            )
            .children(filters)
            .child(
                Button::new("workshop-sort")
                    .outline()
                    .h(CONTROL)
                    .cursor_pointer()
                    .icon(IconName::ArrowDownWideNarrow)
                    .label(shown_sort)
                    .dropdown_menu(move |mut menu, _, _| {
                        for option in Sort::ALL {
                            let this = this.clone();
                            menu = menu.item(
                                PopupMenuItem::new(option.label())
                                    .checked(option == sort)
                                    .on_click(move |_, _, cx| {
                                        let _ = this.update(cx, |this, cx| {
                                            this.sort = option;
                                            this.run(Fetch::Fresh, cx);
                                        });
                                    }),
                            );
                        }
                        menu
                    }),
            )
    }

    /// The filters as a menu, for a window too narrow for their column.
    fn filters_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let (this, build, tags) = (cx.entity().downgrade(), self.build, self.tags.clone());
        let label = match tags.len() + self.excluded.len() {
            0 => "Filters".to_owned(),
            many => format!("Filters ({many})"),
        };
        Button::new("workshop-filters-menu")
            .outline()
            .h(CONTROL)
            .cursor_pointer()
            .icon(IconName::Layers)
            .label(label)
            .dropdown_menu(move |mut menu, window, _| {
                menu = menu
                    .scrollable(true)
                    .max_h(window.viewport_size().height - px(160.))
                    .label("Game version");
                for option in [None, Some(BUILDS[0]), Some(BUILDS[1])] {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(option.unwrap_or("Any version"))
                            .checked(option == build)
                            .on_click(move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| {
                                    this.build = option;
                                    this.run(Fetch::Fresh, cx);
                                });
                            }),
                    );
                }
                menu = menu.separator().label("Categories");
                for tag in TAGS {
                    let this = this.clone();
                    menu = menu.item(
                        PopupMenuItem::new(shown_tag(tag))
                            .checked(tags.contains(&tag))
                            .on_click(move |_, _, cx| {
                                let _ = this.update(cx, |this, cx| this.toggle(tag, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// A category on or off; one left out comes back to neither.
    fn toggle(&mut self, tag: &'static str, cx: &mut Context<Self>) {
        if let Some(at) = self.excluded.iter().position(|t| *t == tag) {
            self.excluded.remove(at);
            return self.run(Fetch::Fresh, cx);
        }
        match self.tags.iter().position(|t| *t == tag) {
            Some(at) => {
                self.tags.remove(at);
            }
            None => self.tags.push(tag),
        }
        self.run(Fetch::Fresh, cx);
    }

    /// Leaves a category out of the results, or lets it back.
    fn exclude(&mut self, tag: &'static str, cx: &mut Context<Self>) {
        self.tags.retain(|t| *t != tag);
        match self.excluded.iter().position(|t| *t == tag) {
            Some(at) => {
                self.excluded.remove(at);
            }
            None => self.excluded.push(tag),
        }
        self.run(Fetch::Fresh, cx);
    }

    /// Back to what the page opens with: Build 42, no categories, no words.
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.build = Some(BUILDS[0]);
        self.tags.clear();
        self.excluded.clear();
        // Clearing the text searches through the field's own change event.
        let typed = !self.search.read(cx).value().is_empty();
        self.search
            .update(cx, |search, cx| search.set_value("", window, cx));
        if !typed {
            self.run(Fetch::Fresh, cx);
        }
    }

    /// The filters: which build, then the categories (any number, all required).
    fn sidebar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let builds = [None, Some(BUILDS[0]), Some(BUILDS[1])].map(|build| {
            let label = build.unwrap_or("Any version");
            choice(label, label, self.build == build, false, window, cx)
                .0
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.build = build;
                    this.run(Fetch::Fresh, cx);
                }))
        });
        let tags = TAGS.map(|tag| self.category(tag, window, cx));
        let theme = cx.theme();
        // How many categories are on, and a way to drop them all; they are all required, so
        // a few can leave nothing.
        let picked = self.tags.len() + self.excluded.len();
        let clear = (picked > 0).then(|| {
            div()
                .id("workshop-clear-tags")
                .px_1()
                .rounded(theme.radius)
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.primary)
                .cursor_pointer()
                .hover(|style| style.bg(theme.muted))
                .child(format!("Clear {picked}"))
                .on_click(cx.listener(|this, _, _, cx| {
                    // Clearing sits in the title, which folds the group otherwise.
                    cx.stop_propagation();
                    this.tags.clear();
                    this.excluded.clear();
                    this.run(Fetch::Fresh, cx);
                }))
        });
        div()
            .id("workshop-filters")
            .flex()
            .flex_col()
            .flex_none()
            .w(SIDEBAR)
            .gap_3()
            .overflow_y_scroll()
            .child(self.section("Game version", None, builds, cx))
            .child(self.section("Categories", clear, tags, cx))
            .into_any_element()
    }

    /// A category row, like Modrinth's: clicking it requires the category, and the button
    /// that shows on hover leaves it out instead (struck through, in the danger color).
    fn category(
        &self,
        tag: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let out = self.excluded.contains(&tag);
        let (row, lit) = choice(
            tag,
            &shown_tag(tag),
            self.tags.contains(&tag),
            out,
            window,
            cx,
        );
        let theme = cx.theme();
        let ban = div()
            .id(SharedString::from(format!("workshop-exclude-{tag}")))
            .flex()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(theme.radius)
            .cursor_pointer()
            .hover(|style| style.bg(theme.muted))
            .tooltip(move |window, cx| {
                Tooltip::new(if out { "Let it back" } else { "Leave it out" }).build(window, cx)
            })
            .child(Icon::empty().path("icons/ban.svg").size_4())
            .on_click(cx.listener(move |this, _, _, cx| {
                // The row under it would require the category.
                cx.stop_propagation();
                this.exclude(tag, cx);
            }))
            // A row's trailing action fades in with its hover; a left-out one keeps it.
            .opacity(if out { 1. } else { lit });
        row.child(ban)
            .on_click(cx.listener(move |this, _, _, cx| this.toggle(tag, cx)))
    }

    /// A group of filters in its own card, like Modrinth's: the title folds it, and
    /// `action` (Clear) sits at the title's end.
    fn section(
        &self,
        title: &'static str,
        action: Option<Stateful<Div>>,
        rows: impl IntoIterator<Item = Stateful<Div>>,
        cx: &mut Context<Self>,
    ) -> Div {
        let open = !self.folded.contains(title);
        // Unfolding brings the rows in one after another, the first few only: past that
        // the last would come in late.
        let reduce = cx.reduce_motion();
        let rows = rows
            .into_iter()
            .enumerate()
            .map(move |(ix, row)| ui::enter_nth(row, (title, ix), ix.min(6), reduce));
        let theme = cx.theme();
        div()
            .flex()
            .flex_col()
            .gap_0p5()
            .p_2()
            .rounded(theme.radius_lg)
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .child(
                div()
                    .id(title)
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .h(px(32.))
                    .pl_2()
                    .pr_1()
                    .font_weight(FontWeight::SEMIBOLD)
                    .cursor_pointer()
                    .child(title)
                    .child(
                        div().flex().items_center().gap_2().children(action).child(
                            Icon::new(if open {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .size_4()
                            .text_color(theme.muted_foreground),
                        ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if !this.folded.remove(title) {
                            this.folded.insert(title);
                        }
                        cx.notify();
                    })),
            )
            .when(open, |this| this.children(rows))
    }

    /// One result: picture, title and author, the start of its description and its tags, and
    /// on the right how many use it, when it last changed and the button to add it.
    fn card(
        &self,
        ix: usize,
        item: &Item,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let hovered = window.use_keyed_state(
            SharedString::from(format!("workshop-card-hover-{}", item.id)),
            cx,
            |_, _| false,
        );
        let lit: f32 = spring(
            SharedString::from(format!("workshop-card-lit-{}", item.id)),
            if *hovered.read(cx) { 1. } else { 0. },
            ui::SPRING_FAST,
            window,
            cx,
        );
        let reduce = cx.reduce_motion();
        let theme = cx.theme();
        let picture = match self.pictures.get(&item.id) {
            Some(path) => div().flex_none().size(PICTURE).child(
                img(path.clone())
                    .size_full()
                    .rounded(px(10.))
                    .object_fit(ObjectFit::Cover)
                    .with_loading(|| {
                        Skeleton::new()
                            .size_full()
                            .rounded(px(10.))
                            .into_any_element()
                    }),
            ),
            None if !self.pictured.contains(&item.id) => div()
                .flex_none()
                .size(PICTURE)
                .child(Skeleton::new().size_full().rounded(px(10.))),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(PICTURE)
                .rounded(px(10.))
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::Puzzle).size_6()),
        };
        let summary = bbcode::plain(&item.summary);
        // The build and the categories filtered by first: they say why this mod is here.
        let mut tags: Vec<&String> = item.tags.iter().collect();
        tags.sort_by_key(|tag| {
            !(BUILDS.contains(&tag.as_str()) || self.tags.contains(&tag.as_str()))
        });
        let more = tags.len().saturating_sub(SHOWN_TAGS);
        let tags = tags.into_iter().take(SHOWN_TAGS).map(|tag| {
            div()
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .bg(theme.muted)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(shown_tag(tag))
        });
        let (shell, picked) = (self.shell.clone(), item.clone());
        let (picture_path, author) = (
            self.pictures.get(&item.id).cloned(),
            self.authors.get(&item.author).cloned(),
        );
        let (id, seed) = (item.id, item.clone());
        let bg = theme.background;
        let card = div()
            .id(("workshop-result", ix))
            .flex()
            .gap_4()
            .p_4()
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(ui::mix(bg, ui::hovered(bg, cx), lit))
            .cursor_pointer()
            .on_hover(move |on, window, cx| {
                hovered.update(cx, |h, _| *h = *on);
                window.refresh();
            })
            // The whole card opens the mod's page; Install stops the click short of it.
            .on_click(cx.listener(move |this, _, _, cx| this.visit(id, Some(seed.clone()), cx)))
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        // Title and author on one line, like Modrinth's results; the title
                        // opens the mod's Workshop page, where the whole of it is.
                        div()
                            .flex()
                            .items_baseline()
                            .gap_2()
                            .min_w_0()
                            .child(
                                div()
                                    .id(("workshop-title", ix))
                                    .min_w_0()
                                    .truncate()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(item.title.clone()),
                            )
                            .children(author.clone().map(|name| {
                                div()
                                    .flex_none()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("by {name}"))
                            })),
                    )
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .line_clamp(2)
                                .text_ellipsis()
                                .child(summary),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_1p5()
                            .pt_1()
                            .children(tags)
                            .when(more > 0, |this| {
                                this.child(
                                    div()
                                        .py_0p5()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("{more} more")),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_none()
                    .items_end()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_end()
                            .gap_1()
                            .child(
                                div()
                                    .id(("workshop-subscribers", ix))
                                    .tooltip(|window, cx| {
                                        Tooltip::new("Subscribers").build(window, cx)
                                    })
                                    .child(Meta::new(
                                        IconName::CircleUser,
                                        crate::discover::compact(item.subscriptions),
                                    )),
                            )
                            .children(item.updated.map(|at| {
                                let at = u64::from(at);
                                div()
                                    .id(("workshop-updated", ix))
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(format!("Updated {}", branches::date(at)))
                                            .build(window, cx)
                                    })
                                    .child(Meta::new(
                                        IconName::Clock,
                                        crate::instance::ago(
                                            crate::instance::store::now().saturating_sub(at),
                                        ),
                                    ))
                            })),
                    )
                    .child(
                        FancyButton::new(("workshop-add", ix), "Install")
                            .small()
                            .icon(IconName::Download)
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                crate::discover::pick(
                                    picked.clone(),
                                    picture_path.clone(),
                                    author.clone(),
                                    shell.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    ),
            );
        // New results come in one after another (a fresh search, or the next page); ones
        // already shown stay put, since the key is the item.
        ui::enter_nth(
            card,
            ("workshop-in", item.id as usize),
            (ix % PAGE as usize).min(6),
            reduce,
        )
        .into_any_element()
    }

    /// Below the results: the next page, or why there is nothing.
    fn footer(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        match &self.failed {
            // The next page failed: what is shown stays, and trying again asks for that page.
            Some((why, Fetch::More)) => {
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap_2()
                        .py_2()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("Couldn't load more mods: {why}"))
                        .child(
                            ui::secondary("workshop-more-retry", "Try again")
                                .on_click(cx.listener(|this, _, _, cx| this.run(Fetch::More, cx))),
                        )
                        .into_any_element(),
                );
            }
            Some((why, _)) => {
                return Some(
                    Empty::new(
                        IconName::TriangleAlert,
                        "Steam didn't answer that one",
                        why.clone(),
                    )
                    .action(
                        FancyButton::new("workshop-retry", "Try again")
                            .small()
                            .on_click(cx.listener(|this, _, _, cx| this.run(Fetch::Fresh, cx))),
                    )
                    .into_any_element(),
                );
            }
            None => {}
        }
        if self.results.is_empty() {
            // The first page on its way: the shape of what is coming.
            if self.loading {
                return Some(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .children((0..3).map(|_| {
                            Skeleton::new()
                                .h(px(122.))
                                .w_full()
                                .rounded(cx.theme().radius_lg)
                        }))
                        .into_any_element(),
                );
            }
            return Some(
                Empty::new(
                    IconName::Search,
                    "No mods match",
                    "Try other words, or fewer categories.",
                )
                .action(
                    ui::secondary("workshop-reset", "Reset filters")
                        .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                )
                .into_any_element(),
            );
        }
        self.next.as_ref()?;
        Some(
            div()
                .flex()
                .justify_center()
                .py_2()
                .child(
                    ui::secondary("workshop-more", "Load more")
                        .loading(self.loading)
                        .disabled(self.loading)
                        .on_click(cx.listener(|this, _, _, cx| this.run(Fetch::More, cx))),
                )
                .into_any_element(),
        )
    }

    fn sign_in(&self) -> AnyElement {
        let shell = self.shell.clone();
        Empty::new(
            IconName::Compass,
            "Sign in to browse the Workshop",
            "lata searches it with your Steam account, so you see who made each mod.",
        )
        .action(
            FancyButton::new("workshop-sign-in", "Sign in with Steam")
                .small()
                .on_click(move |_, window, cx| Shell::open_account(shell.clone(), window, cx)),
        )
        .into_any_element()
    }
}

/// Searches with the account's session (logging on first when there is none, or it belongs
/// to another account) and reads the names of the authors not in `known`. A session Steam
/// closed (sleep, a network change) gets one more try on a fresh one.
/// A job for the logged-on session: it borrows the client for as long as it runs.
type Job<T> = Box<
    dyn for<'c> FnMut(
            &'c mut steam::Client,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = steam::Result<T>> + Send + 'c>,
        > + Send,
>;

/// Runs `job` on `account`'s logged-on session, logging on first when there is none or it
/// belongs to someone else; a session that fails is dropped and the job tried once more on a
/// new one (Steam drops idle connections).
async fn logged<T: Send + 'static>(
    account: Account,
    session: Session,
    mut job: Job<T>,
) -> Result<T, String> {
    crate::runtime::spawn(async move {
        let http = steam::http();
        let mut held = session.lock().await;
        let mut last = String::new();
        for _ in 0..2 {
            if held.as_ref().is_none_or(|(who, _)| *who != account.steamid) {
                let client = account.client(&http).await.map_err(|e| e.to_string())?;
                *held = Some((account.steamid, client));
            }
            let Some((_, client)) = held.as_mut() else {
                break;
            };
            match job(client).await {
                Ok(done) => return Ok(done),
                Err(err) => {
                    last = err.to_string();
                    *held = None;
                }
            }
        }
        Err(last)
    })
    .await
    .map_err(|err| err.to_string())?
}

/// A page of results, with the names of authors not in `known`.
async fn search(
    account: Account,
    session: Session,
    query: Query,
    known: Vec<u64>,
) -> Result<(Page, HashMap<u64, String>), String> {
    logged(
        account,
        session,
        Box::new(move |client| {
            let (query, known) = (query.clone(), known.clone());
            Box::pin(async move {
                let page = steam::workshop::search(client, &query).await?;
                let mut ids: Vec<u64> = page
                    .items
                    .iter()
                    .map(|item| item.author)
                    .filter(|id| !known.contains(id))
                    .collect();
                ids.sort_unstable();
                ids.dedup();
                // Names are a nicety: a page without them still shows.
                let names = match ids.is_empty() {
                    true => HashMap::new(),
                    false => steam::workshop::authors(client, &ids)
                        .await
                        .unwrap_or_default(),
                };
                Ok((page, names))
            })
        }),
    )
    .await
}

/// A filter row: selected ones get the accent's tint and a check, and `out` marks a category
/// left out (struck through, in the danger color). The hover eases in on a spring; it comes back
/// with the row so trailing actions can fade with it.
fn choice(
    id: &'static str,
    label: &str,
    on: bool,
    out: bool,
    window: &mut Window,
    cx: &mut App,
) -> (Stateful<Div>, f32) {
    let hovered = window.use_keyed_state(
        SharedString::from(format!("workshop-choice-hover-{id}")),
        cx,
        |_, _| false,
    );
    let lit: f32 = spring(
        SharedString::from(format!("workshop-choice-lit-{id}")),
        if *hovered.read(cx) { 1. } else { 0. },
        ui::SPRING_FAST,
        window,
        cx,
    );
    let reduce = cx.reduce_motion();
    let theme = cx.theme();
    let (rest, lifted) = if out {
        (theme.danger.opacity(0.), theme.danger.opacity(0.1))
    } else if on {
        (theme.primary.opacity(0.2), theme.primary.opacity(0.3))
    } else {
        (theme.muted.opacity(0.), theme.muted)
    };
    let check = Icon::new(IconName::Check).size_4().with_animation(
        SharedString::from(format!("workshop-choice-check-{id}")),
        Animation::new(ui::ENTER).with_easing(ui::ease_out_quint),
        // The check scales from .6 as it appears, like a checkbox's.
        move |icon, t| {
            let t = if reduce { 1. } else { t };
            let scale = 0.6 + 0.4 * t;
            icon.transform(Transformation::scale(size(scale, scale)))
                .opacity(t)
        },
    );
    let row = div()
        .id(id)
        .flex()
        .items_center()
        .gap_2()
        .h(px(32.))
        .px_3()
        .rounded(theme.radius)
        .text_sm()
        .cursor_pointer()
        .bg(ui::mix(rest, lifted, lit))
        .map(|this| {
            if out {
                this.text_color(theme.danger).line_through()
            } else if on {
                this.font_weight(FontWeight::MEDIUM)
            } else {
                this.text_color(theme.muted_foreground)
            }
        })
        .on_hover(move |on, window, cx| {
            hovered.update(cx, |h, _| *h = *on);
            window.refresh();
        })
        .child(div().flex_1().min_w_0().truncate().child(label.to_owned()))
        .when(on, |this| this.child(check));
    (row, lit)
}

/// A tag as shown: the Workshop's `building` gets the capital its siblings have.
fn shown_tag(tag: &str) -> String {
    let mut chars = tag.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

impl Render for Workshop {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::smooth_scroll("workshop-scroll", &self.scroll, window, cx);
        let page = div().id("workshop").flex().flex_col().size_full().p_6();
        let Some(account) = self.account(cx) else {
            self.who = None;
            return page.child(self.sign_in());
        };
        // The first visit, or another account signed in: what it sees starts over.
        if self.who != Some(account.steamid) {
            // Another account: what it sees starts over. The first visit keeps a page it was
            // opened on (a mod's "View in Workshop").
            if self.who.is_some() {
                self.open = None;
            }
            self.who = Some(account.steamid);
            self.run(Fetch::Fresh, cx);
        }
        // A mod's page covers the results, which stay as they were (scroll included) for
        // coming back.
        if let Some((page_view, _)) = &self.open {
            return page.child(page_view.clone());
        }
        let cards: Vec<AnyElement> = self
            .results
            .iter()
            .enumerate()
            .map(|(ix, item)| self.card(ix, item, window, cx))
            .collect();
        // The page's own width: the window less the instance list, the panel on the right when
        // it shows, and the padding. Below this the filters' column leaves the results too
        // little room, and the filters become a menu.
        let window_width = window.bounds().size.width;
        let panel = match window_width >= crate::discover::MIN_WINDOW {
            true => crate::discover::WIDTH,
            false => px(0.),
        };
        let wide = window_width - crate::shell::sidebar::WIDTH - panel - px(48.) >= WIDE;
        let sidebar = wide.then(|| self.sidebar(window, cx));
        let count = match (self.results.is_empty(), self.total) {
            (true, _) => None,
            (false, 1) => Some("1 mod".to_owned()),
            (false, many) => Some(format!("{} mods", crate::discover::compact(many))),
        };
        let footer = self.footer(cx);
        page.gap_4()
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_baseline()
                    .gap_3()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::BOLD)
                            .child("Workshop"),
                    )
                    .children(count.map(|count| {
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(count)
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .gap_6()
                    .children(sidebar)
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .gap_4()
                            .child(self.toolbar(wide, window, cx))
                            .child(
                                div()
                                    .id("workshop-results")
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_3()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children(cards)
                                    .children(footer),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::shown_tag;

    #[test]
    fn tags_show_with_a_capital() {
        assert_eq!(shown_tag("building"), "Building");
        assert_eq!(shown_tag("QOL"), "QOL");
        assert_eq!(shown_tag(""), "");
    }
}
