//! The Worlds tab: the instance's saves and the servers it has joined, in the Mods table's
//! style, the last played first.

use std::collections::HashMap;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, Disableable as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::servers::{Count, Server};
use super::worlds::World;
use crate::shell::Shell;
use crate::ui::{Empty, Meta};

/// Side of a world's screenshot in its row; the game saves them square.
const THUMB: Pixels = px(48.);

/// One row of the tab. Servers the instance hosts (#49) will be another kind.
enum Row<'a> {
    World(&'a World),
    Server(&'a Server),
}

impl Row<'_> {
    fn saved(&self) -> u64 {
        match self {
            Row::World(world) => world.saved,
            Row::Server(server) => server.world.saved,
        }
    }
}

/// What the Join buttons need: the instance, and why it can't join right now.
pub struct Join {
    pub shell: WeakEntity<Shell>,
    pub id: SharedString,
    pub blocked: Option<&'static str>,
}

pub fn worlds(
    worlds: &[World],
    servers: &[Server],
    counts: &HashMap<String, Count>,
    join: &Join,
    cx: &App,
) -> AnyElement {
    let mut rows: Vec<Row> = worlds
        .iter()
        .map(Row::World)
        .chain(servers.iter().map(Row::Server))
        .collect();
    if rows.is_empty() {
        return Empty::new(
            IconName::Globe,
            "No worlds yet",
            "Start a game or join a server and it shows up here.",
        )
        .into_any_element();
    }
    rows.sort_by_key(|row| std::cmp::Reverse(row.saved()));
    let theme = cx.theme();
    let last = rows.len() - 1;
    div()
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
                Row::World(world) => world_row(ix, world, cx),
                Row::Server(server) => server_row(ix, server, counts, join, cx),
            };
            row.when(ix != last, |row| {
                row.border_b_1().border_color(theme.border)
            })
        }))
        .into_any_element()
}

/// A world's screenshot, or a globe when the game hasn't saved one.
pub(super) fn thumb(world: &World, cx: &App) -> Div {
    picture(world, IconName::Globe, cx)
}

/// A world's screenshot, or `icon` when the game hasn't saved one.
fn picture(world: &World, icon: IconName, cx: &App) -> Div {
    let theme = cx.theme();
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
        None => div()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(THUMB)
            .rounded(px(8.))
            .bg(theme.muted)
            .text_color(theme.muted_foreground)
            .child(Icon::new(icon).size_4()),
    }
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
