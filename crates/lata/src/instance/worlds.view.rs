//! The Worlds tab: the coop servers the instance hosts, then its saves and the servers it has
//! joined, in the Mods table's style, the last played first.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, Disableable as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::host::{Hosted, Hosting, State};
use super::servers::{Count, Server};
use super::worlds::World;
use crate::shell::Shell;
use crate::ui::{Empty, Meta};

/// Side of a world's screenshot in its row; the game saves them square.
const THUMB: Pixels = px(48.);

/// One row of the tab.
enum Row<'a> {
    Hosted(&'a Hosted),
    World(&'a World),
    Server(&'a Server),
}

impl Row<'_> {
    fn saved(&self) -> u64 {
        match self {
            // What can be started stays on top.
            Row::Hosted(_) => u64::MAX,
            Row::World(world) => world.saved,
            Row::Server(server) => server.world.saved,
        }
    }
}

/// What the Join and server buttons need: the instance, why it can't play right now, and
/// the server lata hosts or last hosted, from any instance.
pub struct Join<'a> {
    pub shell: WeakEntity<Shell>,
    pub id: SharedString,
    pub blocked: Option<&'static str>,
    pub hosting: Option<&'a Hosting>,
}

pub fn worlds(
    hosted: &[Hosted],
    worlds: &[World],
    servers: &[Server],
    counts: &HashMap<String, Count>,
    join: &Join,
    cx: &App,
) -> AnyElement {
    let mut rows: Vec<Row> = hosted
        .iter()
        .map(Row::Hosted)
        .chain(worlds.iter().map(Row::World))
        .chain(servers.iter().map(Row::Server))
        .collect();
    let (shell, id) = (join.shell.clone(), join.id.clone());
    let bar = div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Host a server and play it with your friends, mods and all."),
        )
        .child(
            Button::new("server-new")
                .ghost()
                .cursor_pointer()
                .icon(IconName::Plus)
                .label("New server")
                .on_click(move |_, window, cx| {
                    super::host_view::create(shell.clone(), id.clone(), window, cx)
                }),
        );
    if rows.is_empty() {
        return div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_3()
            .child(bar)
            .child(Empty::new(
                IconName::Globe,
                "No worlds yet",
                "Start a game, join a server or host one and it shows up here.",
            ))
            .into_any_element();
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.saved()));
    let theme = cx.theme();
    let last = rows.len() - 1;
    let list = div()
        .id("worlds")
        .flex()
        .flex_col()
        .min_h_0()
        .overflow_y_scroll()
        .rounded(theme.radius_lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.background)
        .children(rows.iter().enumerate().map(|(ix, row)| {
            let row = match row {
                Row::Hosted(hosted) => hosted_row(ix, hosted, join, cx),
                Row::World(world) => world_row(ix, world, cx),
                Row::Server(server) => server_row(ix, server, counts, join, cx),
            };
            row.when(ix != last, |row| {
                row.border_b_1().border_color(theme.border)
            })
        }));
    div()
        .flex()
        .flex_col()
        .min_h_0()
        .gap_3()
        .child(bar)
        .child(list)
        .into_any_element()
}

/// A world's screenshot, or a globe when the game hasn't saved one.
pub(super) fn thumb(world: &World, cx: &App) -> Div {
    picture(world, IconName::Globe, cx)
}

/// A world's screenshot, or `icon` when the game hasn't saved one.
fn picture(world: &World, icon: IconName, cx: &App) -> Div {
    match &world.thumb {
        Some(path) => div().flex_none().size(THUMB).child(
            img(path.clone())
                .size_full()
                .rounded(px(8.))
                .object_fit(ObjectFit::Cover)
                .with_loading(|| {
                    Skeleton::new()
                        .size_full()
                        .rounded(px(8.))
                        .into_any_element()
                }),
        ),
        None => placeholder(icon, cx),
    }
}

/// `icon` where a screenshot would go.
fn placeholder(icon: IconName, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(THUMB)
        .rounded(px(8.))
        .bg(theme.muted)
        .text_color(theme.muted_foreground)
        .child(Icon::new(icon).size_4())
}

/// A row: picture, title over its details side by side, then its buttons.
fn frame(picture: Div, title: String, details: impl IntoIterator<Item = Meta>) -> Div {
    div()
        .flex()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .child(picture)
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
                        .child(title),
                )
                .child(div().flex().gap_4().children(details)),
        )
}

fn played(world: &World) -> Meta {
    let ago = super::store::now().saturating_sub(world.saved);
    Meta::new(IconName::Clock, format!("Played {}", super::ago(ago)))
}

fn open_folder(id: (&'static str, usize), world: &World) -> Button {
    let dir = world.dir.clone();
    Button::new(id)
        .ghost()
        .cursor_pointer()
        .icon(IconName::FolderOpen)
        .tooltip("Open folder")
        .on_click(move |_, _, cx| cx.open_with_system(&dir))
}

fn world_row(ix: usize, world: &World, cx: &App) -> Div {
    let details = [Meta::new(IconName::Gamepad, world.mode.clone())]
        .into_iter()
        // The title is the given name: say who plays it too.
        .chain(
            world
                .character
                .clone()
                .filter(|character| character != world.title())
                .map(|character| Meta::new(IconName::CircleUser, character)),
        )
        .chain([played(world)]);
    frame(thumb(world, cx), world.title().to_owned(), details)
        .child(open_folder(("world-open", ix), world))
}

fn server_row(
    ix: usize,
    server: &Server,
    counts: &HashMap<String, Count>,
    join: &Join,
    cx: &App,
) -> Div {
    let address = server.address();
    let players = match counts.get(&address).copied().unwrap_or(Count::Asking) {
        Count::Asking => "Checking who's on".to_owned(),
        Count::Online(players) => format!("{} of {} online", players.online, players.max),
        Count::Silent => "Not answering".to_owned(),
    };
    // Only a server in the game's list is asked (see `Detail`), so only it has a count.
    let details = server
        .name
        .is_some()
        .then(|| Meta::new(IconName::Users, players))
        .into_iter()
        // A named server says where it is too; an unnamed one has that as its title.
        .chain(
            server
                .name
                .is_some()
                .then(|| Meta::new(IconName::Server, address.clone())),
        )
        .chain(
            server
                .world
                .character
                .clone()
                .map(|character| Meta::new(IconName::CircleUser, character)),
        )
        .chain([played(&server.world)]);
    let (shell, id) = (join.shell.clone(), join.id.clone());
    frame(
        picture(&server.world, IconName::Server, cx),
        server.title(),
        details,
    )
    .child(
        Button::new(("server-join", ix))
            .ghost()
            .icon(IconName::Play)
            .label("Join")
            .map(|button| match join.blocked {
                Some(why) => button.disabled(true).tooltip(why),
                None => button.cursor_pointer().on_click(move |_, window, cx| {
                    Shell::join(shell.clone(), id.clone(), address.clone(), window, cx)
                }),
            }),
    )
    .child(open_folder(("server-open", ix), &server.world))
}

/// A server the instance hosts: Start, then while it runs Play, Invite, the join link, Stop.
fn hosted_row(ix: usize, hosted: &Hosted, join: &Join, cx: &App) -> Div {
    let mine = join
        .hosting
        .filter(|h| h.instance == join.id && h.name == hosted.name);
    let state = mine.map(|h| h.state.clone());
    let picture = match &hosted.world {
        Some(world) => picture(world, IconName::House, cx),
        None => placeholder(IconName::House, cx),
    };
    let details: Vec<Meta> = match &state {
        Some(State::Starting(what)) => vec![
            Meta::new(IconName::LoaderCircle, what.clone()),
            // Build 42 takes minutes, more the first time and with many mods.
            Meta::new(IconName::Clock, "This takes a few minutes, grab a snack"),
        ],
        Some(State::Running) => vec![
            Meta::new(IconName::Radio, "Up and running"),
            // The first one in becomes the host, and the server closes when the host leaves.
            Meta::new(IconName::Users, "Join first, it closes when you leave"),
        ],
        Some(State::Stopping) => vec![Meta::new(
            IconName::LoaderCircle,
            "Saving and shutting down",
        )],
        Some(State::Failed(_)) => {
            vec![Meta::new(IconName::TriangleAlert, "Didn't start last time")]
        }
        Some(State::Stopped) | None => [Meta::new(IconName::House, "Hosted here")]
            .into_iter()
            .chain(hosted.world.as_ref().map(played))
            .collect(),
    };
    let (shell, id) = (join.shell.clone(), join.id.clone());
    let row = frame(picture, hosted.name.clone(), details);
    let row = match (state, mine) {
        (Some(State::Running), Some(hosting)) => {
            let connect = hosting.connect().unwrap_or_default();
            // What the game's own invites carry; Steam starts the game with it.
            let link = format!("steam://run/{}//+connect%20{connect}", super::APP);
            let (play, invite) = (shell.clone(), shell.clone());
            row.child(
                Button::new(("hosted-play", ix))
                    .ghost()
                    .icon(IconName::Play)
                    .label("Play")
                    .map(|button| match join.blocked {
                        Some(why) => button.disabled(true).tooltip(why),
                        None => button.cursor_pointer().on_click(move |_, window, cx| {
                            Shell::join(play.clone(), id.clone(), connect.clone(), window, cx)
                        }),
                    }),
            )
            .child(
                Button::new(("hosted-invite", ix))
                    .ghost()
                    .cursor_pointer()
                    .icon(IconName::UserPlus)
                    .tooltip("Invite a friend")
                    .on_click(move |_, window, cx| {
                        super::host_view::invite(invite.clone(), window, cx)
                    }),
            )
            .child(
                Button::new(("hosted-link", ix))
                    .ghost()
                    .cursor_pointer()
                    .icon(IconName::Link)
                    .tooltip("Copy the join link for invited friends")
                    .on_click(move |_, window, cx| {
                        use gpui_kit::component::WindowExt;
                        cx.write_to_clipboard(ClipboardItem::new_string(link.clone()));
                        window.push_notification(
                            crate::ui::toast::done("Join link copied! Send it to who you invited"),
                            cx,
                        );
                    }),
            )
            .child(stop(ix, shell))
        }
        (Some(State::Starting(_)), _) => row.child(stop(ix, shell)),
        (Some(State::Stopping), _) => row.child(
            Button::new(("hosted-stopping", ix))
                .ghost()
                .icon(IconName::Square)
                .label("Stopping")
                .disabled(true),
        ),
        _ => {
            let elsewhere = join.hosting.is_some_and(|h| h.alive());
            let blocked = match join.blocked {
                Some("The game is already open") => Some("Close the game first"),
                Some(why) => Some(why),
                None if elsewhere => Some("Another server is running, stop it first"),
                None => None,
            };
            let name = hosted.name.clone();
            row.child(
                Button::new(("hosted-start", ix))
                    .ghost()
                    .icon(IconName::Play)
                    .label("Start")
                    .map(|button| match blocked {
                        Some(why) => button.disabled(true).tooltip(why),
                        None => button.cursor_pointer().on_click(move |_, window, cx| {
                            Shell::host(shell.clone(), id.clone(), name.clone(), window, cx)
                        }),
                    }),
            )
        }
    };
    row.children(
        hosted
            .world
            .as_ref()
            .map(|world| open_folder(("hosted-open", ix), world)),
    )
}

fn stop(ix: usize, shell: WeakEntity<Shell>) -> Button {
    Button::new(("hosted-stop", ix))
        .ghost()
        .cursor_pointer()
        .icon(IconName::Square)
        .tooltip("Stop the server, it saves first")
        .on_click(move |_, _, cx| {
            let _ = shell.update(cx, |shell, cx| shell.stop_hosting(cx));
        })
}
