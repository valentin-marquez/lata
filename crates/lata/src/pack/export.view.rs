//! The export dialog (Modrinth App's "Export modpack"): name, version and description, then
//! the instance's files in a tree to check what goes, with what makes the pack play the same
//! checked from the start.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::component::input::{Input, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme, Icon, WindowExt};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{About, Node, PICKED};
use crate::instance::{Instance, install};
use crate::ui::{self, FancyButton, Loader, toast};

const WIDTH: f32 = 720.;
/// The file table's height: about ten rows, the rest scrolls.
const TABLE: Pixels = px(360.);
const ROW: Pixels = px(36.);
const SIZE: Pixels = px(96.);
const MODIFIED: Pixels = px(120.);
/// Indent per folder level.
const INDENT: f32 = 20.;

pub fn open(instance: Instance, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| Export::new(instance, window, cx));
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .entrance(DialogEntrance::FadeSlide)
            .title("Export pack")
            .centered()
            .w(px(WIDTH))
            .child(view.clone())
    });
}

/// A row of the table: a node shown at its depth.
#[derive(Clone)]
struct Row {
    path: PathBuf,
    name: String,
    depth: usize,
    folder: bool,
    size: u64,
    modified: Option<SystemTime>,
}

struct Export {
    instance: Instance,
    name: Entity<InputState>,
    version: Entity<InputState>,
    summary: Entity<TextareaState>,
    /// `None` while the folder is read.
    tree: Option<Vec<Node>>,
    /// Every file under each folder, to check and count a folder at once.
    files: HashMap<PathBuf, Vec<PathBuf>>,
    /// The size of each file.
    sizes: HashMap<PathBuf, u64>,
    /// The checked files; a folder is as checked as the files in it.
    picked: HashSet<PathBuf>,
    /// Folders showing what they hold.
    open: HashSet<PathBuf>,
    /// The rows in view, rebuilt when a folder opens or closes.
    rows: Vec<Row>,
    scroll: UniformListScrollHandle,
    /// Writing the pack.
    busy: bool,
}

impl Export {
    fn new(instance: Instance, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Pack name")
                .default_value(instance.name.to_string())
        });
        let version = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("1.0.0")
                .default_value("1.0.0")
        });
        let summary = cx.new(|cx| {
            TextareaState::new(window, cx)
                .rows(3)
                .placeholder("What's in it, and how to play it")
        });
        let reading = instance.clone();
        cx.spawn(async move |this, cx| {
            let Ok(tree) = crate::runtime::blocking(move || super::tree(&reading)).await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.load(tree);
                cx.notify();
            });
        })
        .detach();
        Self {
            instance,
            name,
            version,
            summary,
            tree: None,
            files: HashMap::new(),
            sizes: HashMap::new(),
            picked: HashSet::new(),
            open: HashSet::new(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            busy: false,
        }
    }

    /// Takes the folder as read, with [`PICKED`] checked.
    fn load(&mut self, tree: Vec<Node>) {
        fn index(
            nodes: &[Node],
            files: &mut HashMap<PathBuf, Vec<PathBuf>>,
            sizes: &mut HashMap<PathBuf, u64>,
        ) -> Vec<PathBuf> {
            let mut all = Vec::new();
            for node in nodes {
                match &node.children {
                    Some(children) => {
                        let inside = index(children, files, sizes);
                        all.extend(inside.iter().cloned());
                        files.insert(node.path.clone(), inside);
                    }
                    None => {
                        sizes.insert(node.path.clone(), node.size);
                        all.push(node.path.clone());
                    }
                }
            }
            all
        }
        index(&tree, &mut self.files, &mut self.sizes);
        for picked in PICKED {
            let path = Path::new(picked);
            match self.files.get(path) {
                Some(files) => self.picked.extend(files.iter().cloned()),
                None if self.sizes.contains_key(path) => {
                    self.picked.insert(path.to_owned());
                }
                None => {}
            }
        }
        self.tree = Some(tree);
        self.layout();
    }

    /// Lays the rows out again: every node whose folders are all open.
    fn layout(&mut self) {
        fn add(nodes: &[Node], depth: usize, open: &HashSet<PathBuf>, rows: &mut Vec<Row>) {
            for node in nodes {
                rows.push(Row {
                    path: node.path.clone(),
                    name: node.name.clone(),
                    depth,
                    folder: node.children.is_some(),
                    size: node.size,
                    modified: node.modified,
                });
                if let Some(children) = &node.children
                    && open.contains(&node.path)
                {
                    add(children, depth + 1, open, rows);
                }
            }
        }
        self.rows.clear();
        if let Some(tree) = &self.tree {
            add(tree, 0, &self.open, &mut self.rows);
        }
    }

    /// The files a row stands for: itself, or everything in its folder.
    fn under(&self, row: &Row) -> Vec<PathBuf> {
        match self.files.get(&row.path) {
            Some(files) => files.clone(),
            None if row.folder => Vec::new(),
            None => vec![row.path.clone()],
        }
    }

    /// Whether all of a row's files are checked; `None` for some of them.
    fn checked(&self, row: &Row) -> Option<bool> {
        let files = self.under(row);
        let on = files.iter().filter(|f| self.picked.contains(*f)).count();
        match on {
            0 => Some(false),
            on if on == files.len() => Some(true),
            _ => None,
        }
    }

    fn pick(&mut self, row: &Row, on: bool) {
        for file in self.under(row) {
            if on {
                self.picked.insert(file);
            } else {
                self.picked.remove(&file);
            }
        }
    }

    fn toggle_open(&mut self, path: &Path) {
        if !self.open.remove(path) {
            self.open.insert(path.to_owned());
        }
        self.layout();
    }

    /// Asks where to save, then writes the pack there.
    fn export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let about = About {
            name: self.name.read(cx).value().to_string(),
            version: self.version.read(cx).value().to_string(),
            summary: self.summary.read(cx).value().to_string(),
        };
        let shown = match about.name.trim() {
            "" => self.instance.name.to_string(),
            name => name.to_owned(),
        };
        // The name as a file name: what some systems refuse in one becomes a space.
        let file: String = format!("{shown} {}", about.version.trim())
            .chars()
            .map(|c| if "/\\:*?\"<>|".contains(c) { ' ' } else { c })
            .collect();
        let file = format!("{}.{}", file.trim(), super::EXT);
        let folder = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_default();
        let asked = cx.prompt_for_new_path(&folder, Some(&file));
        let (instance, picked) = (
            self.instance.clone(),
            self.picked.iter().cloned().collect::<Vec<_>>(),
        );
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(mut out))) = asked.await else {
                return;
            };
            if out.extension().is_none() {
                out.set_extension(super::EXT);
            }
            let _ = this.update(cx, |this, cx| {
                this.busy = true;
                cx.notify();
            });
            let written =
                crate::runtime::blocking(move || super::export(&instance, &about, &picked, &out))
                    .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match written {
                    Ok(Ok(())) => {
                        window.close_dialog(cx);
                        window.push_notification(
                            toast::done(format!("Canned! {shown} is ready to share")),
                            cx,
                        );
                    }
                    Ok(Err(why)) => window.push_notification(
                        toast::failed_because("That pack didn't get canned", why),
                        cx,
                    ),
                    Err(why) => window.push_notification(
                        toast::failed_because("That pack didn't get canned", why),
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The table's first row: check all, and the column names.
    fn header(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let total = self.sizes.len();
        let all = total > 0 && self.picked.len() == total;
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
            .h(px(40.))
            .px_3()
            .bg(theme.muted)
            .border_b_1()
            .border_color(theme.border)
            .child(
                Checkbox::new("export-all")
                    .cursor_pointer()
                    .checked(all)
                    .on_click(cx.listener(|this, on: &bool, _, cx| {
                        if *on {
                            this.picked = this.sizes.keys().cloned().collect();
                        } else {
                            this.picked.clear();
                        }
                        cx.notify();
                    })),
            )
            .child(name("Name").flex_1())
            .child(name("Size").w(SIZE))
            .child(name("Modified").w(MODIFIED))
            .child(div().w(px(16.)))
    }

    fn row(&self, row: &Row, last: bool, cx: &mut Context<Self>) -> Stateful<Div> {
        let theme = cx.theme();
        let checked = self.checked(row);
        let open = self.open.contains(&row.path);
        let icon = if row.folder {
            "icons/folder.svg"
        } else {
            "icons/file.svg"
        };
        let modified = row
            .modified
            .map(|at| {
                chrono::DateTime::<chrono::Local>::from(at)
                    .format("%b %-d, %Y")
                    .to_string()
            })
            .unwrap_or_default();
        let (for_check, for_open) = (row.clone(), row.path.clone());
        div()
            .id(SharedString::from(format!("export-{}", row.path.display())))
            .flex()
            .w_full()
            .items_center()
            .gap_3()
            .h(ROW)
            .px_3()
            .when(!last, |this| this.border_b_1().border_color(theme.border))
            .hover(|this| this.bg(ui::hovered(theme.background, cx)))
            .when(row.folder, |this| {
                this.cursor_pointer()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_open(&for_open);
                        cx.notify();
                    }))
            })
            .child(
                // Some of a folder's files: shown unchecked, and a click checks them all.
                Checkbox::new(SharedString::from(format!("pick-{}", row.path.display())))
                    .cursor_pointer()
                    .checked(checked == Some(true))
                    .on_click(cx.listener(move |this, on: &bool, _, cx| {
                        cx.stop_propagation();
                        this.pick(&for_check, *on);
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .pl(px(INDENT * row.depth as f32))
                    .child(
                        Icon::empty()
                            .path(icon)
                            .size_4()
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .child(div().text_sm().truncate().child(row.name.clone()))
                    .when(checked.is_none(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("Some files"),
                        )
                    }),
            )
            .child(
                div()
                    .w(SIZE)
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(install::size(row.size)),
            )
            .child(
                div()
                    .w(MODIFIED)
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(modified),
            )
            .child(div().w(px(16.)).children(row.folder.then(|| {
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size_4()
                .text_color(theme.muted_foreground)
            })))
    }
}

fn label(text: &'static str) -> Div {
    div()
        .text_sm()
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

impl Render for Export {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let base = self.scroll.0.borrow().base_handle.clone();
        ui::smooth_scroll("export-scroll", &base, window, cx);
        let theme = cx.theme().clone();
        let table = match &self.tree {
            None => div().h(TABLE).child(Loader).into_any_element(),
            Some(_) => div()
                .flex()
                .flex_col()
                .h(TABLE)
                .rounded(theme.radius_lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.background)
                .overflow_hidden()
                .child(self.header(cx))
                .child(
                    uniform_list(
                        "export-rows",
                        self.rows.len(),
                        cx.processor(|this, range: Range<usize>, _, cx| {
                            let last = this.rows.len().saturating_sub(1);
                            let rows: Vec<(usize, Row)> = range
                                .filter_map(|ix| this.rows.get(ix).map(|r| (ix, r.clone())))
                                .collect();
                            rows.iter()
                                .map(|(ix, row)| this.row(row, *ix == last, cx))
                                .collect()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .flex_1()
                    .min_h_0(),
                )
                .into_any_element(),
        };
        let bytes: u64 = self
            .picked
            .iter()
            .filter_map(|file| self.sizes.get(file))
            .sum();
        let count = match self.picked.len() {
            1 => "1 file".to_owned(),
            many => format!("{many} files"),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .gap_1p5()
                            .child(label("Name"))
                            .child(Input::new(&self.name)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .w(px(200.))
                            .gap_1p5()
                            .child(label("Version"))
                            .child(Input::new(&self.version)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(label("Description"))
                    .child(Textarea::new(&self.summary)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(label("Files"))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Workshop mods go as links to their exact version, so they don't count here."),
                    )
                    .child(table),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .gap_3()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(count)
                            .child(install::size(bytes)),
                    )
                    .child(
                        ui::secondary("export-cancel", "Cancel")
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        FancyButton::new("export", "Export")
                            .small()
                            .icon(IconName::Box)
                            .disabled(self.busy || self.tree.is_none())
                            .on_click(cx.listener(|this, _, window, cx| this.export(window, cx))),
                    ),
            )
    }
}
