//! The selected instance: header with Play, and its Mods / Worlds / Logs tabs.

use gpui_kit::assets::IconName;
use gpui_kit::component::ActiveTheme;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{Instance, Status};
use crate::shell::Shell;
use crate::ui::{FancyButton, Meta};

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Mods,
    Worlds,
    Logs,
}

impl Tab {
    const ALL: [Tab; 3] = [Tab::Mods, Tab::Worlds, Tab::Logs];

    fn label(self) -> &'static str {
        match self {
            Tab::Mods => "Mods",
            Tab::Worlds => "Worlds",
            Tab::Logs => "Logs",
        }
    }
}

pub struct Detail {
    instance: Instance,
    tab: Tab,
    shell: WeakEntity<Shell>,
    /// Made on first view; it needs a window for its search field.
    logs: Option<Entity<super::logs_view::Logs>>,
    /// Made on first view too, for its link field.
    mods: Option<Entity<super::mods_view::Mods>>,
    /// Read when the Worlds tab opens, and again after the instance changes.
    worlds: Option<Vec<super::worlds::World>>,
}

impl Detail {
    pub fn new(instance: Instance, shell: WeakEntity<Shell>) -> Self {
        Self {
            instance,
            tab: Tab::Mods,
            shell,
            logs: None,
            mods: None,
            worlds: None,
        }
    }

    pub fn id(&self) -> SharedString {
        self.instance.id.clone()
    }

    /// Something about the instance changed outside this view (a download moved or ended).
    pub fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(mods) = &self.mods {
            mods.update(cx, |mods, cx| mods.refresh(cx));
        }
        self.worlds = None;
        cx.notify();
    }

    pub fn show(&mut self, instance: Instance, cx: &mut Context<Self>) {
        if instance.id != self.instance.id {
            self.logs = None;
            self.mods = None;
        }
        self.worlds = None;
        self.instance = instance;
        cx.notify();
    }

    fn header(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let instance = &self.instance;
        let played = instance.played();
        let latest = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).latest(instance));
        let status = self
            .shell
            .upgrade()
            .map_or(instance.status(latest), |shell| {
                shell.read(cx).status(instance)
            });
        let newest = self
            .shell
            .upgrade()
            .and_then(|shell| shell.read(cx).latest_name(instance));
        let note = match (status, instance.build_name(), newest) {
            (Status::Installing(_) | Status::Bringing(_), _, _) => Some(status.label()),
            (Status::Outdated, Some(have), Some(newest)) => {
                Some(format!("Update available: {have} \u{2192} {newest}"))
            }
            _ => None,
        };

        div()
            .flex()
            .items_center()
            .gap_4()
            .child(super::art::art(instance, 56., theme.radius_lg, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::BOLD)
                            .truncate()
                            .child(instance.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .children(instance.version().map(|v| Meta::new(IconName::Gamepad, v)))
                            .child(Meta::new(IconName::GitBranch, instance.channel()))
                            .child(Meta::new(IconName::Clock, played))
                            .children(note.map(|text| Meta::new(IconName::Download, text))),
                    )
                    // As wide as the line of facts above it, not the whole header: a bar running
                    // to the buttons reads as belonging to them.
                    .children(
                        super::bar(&instance.id, status, cx)
                            .map(|bar| div().w(px(340.)).max_w_full().child(bar)),
                    ),
            )
            .when(status == Status::Outdated, |this| {
                let (shell, id) = (self.shell.clone(), instance.id.clone());
                this.child(
                    Button::new("update")
                        .outline()
                        .cursor_pointer()
                        .icon(IconName::Download)
                        .label("Update")
                        .h(px(40.))
                        .rounded(px(10.))
                        .on_click(move |_, window, cx| {
                            Shell::confirm_update(shell.clone(), id.clone(), window, cx)
                        }),
                )
            })
            .child({
                let (label, icon, enabled) = status.action();
                let (shell, id) = (self.shell.clone(), instance.id.clone());
                // One width for Play, Install, Cancel and Stop, so the header doesn't shift
                // as the instance changes state.
                FancyButton::new("play", label)
                    .width(px(124.))
                    .icon(icon)
                    .when(enabled, |button| {
                        button.on_click(move |_, window, cx| {
                            Shell::play(shell.clone(), id.clone(), window, cx)
                        })
                    })
            })
            .child(self.more(instance))
    }

    /// Everything but Play, behind one button: the header keeps a single strong action.
    fn more(&self, instance: &Instance) -> impl IntoElement + use<> {
        let (shell, instance) = (self.shell.clone(), instance.clone());
        Button::new("instance-more")
            .ghost()
            .cursor_pointer()
            .icon(IconName::EllipsisVertical)
            .size(px(40.))
            .rounded(px(10.))
            .dropdown_menu(move |menu, _, _| {
                let on = |action: fn(WeakEntity<Shell>, SharedString, &mut Window, &mut App)| {
                    let (shell, id) = (shell.clone(), instance.id.clone());
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        action(shell.clone(), id.clone(), window, cx)
                    }
                };
                let (pack, folder) = (instance.clone(), instance.dir.clone());
                menu.item(
                    PopupMenuItem::new("Settings")
                        .icon(IconName::Settings)
                        .on_click(on(Shell::settings)),
                )
                .item(
                    PopupMenuItem::new("Edit icon")
                        .icon(IconName::Palette)
                        .on_click(on(Shell::customize)),
                )
                // Same groups as the Library card's menu: the instance, its files, copies of
                // it, and last the one that can't be undone.
                .separator()
                .item(
                    PopupMenuItem::new("Open folder")
                        .icon(IconName::FolderOpen)
                        .on_click(move |_, _, cx| cx.open_with_system(&folder)),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Duplicate")
                        .icon(IconName::Copy)
                        .on_click(on(Shell::duplicate)),
                )
                .item(
                    PopupMenuItem::new("Export as .lata")
                        .icon(IconName::Box)
                        .on_click(move |_, window, cx| {
                            crate::pack::export::open(pack.clone(), window, cx)
                        }),
                )
                .separator()
                .item(
                    PopupMenuItem::new("Delete")
                        .icon(IconName::Trash)
                        .on_click(on(Shell::confirm_delete)),
                )
            })
    }

    /// Mods, Worlds, Logs as pills in a tray: the chosen one is filled with the accent, and
    /// the fill slides between them on a spring (gpui-component's TabBar measures
    /// each tab and keeps the indicator interruptible).
    fn tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        use gpui_kit::component::tab::{Tab as Item, TabBar};
        let selected = Tab::ALL.iter().position(|t| *t == self.tab).unwrap_or(0);
        use gpui_kit::component::Sizable as _;
        TabBar::new("instance-tabs")
            .pill()
            // 36 px, the height of the buttons around it.
            .large()
            .selected_index(selected)
            .children(Tab::ALL.map(|tab| Item::new().label(tab.label()).cursor_pointer()))
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.tab = Tab::ALL[*ix];
                this.worlds = None;
                cx.notify();
            }))
    }
}

impl Render for Detail {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.tab {
            Tab::Logs => {
                let cachedir = self.instance.dir.join("zomboid");
                self.logs
                    .get_or_insert_with(|| {
                        cx.new(|cx| super::logs_view::Logs::new(&cachedir, window, cx))
                    })
                    .clone()
                    .into_any_element()
            }
            Tab::Mods => {
                let (instance, shell) = (self.instance.clone(), self.shell.clone());
                self.mods
                    .get_or_insert_with(|| {
                        cx.new(|cx| super::mods_view::Mods::new(instance, shell, window, cx))
                    })
                    .clone()
                    .into_any_element()
            }
            Tab::Worlds => {
                let cachedir = self.instance.dir.join("zomboid");
                let worlds = self
                    .worlds
                    .get_or_insert_with(|| super::worlds::list(&cachedir));
                super::worlds_view::worlds(worlds, cx)
            }
        };
        let tabs = self.tabs(cx).into_any_element();
        div()
            .flex()
            .flex_col()
            .size_full()
            .gap_5()
            .p_6()
            .child(self.header(cx))
            // The tabs are their own island: a tray that holds the three pills, so they read
            // as one control. What a tab shows stays a separate island below.
            .child(div().flex().child(crate::ui::tab_tray(tabs, cx)))
            .child(body)
    }
}
