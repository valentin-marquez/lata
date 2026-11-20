//! A mod's page, like Modrinth's project page: its header (picture, title, author, numbers and
//! Add), tabs for its description, gallery and changelog, and a side column with what it runs
//! on, its categories, what it requires, its details and links.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::text::TextView;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use steam::workshop::{About, Change, Item};

use super::{BUILDS, Session, bbcode, shown_tag};
use crate::account::Account;
use crate::instance::{branches, mods};
use crate::shell::Shell;
use crate::ui::{self, Empty, FancyButton, Meta};

/// Hosts a description's pictures load from: Steam's and the image hosts mod authors use.
/// Opening a page doesn't make lata fetch from wherever an author points it.
const IMAGE_HOSTS: [&str; 9] = [
    "i.imgur.com",
    "imgur.com",
    "i.ibb.co",
    "i.postimg.cc",
    "cdn.discordapp.com",
    "media.discordapp.net",
    "raw.githubusercontent.com",
    "user-images.githubusercontent.com",
    "storage.ko-fi.com",
];

/// Whether a description's picture at `url` is fetched.
fn shown(url: &str) -> bool {
    let host = url
        .strip_prefix("https://")
        .and_then(|rest| rest.split(['/', '?', '#']).next())
        .unwrap_or("");
    steam::workshop::is_steam_image(url) || IMAGE_HOSTS.contains(&host)
}

/// The side column's width, Modrinth's.
const SIDE: Pixels = px(272.);
/// The header picture's side.
const PICTURE: Pixels = px(96.);

/// What the page asks of the Workshop page that holds it.
pub enum Event {
    /// Back to the results.
    Back,
    /// Another mod's page: one it requires, or one its description links to.
    Show(u64),
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Description,
    Gallery,
    Changelog,
}

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Description => "Description",
            Tab::Gallery => "Gallery",
            Tab::Changelog => "Changelog",
        }
    }
}

/// What a page shows past what the results already knew.
struct Loaded {
    about: About,
    /// The description as HTML, and each change's note: made once, not on every frame of a
    /// scroll.
    html: SharedString,
    notes: Vec<SharedString>,
    changes: Vec<Change>,
    requires: Vec<Item>,
    author: Option<String>,
}

pub struct Detail {
    shell: WeakEntity<Shell>,
    session: Session,
    id: u64,
    /// What the results knew about it, shown while the rest loads.
    seed: Option<Item>,
    author: Option<String>,
    tab: Tab,
    loaded: Option<Result<Loaded, String>>,
    /// Pictures downloaded, by their address: the header's, the description's, the gallery's
    /// and the required mods'.
    images: Arc<HashMap<String, PathBuf>>,
    /// Whether the description's pictures are still downloading: it waits for them, so its
    /// text doesn't jump as they land.
    fetching: bool,
    scroll: ScrollHandle,
}

impl EventEmitter<Event> for Detail {}

impl Detail {
    pub fn new(
        id: u64,
        seed: Option<Item>,
        picture: Option<PathBuf>,
        author: Option<String>,
        shell: WeakEntity<Shell>,
        session: Session,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut images = HashMap::new();
        if let (Some(url), Some(path)) = (seed.as_ref().and_then(|s| s.preview.clone()), picture) {
            images.insert(url, path);
        }
        let mut this = Self {
            shell,
            session,
            id,
            seed,
            author,
            tab: Tab::Description,
            loaded: None,
            images: Arc::new(images),
            fetching: false,
            scroll: ScrollHandle::new(),
        };
        this.load(cx);
        this
    }

    fn account(&self, cx: &App) -> Option<Account> {
        self.shell.upgrade()?.read(cx).account().cloned()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account(cx) else {
            return;
        };
        self.loaded = None;
        self.loader(true, cx);
        let (id, session) = (self.id, self.session.clone());
        let named = self.author.is_some();
        cx.spawn(async move |this, cx| {
            let found = super::logged(
                account,
                session,
                Box::new(move |client| {
                    Box::pin(async move {
                        let about = steam::workshop::about(client, id).await?;
                        // The rest is a nicety: the page shows without it.
                        let changes = steam::workshop::history(client, id)
                            .await
                            .unwrap_or_default();
                        let requires = match about.item.requires.is_empty() {
                            true => Vec::new(),
                            false => steam::workshop::details(client, &about.item.requires)
                                .await
                                .unwrap_or_default(),
                        };
                        let author = match named {
                            true => None,
                            false => steam::workshop::authors(client, &[about.item.author])
                                .await
                                .ok()
                                .and_then(|names| names.into_values().next()),
                        };
                        let html = bbcode::html(&about.description).into();
                        let notes = changes
                            .iter()
                            .map(|change| bbcode::html(change.note.trim()).into())
                            .collect();
                        Ok(Loaded {
                            about,
                            html,
                            notes,
                            changes,
                            requires,
                            author,
                        })
                    })
                }),
            )
            .await;
            let _ = this.update(cx, |this, cx| {
                match &found {
                    Ok(loaded) => {
                        if loaded.author.is_some() {
                            this.author = loaded.author.clone();
                        }
                        this.fetch_images(loaded, cx);
                    }
                    Err(_) => this.loader(false, cx),
                }
                this.loaded = Some(found);
                cx.notify();
            });
        })
        .detach();
    }

    /// The accent bar under the title bar, while the page loads.
    fn loader(&self, on: bool, cx: &mut Context<Self>) {
        let _ = self.shell.update(cx, |shell, cx| shell.set_loading(on, cx));
    }

    /// Downloads the pictures the page shows, into the same cache as the results'.
    fn fetch_images(&mut self, loaded: &Loaded, cx: &mut Context<Self>) {
        let mut wanted: Vec<String> = loaded.about.item.preview.iter().cloned().collect();
        wanted.extend(
            bbcode::images(&loaded.about.description)
                .into_iter()
                .filter(|url| shown(url)),
        );
        wanted.extend(loaded.about.gallery.iter().cloned());
        wanted.extend(
            loaded
                .requires
                .iter()
                .filter_map(|item| item.preview.clone()),
        );
        wanted.retain(|url| !self.images.contains_key(url));
        wanted.dedup();
        self.fetching = true;
        cx.spawn(async move |this, cx| {
            let found = crate::runtime::spawn(async move {
                let http = steam::http();
                let mut running = tokio::task::JoinSet::new();
                for url in wanted {
                    let http = http.clone();
                    // ponytail: the cache names a picture by its file name, which two hosts
                    // could share; name it by the whole address if that ever shows.
                    running.spawn(async move {
                        let path = mods::picture(&http, Some(&url)).await;
                        (url, path)
                    });
                }
                let mut found = Vec::new();
                while let Some(Ok((url, Some(path)))) = running.join_next().await {
                    found.push((url, path));
                }
                found
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                Arc::make_mut(&mut this.images).extend(found);
                this.fetching = false;
                this.loader(false, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn item(&self) -> Option<&Item> {
        match &self.loaded {
            Some(Ok(loaded)) => Some(&loaded.about.item),
            _ => self.seed.as_ref(),
        }
    }

    fn page_url(&self) -> String {
        format!(
            "https://steamcommunity.com/sharedfiles/filedetails/?id={}",
            self.id
        )
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(item) = self.item().cloned() else {
            return div()
                .flex()
                .gap_4()
                .child(Skeleton::new().size(PICTURE).rounded(theme.radius_lg))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(Skeleton::new().w(px(280.)).h(px(28.)))
                        .child(Skeleton::new().w(px(180.)).h(px(18.))),
                )
                .into_any_element();
        };
        let picture = item
            .preview
            .as_ref()
            .and_then(|url| self.images.get(url))
            .cloned();
        let picture = match picture {
            Some(path) => div().flex_none().size(PICTURE).child(
                img(path)
                    .size_full()
                    .rounded(theme.radius_lg)
                    .object_fit(ObjectFit::Cover),
            ),
            None => div()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(PICTURE)
                .rounded(theme.radius_lg)
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .child(Icon::new(IconName::Puzzle).size_8()),
        };
        let summary = match &self.loaded {
            Some(Ok(loaded)) if item.summary.is_empty() => bbcode::plain(&loaded.about.description),
            _ => bbcode::plain(&item.summary),
        };
        let favorited = match &self.loaded {
            Some(Ok(loaded)) => Some(loaded.about.favorited),
            _ => None,
        };
        let (shell, picked, author) = (self.shell.clone(), item.clone(), self.author.clone());
        let picture_path = item
            .preview
            .as_ref()
            .and_then(|url| self.images.get(url))
            .cloned();
        let url = self.page_url();
        div()
            .flex()
            .items_start()
            .gap_4()
            .child(picture)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap_x_2()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_weight(FontWeight::BOLD)
                                    .child(item.title.clone()),
                            )
                            .children(self.author.clone().map(|name| {
                                div()
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
                            .gap_4()
                            .pt_1()
                            .child(Meta::new(
                                IconName::CircleUser,
                                format!(
                                    "{} subscribers",
                                    crate::discover::compact(item.subscriptions)
                                ),
                            ))
                            .children(favorited.map(|n| {
                                Meta::new(
                                    Icon::empty().path("icons/heart.svg"),
                                    format!("{} favorites", crate::discover::compact(n)),
                                )
                            }))
                            .children(item.updated.map(|at| {
                                Meta::new(
                                    IconName::Clock,
                                    format!(
                                        "Updated {}",
                                        crate::instance::ago(
                                            crate::instance::store::now()
                                                .saturating_sub(u64::from(at)),
                                        )
                                    ),
                                )
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_2()
                    .child(
                        FancyButton::new("workshop-detail-add", "Install")
                            .icon(IconName::Download)
                            .on_click(move |_, window, cx| {
                                crate::discover::pick(
                                    picked.clone(),
                                    picture_path.clone(),
                                    author.clone(),
                                    shell.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .child(
                        Button::new("workshop-detail-more")
                            .ghost()
                            .cursor_pointer()
                            .icon(IconName::EllipsisVertical)
                            .size(px(40.))
                            .rounded(px(10.))
                            .dropdown_menu(move |menu, _, _| {
                                let (open, copy) = (url.clone(), url.clone());
                                menu.item(
                                    PopupMenuItem::new("Open its Workshop page")
                                        .icon(IconName::ExternalLink)
                                        .on_click(move |_, _, cx| cx.open_url(&open)),
                                )
                                .item(
                                    PopupMenuItem::new("Copy link")
                                        .icon(IconName::Copy)
                                        .on_click(move |_, _, cx| {
                                            cx.write_to_clipboard(ClipboardItem::new_string(
                                                copy.clone(),
                                            ))
                                        }),
                                )
                            }),
                    ),
            )
            .into_any_element()
    }

    fn tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let loaded = match &self.loaded {
            // The tray's shape while it's unknown which tabs there are.
            None => {
                return Some(
                    Skeleton::new()
                        .w(px(300.))
                        .h(px(46.))
                        .rounded(cx.theme().radius)
                        .into_any_element(),
                );
            }
            Some(Err(_)) => return None,
            Some(Ok(loaded)) => loaded,
        };
        let mut tabs = vec![Tab::Description];
        if !loaded.about.gallery.is_empty() {
            tabs.push(Tab::Gallery);
        }
        if !loaded.changes.is_empty() {
            tabs.push(Tab::Changelog);
        }
        if tabs.len() == 1 {
            return None;
        }
        let selected = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        use gpui_kit::component::Sizable as _;
        use gpui_kit::component::tab::{Tab as TabItem, TabBar};
        let bar = TabBar::new("workshop-detail-tabs")
            .pill()
            .large()
            .selected_index(selected)
            .children(
                tabs.iter()
                    .map(|tab| TabItem::new().label(tab.label()).cursor_pointer()),
            )
            .on_click(cx.listener(move |this, ix: &usize, _, cx| {
                this.tab = tabs[*ix];
                cx.notify();
            }));
        Some(div().flex().child(ui::tab_tray(bar, cx)).into_any_element())
    }

    /// What the tab shows, in a card.
    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let card = div()
            .flex()
            .flex_col()
            .p_6()
            .rounded(theme.radius_lg)
            .bg(theme.background)
            .border_1()
            .border_color(theme.border);
        let waiting = self.loaded.is_none() || (self.fetching && self.tab == Tab::Description);
        if waiting {
            // A description's shape: a heading, a paragraph, a picture, another paragraph.
            let line = |width: f32| Skeleton::new().w(relative(width)).h(px(14.));
            return card
                .gap_3()
                .child(Skeleton::new().w(px(240.)).h(px(26.)))
                .child(line(1.))
                .child(line(0.94))
                .child(line(0.7))
                .child(
                    Skeleton::new()
                        .w_full()
                        .h(px(200.))
                        .rounded(theme.radius_lg),
                )
                .child(line(0.97))
                .child(line(0.88))
                .child(line(0.5))
                .into_any_element();
        }
        let loaded = match &self.loaded {
            None => return card.into_any_element(),
            Some(Err(why)) => {
                return card
                    .child(
                        Empty::new(
                            IconName::TriangleAlert,
                            "Couldn't open this mod",
                            why.clone(),
                        )
                        .action(
                            ui::secondary("workshop-detail-retry", "Try again")
                                .on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                        ),
                    )
                    .into_any_element();
            }
            Some(Ok(loaded)) => loaded,
        };
        match self.tab {
            Tab::Description if loaded.about.description.trim().is_empty() => card
                .text_color(theme.muted_foreground)
                .child("Its author didn't write a description.")
                .into_any_element(),
            Tab::Description => {
                let images = self.images.clone();
                let this = cx.entity().downgrade();
                card.child(
                    TextView::html(
                        ("workshop-description", self.id as usize),
                        loaded.html.clone(),
                    )
                    .selectable(true)
                    // Pictures come from the cache once downloaded; until then they're blank.
                    .image_source(move |uri| match images.get(uri.as_ref()) {
                        Some(path) => ImageSource::from(path.as_path()),
                        None => ImageSource::from(std::path::Path::new("")),
                    })
                    // Links to other Workshop mods open their page here.
                    .on_link_click(move |url, _, _, cx| {
                        // The description is the author's: only web links leave the app.
                        if !url.starts_with("https://") && !url.starts_with("http://") {
                            return;
                        }
                        // parse_id checks the host is Steam's own.
                        match steam::workshop::parse_id(url) {
                            Some(id) => {
                                let _ = this.update(cx, |_, cx| cx.emit(Event::Show(id)));
                            }
                            None => cx.open_url(url),
                        }
                    }),
                )
                .into_any_element()
            }
            Tab::Gallery => card
                .flex_row()
                .flex_wrap()
                .gap_3()
                .children(loaded.about.gallery.iter().enumerate().map(|(ix, url)| {
                    let (open, path) = (url.clone(), self.images.get(url).cloned());
                    div()
                        .id(("workshop-gallery", ix))
                        .w(relative(0.48))
                        .h(px(220.))
                        .rounded(theme.radius)
                        .overflow_hidden()
                        .bg(theme.muted)
                        .cursor_pointer()
                        .tooltip(|window, cx| Tooltip::new("Open full size").build(window, cx))
                        .on_click(move |_, _, cx| cx.open_url(&open))
                        .child(match path {
                            Some(path) => img(path)
                                .size_full()
                                .object_fit(ObjectFit::Contain)
                                .into_any_element(),
                            None => Skeleton::new().size_full().into_any_element(),
                        })
                }))
                .into_any_element(),
            Tab::Changelog => card
                .gap_5()
                .children(loaded.changes.iter().enumerate().map(|(ix, change)| {
                    let note = change.note.trim();
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(branches::date(change.date)),
                        )
                        .child(if note.is_empty() {
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child("No notes for this one.")
                                .into_any_element()
                        } else {
                            TextView::html(
                                SharedString::from(format!("workshop-change-{}-{ix}", self.id)),
                                loaded.notes[ix].clone(),
                            )
                            .selectable(true)
                            .text_sm()
                            .into_any_element()
                        })
                }))
                .into_any_element(),
        }
    }

    /// The side column: builds, categories, requirements, details and links, each a card.
    fn side(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.item()?.clone();
        let reduce = cx.reduce_motion();
        let theme = cx.theme();
        let section = |title: &'static str| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .p_4()
                .rounded(theme.radius_lg)
                .bg(theme.background)
                .border_1()
                .border_color(theme.border)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
        };
        let chip = |text: String| {
            div()
                .px_2()
                .py_0p5()
                .rounded(theme.radius)
                .bg(theme.muted)
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let builds: Vec<String> = item
            .tags
            .iter()
            .filter(|tag| BUILDS.contains(&tag.as_str()))
            .cloned()
            .collect();
        let categories: Vec<String> = item
            .tags
            .iter()
            .filter(|tag| !BUILDS.contains(&tag.as_str()))
            .map(|tag| shown_tag(tag))
            .collect();
        let chips = |tags: Vec<String>| {
            div()
                .flex()
                .flex_wrap()
                .gap_1p5()
                .children(tags.into_iter().map(chip))
        };
        let compatibility = section("Compatibility").child(if builds.is_empty() {
            div()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("Its author didn't say which build it's for.")
                .into_any_element()
        } else {
            chips(builds).into_any_element()
        });
        let categories =
            (!categories.is_empty()).then(|| section("Categories").child(chips(categories)));
        let requires = match &self.loaded {
            None if !item.requires.is_empty() => Some(
                section("Requires")
                    .children(item.requires.iter().map(|_| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .p_1()
                            .child(Skeleton::new().size(px(32.)).rounded(px(6.)))
                            .child(Skeleton::new().w(px(150.)).h(px(14.)))
                    }))
                    .into_any_element(),
            ),
            // It only shows once the page is in: it fades in rather than popping.
            Some(Ok(loaded)) if !loaded.requires.is_empty() => Some(
                ui::enter_nth(
                    section("Requires").children(loaded.requires.iter().map(|req| {
                        let id = req.id;
                        let picture = req.preview.as_ref().and_then(|url| self.images.get(url));
                        div()
                            .id(("workshop-requires", id as usize))
                            .flex()
                            .items_center()
                            .gap_2()
                            .p_1()
                            .rounded(theme.radius)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.muted))
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(Event::Show(id))))
                            .child(match picture {
                                Some(path) => img(path.clone())
                                    .flex_none()
                                    .size(px(32.))
                                    .rounded(px(6.))
                                    .object_fit(ObjectFit::Cover)
                                    .into_any_element(),
                                None => div()
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .justify_center()
                                    .size(px(32.))
                                    .rounded(px(6.))
                                    .bg(theme.muted)
                                    .child(Icon::new(IconName::Puzzle).size_4())
                                    .into_any_element(),
                            })
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .min_w_0()
                                    .child(req.title.clone()),
                            )
                    })),
                    ("workshop-requires-in", self.id as usize),
                    0,
                    reduce,
                )
                .into_any_element(),
            ),
            _ => None,
        };
        let created = match &self.loaded {
            Some(Ok(loaded)) => loaded.about.created,
            _ => None,
        };
        let details = section("Details")
            // The publish date comes with the page; its line holds the place meanwhile.
            .when(self.loaded.is_none(), |this| {
                this.child(Skeleton::new().w(px(170.)).h(px(18.)))
            })
            .children(created.map(|at| {
                Meta::new(
                    IconName::Calendar,
                    format!("Published {}", branches::date(u64::from(at))),
                )
            }))
            .children(item.updated.map(|at| {
                Meta::new(
                    IconName::Clock,
                    format!("Updated {}", branches::date(u64::from(at))),
                )
            }))
            .when(item.size > 0, |this| {
                this.child(Meta::new(
                    IconName::HardDrive,
                    crate::instance::install::size(item.size),
                ))
            });
        let link = |id: &'static str, label: String, url: String| {
            div()
                .id(id)
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|style| style.underline())
                .on_click(move |_, _, cx| cx.open_url(&url))
                .child(label)
                .child(
                    Icon::new(IconName::ExternalLink)
                        .size_3p5()
                        .text_color(theme.muted_foreground),
                )
        };
        let links = section("Links")
            .child(link(
                "workshop-detail-page",
                "Workshop page".to_owned(),
                self.page_url(),
            ))
            .child(link(
                "workshop-detail-author",
                match &self.author {
                    Some(name) => format!("More by {name}"),
                    None => "More by its author".to_owned(),
                },
                format!(
                    "https://steamcommunity.com/profiles/{}/myworkshopfiles/?appid={}",
                    item.author,
                    crate::instance::APP
                ),
            ));
        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .child(compatibility)
                .children(categories)
                .children(requires)
                .child(details)
                .child(links)
                .into_any_element(),
        )
    }
}

impl Render for Detail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        ui::smooth_scroll("workshop-detail-scroll", &self.scroll, window, cx);
        // Below this the side column leaves the description too little room, and goes under it.
        let window_width = window.bounds().size.width;
        let panel = match window_width >= crate::discover::MIN_WINDOW {
            true => crate::discover::WIDTH,
            false => px(0.),
        };
        let wide = window_width - crate::shell::sidebar::WIDTH - panel - px(48.) >= px(820.);
        let header = self.header(cx);
        let tabs = self.tabs(cx);
        let ready = self.loaded.is_some() && !(self.fetching && self.tab == Tab::Description);
        let body = match ready {
            // What loaded fades in over where its skeleton was, and again on another tab.
            true => ui::enter_nth(
                div().child(self.body(cx)),
                SharedString::from(format!(
                    "workshop-detail-in-{}-{}",
                    self.id,
                    self.tab.label()
                )),
                0,
                cx.reduce_motion(),
            )
            .into_any_element(),
            false => self.body(cx),
        };
        let side = self.side(cx);
        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .gap_4()
            .children(tabs)
            .child(body);
        div()
            .id("workshop-detail")
            .flex()
            .flex_col()
            .size_full()
            .gap_5()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div().flex().child(
                    Button::new("workshop-detail-back")
                        .ghost()
                        .cursor_pointer()
                        .icon(IconName::ChevronLeft)
                        .label("Workshop")
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Back))),
                ),
            )
            .child(header)
            .map(|this| {
                if wide {
                    this.child(
                        div()
                            .flex()
                            .items_start()
                            .gap_5()
                            .child(main)
                            .children(side.map(|side| div().flex_none().w(SIDE).child(side))),
                    )
                } else {
                    this.child(main).children(side)
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::shown;

    #[test]
    fn description_pictures_load_only_from_known_hosts() {
        assert!(shown("https://i.imgur.com/8CRHL5v.png"));
        assert!(shown(
            "https://steamuserimages-a.akamaihd.net/ugc/2047488292136394117/C7/"
        ));
        assert!(!shown("https://192.168.1.1/a.png"));
        assert!(!shown("https://i.imgur.com.evil.example/a.png"));
        assert!(!shown("http://i.imgur.com/a.png"));
    }
}
