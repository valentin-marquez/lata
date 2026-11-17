//! Window frame: title bar, navigation and instances on the left, the current page on the right.

#[path = "downloads.view.rs"]
mod downloads;
#[path = "sidebar.view.rs"]
mod sidebar;
#[path = "topbar.view.rs"]
mod topbar;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;

use gpui_kit::component::ActiveTheme;
use gpui_kit::component::dialog::DialogEntrance;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::account::{self, Account};
use crate::instance::{self, Instance, Status, detail::Detail, install, launch, store};
use crate::library::Library;
use crate::screenshots;
use crate::ui::Loader;
use topbar::Topbar;

#[derive(Clone, Copy, PartialEq)]
pub enum Page {
    Library,
    Screenshots,
    Instance(usize),
}

pub struct Shell {
    root: PathBuf,
    instances: Vec<Instance>,
    page: Page,
    detail: Option<Entity<Detail>>,
    /// Made on first view; it needs a window for its search field.
    library: Option<Entity<Library>>,
    screenshots: Option<Entity<screenshots::Screenshots>>,
    account: Option<Account>,
    /// Newest Steam build id per branch name; empty until the first check succeeds.
    latest: HashMap<SharedString, instance::branches::Branch>,
    /// Downloads in progress: a game install by its instance id, a mod by [`mod_key`].
    installs: HashMap<SharedString, install::Download>,
    /// Games running, by instance id, with the process lata spawned if it started the game in
    /// this run (kept to reap it).
    running: HashMap<SharedString, Option<std::process::Child>>,
    /// News and trending mods for the panel on the right; `None` until Steam answers.
    feed: Option<crate::discover::Feed>,
    /// The banner the last run showed, for the Library until the feed lands.
    cached_banner: Option<std::path::PathBuf>,
    /// How many trending mods fit whole in the panel, measured as it is drawn.
    pub(crate) trending_fit: usize,
    /// Scroll position of the sidebar's instance list, eased by `ui::smooth_scroll`.
    pub(crate) sidebar_scroll: ScrollHandle,
    /// The Workshop id of the link whose install dialog is open, if one is.
    link_open: Option<u64>,
    /// Mods from other links that arrived while that dialog was open; each gets its turn.
    link_queue: VecDeque<crate::link::Found>,
    /// How many links are being looked up on Steam right now; the loader shows meanwhile.
    link_resolving: usize,
    /// A linked mod that found no instance to go into: it is offered again once one is created.
    link_waiting: Option<crate::instance::mods::Listing>,
    /// Shows the loader under the title bar while the page's data loads.
    loading: bool,
    /// A newer lata, once a check finds one.
    update: Option<crate::update::Found>,
}

impl Shell {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let root = store::root();
        store::migrate(&root);
        cx.spawn(async move |this, cx| {
            let saved = account::load().await;
            let _ = this.update(cx, |shell, cx| {
                shell.account = saved;
                cx.notify();
            });
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let feed = crate::discover::load().await;
            let _ = this.update(cx, |shell, cx| {
                shell.feed = Some(feed);
                shell.changed(cx);
            });
        })
        .detach();
        let mut shell = Self {
            feed: None,
            cached_banner: crate::discover::cached_banner(),
            trending_fit: usize::MAX,
            sidebar_scroll: ScrollHandle::new(),
            account: None,
            latest: HashMap::new(),
            installs: HashMap::new(),
            running: HashMap::new(),
            instances: store::list(&root),
            root,
            page: Page::Library,
            detail: None,
            library: None,
            screenshots: None,
            link_open: None,
            link_queue: VecDeque::new(),
            link_resolving: 0,
            link_waiting: None,
            loading: false,
            update: None,
        };
        shell.check_updates(cx);
        shell.check_builds(cx);
        shell.find_games(cx);
        shell
    }

    /// Picks up games that were already open when lata started, so they show as running.
    fn find_games(&mut self, cx: &mut Context<Self>) {
        let instances = self.instances.clone();
        cx.spawn(async move |this, cx| {
            let open: Vec<Instance> = cx
                .background_executor()
                .spawn(async move { instances.into_iter().filter(launch::alive).collect() })
                .await;
            let _ = this.update(cx, |shell, cx| {
                for instance in open {
                    shell.watch(instance, None, cx);
                }
            });
        })
        .detach();
    }

    /// Looks for a newer lata now and every few hours, quietly: offline or refused just
    /// means asking again later. The setting is read each time, so turning it off stops it.
    /// The update window already asked at startup where lata can install updates; this is
    /// for a lata left open, for where it can't, and for when GitHub was slow to answer then.
    fn check_updates(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                if crate::update::enabled()
                    && let Ok(Some(found)) = crate::update::check().await
                    && this
                        .update(cx, |shell, cx| shell.update_found(found, cx))
                        .is_err()
                {
                    return;
                }
                cx.background_executor().timer(crate::update::EVERY).await;
            }
        })
        .detach();
    }

    /// A check found a newer release: the title bar offers it, and a toast says so once per
    /// version. Nothing is interrupted.
    fn update_found(&mut self, found: crate::update::Found, cx: &mut Context<Self>) {
        let version = found.release.version.clone();
        if self
            .update
            .as_ref()
            .is_some_and(|known| known.release.version >= version)
        {
            return;
        }
        self.update = Some(found);
        let label = self.update_button().map(|(label, _)| label);
        cx.notify();
        let shell = cx.entity().downgrade();
        notify(cx, move || {
            crate::ui::toast::update(
                format!("lata {version} is out"),
                "Fresh out of the can, whenever you feel like it",
                label.unwrap_or_default(),
                shell,
            )
        });
    }

    /// The newer lata found, and whether a restart would install it.
    pub fn newer(&self) -> Option<(String, bool)> {
        let found = self.update.as_ref()?;
        Some((found.release.version.to_string(), found.installable))
    }

    /// The label and icon of the update button in the title bar; none while there is
    /// nothing newer.
    fn update_button(&self) -> Option<(SharedString, gpui_kit::assets::IconName)> {
        use gpui_kit::assets::IconName;
        let found = self.update.as_ref()?;
        Some(if found.installable {
            ("Restart to update".into(), IconName::Rocket)
        } else {
            // lata can't replace itself here: the button opens the release's page.
            (
                format!("Get lata {}", found.release.version).into(),
                IconName::ExternalLink,
            )
        })
    }

    /// What the update button and the toast's button do: close lata and start it again
    /// through the update window, which installs the release; or open the release's page
    /// where lata can't install it.
    pub fn update_lata(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
        use gpui_kit::component::WindowExt;
        let outcome = shell.update(cx, |shell, cx| {
            let found = shell.update.as_ref()?;
            if !found.installable {
                cx.open_url(&found.release.page());
                return None;
            }
            // Restarting would take a running game's log and the downloads with it, and a
            // mod link that is being looked up or waits in its dialog.
            let links = shell.link_resolving > 0
                || shell.link_open.is_some()
                || !shell.link_queue.is_empty();
            if shell.busy() || links {
                return Some(crate::ui::toast::heads_up(
                    "Hold on, lata is busy",
                    "Restart once the game is closed and the downloads and mod links are done",
                ));
            }
            match crate::update::restart() {
                Ok(()) => {
                    cx.quit();
                    None
                }
                Err(err) => Some(crate::ui::toast::failed_because(
                    "Oof, lata couldn't restart itself",
                    format!("{err}. Close it and open it again to update"),
                )),
            }
        });
        if let Ok(Some(toast)) = outcome {
            window.push_notification(toast, cx);
        }
    }

    /// Newest build of the instance's branch, when it is known.
    pub fn latest(&self, instance: &Instance) -> Option<u32> {
        self.latest.get(&instance.branch).map(|b| b.build_id)
    }

    /// What the newest build of the instance's branch is called.
    pub fn latest_name(&self, instance: &Instance) -> Option<String> {
        let branch = self.latest.get(&instance.branch)?;
        Some(instance::build::name(branch.build_id, None, Some(branch)))
    }

    /// Asks Steam anonymously for each branch's build; offline just leaves the marks as they were.
    fn check_builds(&mut self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let Ok(branches) = instance::branches::fetch().await else {
                return;
            };
            let _ = this.update(cx, |shell, cx| {
                shell.latest = branches
                    .into_iter()
                    .map(|b| (SharedString::from(b.name.clone()), b))
                    .collect();
                if let (Page::Instance(ix), Some(detail)) = (shell.page, &shell.detail)
                    && let Some(instance) = shell.instances.get(ix).cloned()
                {
                    detail.update(cx, |d, cx| d.show(instance, cx));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The game screenshot for the Library's banner, once it has been fetched.
    /// `None` while the feed is being asked for and no earlier banner is cached; then the
    /// screenshot, when one came.
    pub fn banner(&self) -> Option<Option<std::path::PathBuf>> {
        match (&self.feed, &self.cached_banner) {
            (Some(feed), _) => Some(feed.banner.clone()),
            (None, Some(cached)) => Some(Some(cached.clone())),
            (None, None) => None,
        }
    }

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        if let Page::Instance(ix) = page {
            let Some(instance) = self.instances.get(ix).cloned() else {
                return;
            };
            match &self.detail {
                Some(detail) => detail.update(cx, |d, cx| d.show(instance, cx)),
                None => {
                    let shell = cx.entity().downgrade();
                    self.detail = Some(cx.new(|_| Detail::new(instance, shell)));
                }
            }
            self.check_builds(cx);
        }
        // New screenshots land while the game runs: look again on every visit.
        if let (Page::Screenshots, Some(view)) = (page, &self.screenshots) {
            let instances = self.instances.clone();
            view.update(cx, |view, cx| view.reload(instances, cx));
        }
        self.page = page;
        cx.notify();
    }

    pub fn status(&self, instance: &Instance) -> Status {
        if self.running.contains_key(&instance.id) {
            return Status::Running;
        }
        // The game downloads alongside, but what someone waits for first is their game coming in.
        if let Some(bringing) = self.installs.get(&mod_key(&instance.id, "existing")) {
            return Status::Bringing(bringing.pct());
        }
        match self.installs.get(&instance.id) {
            Some(download) => Status::Installing(download.pct()),
            None => instance.status(self.latest(instance)),
        }
    }

    /// The main button of an instance: install it if needed, else launch it; stops whatever
    /// it is doing (a running game, an install).
    pub fn play(shell: WeakEntity<Shell>, id: SharedString, window: &mut Window, cx: &mut App) {
        let Ok((instance, account)) = shell.read_with(cx, |shell, _| {
            let instance = shell.instances.iter().find(|i| i.id == id).cloned();
            (instance, shell.account.clone())
        }) else {
            return;
        };
        let Some(instance) = instance else { return };
        let status = shell
            .read_with(cx, |shell, _| shell.status(&instance))
            .unwrap_or(Status::Ready);
        match (status, account) {
            (Status::Running, _) => {
                launch::stop(&instance);
            }
            // The game quits silently without Steam, so say so and offer to open it.
            (Status::Ready | Status::Outdated, _) if !launch::steam_running() => {
                use gpui_kit::component::WindowExt;
                window.push_notification(crate::ui::toast::steam_closed(), cx);
            }
            (Status::Ready | Status::Outdated, _) => match launch::start(&instance) {
                Ok(child) => {
                    let _ = shell.update(cx, |shell, cx| shell.watch(instance, Some(child), cx));
                }
                Err(err) => {
                    use gpui_kit::component::WindowExt;
                    window.push_notification(
                        crate::ui::toast::failed_because("The game refused to start", err),
                        cx,
                    )
                }
            },
            (Status::Installing(_), _) => {
                let _ = shell.update(cx, |shell, cx| shell.cancel(&instance.id, cx));
            }
            (Status::Bringing(_), _) => {
                let _ = shell.update(cx, |shell, cx| {
                    shell.cancel(&mod_key(&instance.id, "existing"), cx);
                    shell.cancel(&instance.id, cx);
                });
            }
            (_, None) => Self::open_account(shell, window, cx),
            (_, Some(account)) => {
                let handle = window.window_handle();
                let _ = shell.update(cx, |shell, cx| shell.install(instance, account, handle, cx));
            }
        }
    }

    /// Asks before updating an outdated instance: a new build can break saves and mods.
    pub fn confirm_update(
        shell: WeakEntity<Shell>,
        id: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) {
        use gpui_kit::component::WindowExt;
        let Ok((instance, account)) = shell.read_with(cx, |shell, _| {
            let instance = shell.instances.iter().find(|i| i.id == id).cloned();
            (
                instance.map(|i| (shell.latest(&i), shell.latest_name(&i), i)),
                shell.account.clone(),
            )
        }) else {
            return;
        };
        let Some((Some(_), Some(newest), instance)) = instance else {
            return;
        };
        let Some(have) = instance.build_name() else {
            return;
        };
        let Some(account) = account else {
            return Self::open_account(shell, window, cx);
        };
        let text = format!(
            "{have} will be replaced by {newest}. A new version can break saves and \
             mods. Files that didn't change are not downloaded again."
        );
        window.open_dialog(cx, move |dialog, _, _| {
            // Fades in settling from just above, and back out (the default drops from the window top).
            let dialog = dialog.entrance(DialogEntrance::FadeSlide);
            let (shell, instance, account) = (shell.clone(), instance.clone(), account.clone());
            dialog
                .title(format!("Update {}?", instance.name))
                .w(px(420.))
                .child(text.clone())
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .pt_4()
                        .child(
                            crate::ui::secondary("cancel-update", "Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            crate::ui::FancyButton::new("confirm-update", "Update")
                                .small()
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let handle = window.window_handle();
                                    let _ = shell.update(cx, |shell, cx| {
                                        shell.install(instance.clone(), account.clone(), handle, cx)
                                    });
                                }),
                        ),
                )
        });
    }

    /// Tracks a running game until it exits, then records the play time.
    /// The game counts as running while any process carries the instance's cachedir, so a game
    /// lata didn't start ends the same way, and the script exiting before the game doesn't.
    fn watch(
        &mut self,
        instance: Instance,
        child: Option<std::process::Child>,
        cx: &mut Context<Self>,
    ) {
        let id = instance.id.clone();
        // Startup's search can land after Play was pressed; that game is already watched.
        if child.is_none() && self.running.contains_key(&id) {
            return;
        }
        self.running.insert(id.clone(), child);
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(1))
                    .await;
                // Reaps the spawned process once it ends, so it doesn't linger as a zombie.
                let _ = this.update(cx, |shell, _| {
                    if let Some(Some(child)) = shell.running.get_mut(&id) {
                        let _ = child.try_wait();
                    }
                });
                let check = instance.clone();
                let alive = cx
                    .background_executor()
                    .spawn(async move { launch::alive(&check) })
                    .await;
                if !alive {
                    break;
                }
            }
            let _ = store::set_last_played(&instance.dir, store::now());
            let _ = this.update(cx, |shell, cx| {
                shell.running.remove(&id);
                shell.reload(cx);
            });
        })
        .detach();
    }

    /// Name of a running game, for the title bar.
    pub fn running(&self) -> Option<SharedString> {
        let id = self.running.keys().next()?;
        self.instances
            .iter()
            .find(|i| &i.id == id)
            .map(|i| i.name.clone())
    }

    fn install(
        &mut self,
        instance: Instance,
        account: Account,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        if self.installs.contains_key(&instance.id) {
            return;
        }
        let id = instance.id.clone();
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let name = instance.name.clone();
        let cancel = install::start(account, instance, events_tx);
        self.installs.insert(
            id.clone(),
            install::Download::new(name, install::Kind::Game, Some(cancel)),
        );
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                let _ = this.update(cx, |shell, cx| match event {
                    install::Event::Progress { done, total } => {
                        // Gone means cancelled; events already queued must not bring it back.
                        if let Some(download) = shell.installs.get_mut(&id)
                            && download.progress(done, total, std::time::Instant::now())
                        {
                            shell.changed(cx);
                        }
                    }
                    install::Event::Done => {
                        shell.installs.remove(&id);
                        shell.reload(cx);
                    }
                    install::Event::Failed(err) => {
                        if shell.installs.remove(&id).is_none() {
                            return;
                        }
                        shell.changed(cx);
                        // The window that started the install, not whichever has focus now.
                        let _ = window.update(cx, |_, window, cx| {
                            use gpui_kit::component::WindowExt;
                            window.push_notification(
                                crate::ui::toast::failed_because("Yikes, the install tripped", err),
                                cx,
                            );
                        });
                    }
                });
            }
        })
        .detach();
    }

    /// Installs in progress, by name so the order holds still between frames.
    /// Whether a mod is being downloaded into instance `id`.
    pub fn adding_mods(&self, id: &SharedString) -> bool {
        let prefix = mod_key(id, "");
        self.installs.keys().any(|key| key.starts_with(&*prefix))
    }

    /// Downloads Workshop item `item` into `instance`, shown in the download manager like a
    /// game install: progress, speed, cancel.
    pub fn add_mod(
        &mut self,
        instance: Instance,
        item: u64,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        self.add_mods(instance, vec![item], None, Why::Add, None, window, cx);
    }

    /// Downloads the current version of Workshop items `items` that `instance` already has.
    pub fn update_mods(
        &mut self,
        instance: Instance,
        items: Vec<u64>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        // What they are at now is what "Undo" goes back to.
        let _ = instance::mods::remember(&instance, &items);
        self.add_mods(instance, items, None, Why::Update, None, window, cx);
    }

    /// Moves installed Workshop items to the versions in `exact` (item, manifest). `hold`
    /// keeps them there when updates come; picking the newest version lets them update again.
    pub fn mod_versions(
        &mut self,
        instance: Instance,
        exact: Vec<(u64, u64)>,
        hold: bool,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let items = exact.iter().map(|(id, _)| *id).collect();
        let wanted = crate::pack::Wanted {
            exact,
            active: None,
        };
        self.add_mods(
            instance,
            items,
            None,
            Why::Version { hold },
            Some(wanted),
            window,
            cx,
        );
    }

    /// Puts back the versions the last mod update replaced, all at once.
    pub fn undo_mods(
        &mut self,
        instance: Instance,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let exact = instance::mods::undo(&instance);
        if exact.is_empty() {
            return;
        }
        let items = exact.iter().map(|(id, _)| *id).collect();
        let wanted = crate::pack::Wanted {
            exact,
            active: None,
        };
        self.add_mods(instance, items, None, Why::Undo, Some(wanted), window, cx);
    }

    /// Downloads Workshop items `items` into `instance` as one entry of the download manager.
    /// `collection` names where they came from, for the note at the end; `why` says what the
    /// download is for. `pack` gives the versions to bring (a `.lata`'s, or the ones picked)
    /// and, for a pack, the mods it has on.
    #[expect(
        clippy::too_many_arguments,
        reason = "one call per kind of mod download"
    )]
    pub fn add_mods(
        &mut self,
        instance: Instance,
        items: Vec<u64>,
        collection: Option<SharedString>,
        why: Why,
        pack: Option<crate::pack::Wanted>,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt;
        let Some(first) = items.first() else { return };
        let key = mod_key(&instance.id, &first.to_string());
        if self.installs.contains_key(&key) {
            // Asked twice (a link clicked again, say): say so instead of doing nothing.
            let _ = window.update(cx, |_, window, cx| {
                window.push_notification(
                    crate::ui::toast::done("Easy, that one is already downloading"),
                    cx,
                );
            });
            return;
        }
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let (exact, active) = pack.map_or((Vec::new(), None), |p| (p.exact, p.active));
        let picked: Vec<u64> = exact.iter().map(|(id, _)| *id).collect();
        let task = crate::runtime::spawn(async move {
            let added = instance::mods::install(
                instance.clone(),
                items,
                exact,
                move |title, done, total| {
                    let _ = events_tx.send((SharedString::from(title.to_owned()), done, total));
                },
            )
            .await?;
            if let Some(active) = active {
                instance::mods::activate(&instance, &active).map_err(|err| err.to_string())?;
            }
            match why {
                // Only what really came at the version asked for stays there.
                Why::Version { hold } if added.newer.is_empty() => {
                    instance::mods::hold(&instance, &picked, hold).map_err(|e| e.to_string())?
                }
                Why::Undo => instance::mods::forget(&instance),
                _ => {}
            }
            Ok::<_, String>(added)
        });
        let name = match (&collection, why) {
            (Some(name), _) => name.clone(),
            (None, Why::Update) => "Mod updates".into(),
            (None, Why::Version { .. }) => "Mod version".into(),
            (None, Why::Undo) => "Undoing the update".into(),
            (None, Why::Add) => "Workshop mod".into(),
        };
        let download = install::Download::new(name, install::Kind::Mods, Some(task.abort_handle()));
        self.installs.insert(key.clone(), download);
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            // The sender lives in the task: this loop ends when the task does.
            while let Some((title, done, total)) = events.recv().await {
                let _ = this.update(cx, |shell, cx| {
                    let Some(download) = shell.installs.get_mut(&key) else {
                        return;
                    };
                    let renamed = download.name != title;
                    download.name = title;
                    if download.progress(done, total, std::time::Instant::now()) || renamed {
                        shell.changed(cx);
                    }
                });
            }
            let added = task.await;
            let _ = this.update(cx, |shell, cx| {
                shell.installs.remove(&key);
                shell.changed(cx);
            });
            let note = match added {
                Ok(Ok(added)) if why == Why::Update => {
                    crate::ui::toast::done(match added.with.len() {
                        0 => format!("{} is up to date", added.title),
                        many => format!("Fresh! {} mods are up to date", many + 1),
                    })
                }
                Ok(Err(reason)) if why == Why::Update => {
                    crate::ui::toast::failed_because("That update didn't make it", reason)
                }
                Ok(Ok(added)) if why != Why::Add && !added.newer.is_empty() => {
                    crate::ui::toast::heads_up(
                        "Steam no longer has that version",
                        format!("Still on the newest: {}", added.newer.join(", ")),
                    )
                }
                Ok(Ok(added)) if matches!(why, Why::Version { .. }) => crate::ui::toast::done(
                    format!("Done! {} is on the version you picked", added.title),
                ),
                Ok(Ok(_)) if why == Why::Undo => {
                    crate::ui::toast::done("Rewound! Your mods are back to before the update")
                }
                Ok(Err(reason)) if why != Why::Add => {
                    crate::ui::toast::failed_because("That version didn't make it", reason)
                }
                Ok(Ok(added)) if !added.newer.is_empty() => crate::ui::toast::heads_up(
                    "Some mods came at a newer version",
                    format!(
                        "Steam no longer has the version the pack used: {}",
                        added.newer.join(", ")
                    ),
                ),
                Ok(Ok(added)) => crate::ui::toast::done(match (&collection, added.with.len()) {
                    (Some(name), many) => {
                        format!("{name} is in! All {} mods are ready to go", many + 1)
                    }
                    (None, 0) => format!("{} is in! Ready for your next run", added.title),
                    (None, 1) => format!("{} is in, with the mod it needs", added.title),
                    (None, many) => {
                        format!("{} is in, with the {many} mods it needs", added.title)
                    }
                }),
                Ok(Err(reason)) => {
                    crate::ui::toast::failed_because("That mod didn't make it", reason)
                }
                // Cancelled from the download manager: nothing to say.
                Err(_) => return,
            };
            let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
        })
        .detach();
    }

    pub fn downloads(&self) -> Vec<(&SharedString, &install::Download)> {
        let mut all: Vec<_> = self.installs.iter().collect();
        all.sort_by(|a, b| a.1.name.cmp(&b.1.name));
        all
    }

    /// Stops an install; what it wrote stays for the next one to reuse.
    pub fn cancel(&mut self, id: &SharedString, cx: &mut Context<Self>) {
        if let Some(download) = self.installs.remove(id) {
            download.cancel();
            self.changed(cx);
        }
    }

    /// Shows the page of instance `id`.
    pub fn view(shell: WeakEntity<Shell>, id: SharedString, _: &mut Window, cx: &mut App) {
        let _ = shell.update(cx, |shell, cx| {
            if let Some(ix) = shell.instances.iter().position(|i| i.id == id) {
                shell.go(Page::Instance(ix), cx);
            }
        });
    }

    /// Opens the settings dialog of instance `id`.
    pub fn settings(shell: WeakEntity<Shell>, id: SharedString, window: &mut Window, cx: &mut App) {
        Self::settings_at(shell, id, instance::settings::Section::General, window, cx);
    }

    /// Opens the settings dialog of instance `id` on its picture.
    pub fn customize(
        shell: WeakEntity<Shell>,
        id: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) {
        Self::settings_at(
            shell,
            id,
            instance::settings::Section::Customize,
            window,
            cx,
        );
    }

    fn settings_at(
        shell: WeakEntity<Shell>,
        id: SharedString,
        section: instance::settings::Section,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Ok(Some(instance)) = shell.read_with(cx, |shell, _| {
            shell.instances.iter().find(|i| i.id == id).cloned()
        }) else {
            return;
        };
        instance::settings::open(instance, section, shell, window, cx);
    }

    /// Asks before deleting instance `id`: its saves and mods go with it.
    pub fn confirm_delete(
        shell: WeakEntity<Shell>,
        id: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) {
        use gpui_kit::component::WindowExt;
        let Ok(Some(name)) = shell.read_with(cx, |shell, _| {
            let instance = shell.instances.iter().find(|i| i.id == id);
            instance.map(|i| i.name.clone())
        }) else {
            return;
        };
        window.open_dialog(cx, move |dialog, _, _| {
            // Fades in settling from just above, and back out (the default drops from the window top).
            let dialog = dialog.entrance(DialogEntrance::FadeSlide);
            let (shell, id) = (shell.clone(), id.clone());
            dialog
                .title(format!("Delete {name}?"))
                .w(px(420.))
                .child("Its saves, mods and settings are deleted for good.")
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .pt_4()
                        .child(
                            crate::ui::secondary("cancel-delete", "Keep")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            crate::ui::FancyButton::new("confirm-delete", "Delete")
                                .small()
                                .danger()
                                .icon(gpui_kit::assets::IconName::Trash)
                                .on_click(move |_, window, cx| {
                                    // The instance's settings dialog may be open under this one.
                                    window.close_all_dialogs(cx);
                                    if let Ok(Err(why)) =
                                        shell.update(cx, |shell, cx| shell.remove(&id, window, cx))
                                    {
                                        window.push_notification(crate::ui::toast::failed(why), cx);
                                    }
                                }),
                        ),
                )
        });
    }

    /// Deletes instance `id` with its saves and mods, and goes back to the Library. Refused
    /// while the game runs or an install writes into it. The instance leaves the list at once;
    /// its files (gigabytes, with its mods) go in the background, shown in the download
    /// manager with the space freed.
    pub fn remove(
        &mut self,
        id: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        use gpui_kit::component::WindowExt;
        if self.running.contains_key(id) || self.installs.contains_key(id) || self.adding_mods(id) {
            return Err("Whoa! Stop the game before deleting it".into());
        }
        let Some(ix) = self.instances.iter().position(|i| i.id == *id) else {
            return Ok(());
        };
        let instance = self.instances.remove(ix);
        self.page = Page::Library;
        self.changed(cx);
        let name = instance.name.clone();
        let key: SharedString = format!("delete:{id}").into();
        self.installs.insert(
            key.clone(),
            install::Download::new(name.clone(), install::Kind::Delete, None),
        );
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let (root, handle) = (self.root.clone(), window.window_handle());
        // Spawned now: `blocking` only starts when awaited, and the loop below waits on the
        // progress it sends.
        let deleting = crate::runtime::spawn(crate::runtime::blocking(move || {
            store::delete(&root, &instance.dir, |freed, total| {
                let _ = events_tx.send((freed, total));
            })
            .map_err(|err| err.to_string())
        }));
        cx.spawn(async move |this, cx| {
            // The sender lives in the deletion: this loop ends when it does.
            while let Some((freed, total)) = events.recv().await {
                let _ = this.update(cx, |shell, cx| {
                    if let Some(task) = shell.installs.get_mut(&key)
                        && task.progress(freed, total, std::time::Instant::now())
                    {
                        shell.changed(cx);
                    }
                });
            }
            let deleted = deleting
                .await
                .map_err(|err| err.to_string())
                .and_then(|result| result)
                .and_then(|result| result);
            // Whatever happened, show what is on disk now.
            let _ = this.update(cx, |shell, cx| {
                shell.installs.remove(&key);
                shell.reload(cx);
            });
            let toast = match deleted {
                Ok(()) => crate::ui::toast::done(format!("Poof! {name} is gone")),
                Err(err) => crate::ui::toast::failed_because("That instance wouldn't budge", err),
            };
            let _ = handle.update(cx, |_, window, cx| window.push_notification(toast, cx));
        })
        .detach();
        Ok(())
    }

    /// Copies instance `id` into a new one and opens it. The copy runs in the background, shown
    /// in the download manager; nothing is listed until it is whole. Refused while the game
    /// runs or an install writes into it, since the files would change under the copy.
    pub fn duplicate(
        shell: WeakEntity<Shell>,
        id: SharedString,
        window: &mut Window,
        cx: &mut App,
    ) {
        use crate::ui::toast;
        use gpui_kit::component::WindowExt;
        let handle = window.window_handle();
        let started = shell.update(cx, |shell, cx| {
            if shell.running.contains_key(&id)
                || shell.installs.contains_key(&id)
                || shell.adding_mods(&id)
            {
                return Err("Whoa! Stop the game before duplicating it".to_owned());
            }
            let Some(instance) = shell.instances.iter().find(|i| i.id == id).cloned() else {
                return Ok(());
            };
            let key: SharedString = format!("copy:{id}").into();
            if shell.installs.contains_key(&key) {
                return Ok(());
            }
            shell.installs.insert(
                key.clone(),
                install::Download::new(instance.name.clone(), install::Kind::Copy, None),
            );
            shell.changed(cx);
            let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
            let root = shell.root.clone();
            let copying = crate::runtime::spawn(crate::runtime::blocking(move || {
                store::duplicate(&root, &instance.dir, |done, total| {
                    let _ = events_tx.send((done, total));
                })
                .map_err(|err| err.to_string())
            }));
            cx.spawn(async move |this, cx| {
                while let Some((done, total)) = events.recv().await {
                    let _ = this.update(cx, |shell, cx| {
                        if let Some(task) = shell.installs.get_mut(&key)
                            && task.progress(done, total, std::time::Instant::now())
                        {
                            shell.changed(cx);
                        }
                    });
                }
                let made = copying
                    .await
                    .map_err(|err| err.to_string())
                    .and_then(|result| result)
                    .and_then(|result| result);
                let note = match &made {
                    Ok(copy) => toast::done(format!("{} is ready", copy.name)),
                    Err(why) => toast::failed_because("That copy didn't work out", why.clone()),
                };
                let _ = this.update(cx, |shell, cx| {
                    shell.installs.remove(&key);
                    shell.reload(cx);
                    if let Ok(copy) = &made
                        && let Some(ix) = shell.instances.iter().position(|i| i.id == copy.id)
                    {
                        shell.go(Page::Instance(ix), cx);
                    }
                });
                let _ = handle.update(cx, |_, window, cx| window.push_notification(note, cx));
            })
            .detach();
            Ok(())
        });
        if let Ok(Err(why)) = started {
            window.push_notification(toast::failed(why), cx);
        }
    }

    /// Shows or hides the loader under the title bar.
    pub fn set_loading(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.loading != on {
            self.loading = on;
            cx.notify();
        }
    }

    /// Whether a game runs or anything downloads, deletes or copies: the library can't move.
    pub fn busy(&self) -> bool {
        !self.running.is_empty() || !self.installs.is_empty()
    }

    /// The library moved: instances are read from its new place.
    pub fn relocated(&mut self, cx: &mut Context<Self>) {
        self.root = store::root();
        self.reload(cx);
    }

    /// Re-reads instances from disk, keeping the current page.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.instances = store::list(&self.root);
        if let (Page::Instance(_), Some(detail)) = (self.page, &self.detail) {
            let shown = detail.read(cx).id();
            if let Some(ix) = self.instances.iter().position(|i| i.id == shown) {
                return self.go(Page::Instance(ix), cx);
            }
        }
        self.changed(cx);
    }

    /// Repaints the shell and the detail view (a child entity doesn't redraw on its own).
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(detail) = &self.detail {
            detail.update(cx, |detail, cx| detail.changed(cx));
        }
        if let Some(library) = &self.library {
            library.update(cx, |_, cx| cx.notify());
        }
        cx.notify();
    }

    pub fn account(&self) -> Option<&Account> {
        self.account.as_ref()
    }

    /// A `lata://` link from outside, such as the browser extension's. It brings the window to
    /// the front right away, with the loader on while Steam is asked about the mod (the click
    /// happened in another program, so lata has to show it was heard). Then it asks which
    /// instance the mod goes into, or says why the link can't be used. A link that arrives
    /// while that dialog is open waits its turn.
    pub fn receive_link(&mut self, text: String, window: AnyWindowHandle, cx: &mut Context<Self>) {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        self.link_resolving += 1;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let found = crate::link::resolve(&text).await;
            let _ = this.update(cx, |shell, cx| {
                shell.link_resolving -= 1;
                cx.notify();
            });
            match found {
                Ok(found) => {
                    let _ = this.update(cx, |shell, cx| shell.offer(found, window, cx));
                }
                Err(why) => {
                    let _ = window.update(cx, |_, window, cx| {
                        use gpui_kit::component::WindowExt;
                        window.push_notification(
                            crate::ui::toast::failed_because("Couldn't use that link", why),
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    /// Opens the install dialog for `found`, or queues it behind the one that is open. The same
    /// mod twice is offered once.
    fn offer(
        &mut self,
        found: crate::link::Found,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        let id = found.id();
        if self.link_open == Some(id) || self.link_queue.iter().any(|queued| queued.id() == id) {
            return;
        }
        if self.link_open.is_some() {
            // A page allowed to open lata could send links without end; past a handful the
            // rest are dropped rather than piled up as dialogs.
            if self.link_queue.len() < LINK_QUEUE {
                self.link_queue.push_back(found);
            }
            return;
        }
        let (instances, shell) = (self.instances.clone(), cx.entity().downgrade());
        let opened = window.update(cx, |_, window, cx| {
            crate::link::install::open(found, instances, shell, window, cx)
        });
        // Only a dialog that did open closes later and frees the turn for the next link.
        if opened.is_ok() {
            self.link_open = Some(id);
        }
    }

    /// Keeps a linked mod for when there is an instance to put it in: the install dialog found
    /// none and the user went to create one.
    // ponytail: waits for the next instance created in this run, even if this create dialog is
    // cancelled; clear it when the dialog closes if that ever surprises someone.
    pub fn link_wait(&mut self, found: crate::instance::mods::Listing) {
        self.link_waiting = Some(found);
    }

    /// The install dialog closed, installed or not: the next waiting link gets its turn.
    pub fn link_closed(&mut self, window: AnyWindowHandle, cx: &mut Context<Self>) {
        self.link_open = None;
        if let Some(next) = self.link_queue.pop_front() {
            self.offer(next, window, cx);
        }
    }

    /// The sign-in dialog, unless an account is already signed in.
    pub fn open_account(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
        if shell
            .read_with(cx, |shell, _| shell.account.is_some())
            .unwrap_or(true)
        {
            return;
        }
        account::login::open(
            move |signed_in, _, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.account = Some(signed_in);
                    cx.notify();
                });
            },
            window,
            cx,
        )
    }

    /// Forgets the saved login.
    pub fn sign_out(shell: WeakEntity<Shell>, cx: &mut App) {
        cx.spawn(async move |cx| {
            let _ = account::forget().await;
            let _ = shell.update(cx, |shell, cx| {
                shell.account = None;
                cx.notify();
            });
        })
        .detach();
    }

    /// Offers the game someone already plays, once: on the first start with no instances yet,
    /// when the game's own folder has worlds or mods to bring. Declined, it stays reachable
    /// from the create dialog.
    pub fn welcome(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
        use instance::existing;
        let fresh = shell
            .read_with(cx, |shell, _| shell.instances.is_empty())
            .unwrap_or(false);
        let saved = crate::settings::load();
        // Where instances go is asked first; it comes back here once picked.
        if fresh && saved.library.is_none() {
            return crate::storage::picker::first_start(shell, window, cx);
        }
        if !fresh || saved.existing_offered {
            return;
        }
        let Some(game) = existing::home().and_then(|dir| existing::find(&dir)) else {
            return;
        };
        crate::settings::update(|saved| saved.existing_offered = true);
        instance::existing_view::open(game, shell, window, cx);
    }

    /// Makes an instance for each of `plans` out of `game` and starts bringing its things in.
    /// The first one is shown and its game install starts, as for any new instance.
    pub fn adopt(
        shell: WeakEntity<Shell>,
        game: instance::existing::Existing,
        plans: Vec<instance::existing::Plan>,
        window: &mut Window,
        cx: &mut App,
    ) {
        use gpui_kit::component::WindowExt;
        let handle = window.window_handle();
        let mut made = Vec::new();
        let result = shell.update(cx, |shell, _| {
            for plan in plans {
                // The branch names Steam gives the two build lines; an instance can move to
                // another branch of its line later.
                let (name, branch) = if plan.modern {
                    ("My game", "public")
                } else {
                    ("My Build 41 game", "legacy41")
                };
                made.push((plan, store::create(&shell.root, name, branch)?));
            }
            std::io::Result::Ok(())
        });
        let first = made.first().map(|(_, created)| created.id.clone());
        let _ = shell.update(cx, |shell, cx| {
            shell.instances = store::list(&shell.root);
            let ix = first
                .as_ref()
                .and_then(|id| shell.instances.iter().position(|i| i.id == *id));
            shell.go(ix.map_or(Page::Library, Page::Instance), cx);
        });
        // The game install first: once bringing starts, the main button is how it's cancelled.
        if let Some(id) = first {
            Shell::play(shell.clone(), id, window, cx);
        }
        let _ = shell.update(cx, |shell, cx| {
            for (plan, created) in made {
                shell.bring(game.clone(), plan, created, handle, cx);
            }
        });
        if let Ok(Err(err)) = result {
            window.push_notification(
                crate::ui::toast::failed_because("Well, that instance didn't happen", err),
                cx,
            );
        }
    }

    /// Copies what `plan` takes of `game` into `instance`, then downloads the Workshop mods
    /// its worlds load, all as one entry of the download manager.
    fn bring(
        &mut self,
        game: instance::existing::Existing,
        plan: instance::existing::Plan,
        instance: Instance,
        window: AnyWindowHandle,
        cx: &mut Context<Self>,
    ) {
        use gpui_kit::component::WindowExt;
        // A mod key, so the instance counts as getting mods: it can't be deleted meanwhile.
        let key = mod_key(&instance.id, "existing");
        let name = instance.name.clone();
        let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
        let task = crate::runtime::spawn({
            let name = name.clone();
            async move {
                let copying = (events_tx.clone(), name);
                let (from, plan_, into) = (game, plan.clone(), instance.clone());
                // Aborting the task drops `alive`; the copy runs on its own thread and stops
                // at the next file once it sees that.
                let alive = std::sync::Arc::new(());
                let watch = std::sync::Arc::downgrade(&alive);
                let sources = crate::runtime::blocking(move || {
                    from.copy(&plan_, &into, |done, total| {
                        if watch.strong_count() == 0 {
                            return Err(std::io::Error::other("cancelled"));
                        }
                        let _ = copying.0.send((copying.1.clone(), done, total));
                        Ok(())
                    })
                    .map_err(|err| err.to_string())?;
                    Ok::<_, String>(from.sources(&plan_, &instance::import::workshop()))
                })
                .await??;
                drop(alive);
                // The worlds are in by now: mods that fail leave an instance worth keeping.
                let mods = if sources.workshop.is_empty() {
                    Ok(())
                } else {
                    let items = sources.workshop.clone();
                    instance::mods::install(
                        instance.clone(),
                        items,
                        Vec::new(),
                        move |title, done, total| {
                            let _ = events_tx.send((title.to_owned().into(), done, total));
                        },
                    )
                    .await
                    // Installing turns on every mod an item carries; back to what was played.
                    .and_then(|_| {
                        instance::mods::activate(&instance, &plan.active)
                            .map_err(|err| err.to_string())
                    })
                };
                Ok::<_, String>((sources.missing, mods))
            }
        });
        self.installs.insert(
            key.clone(),
            install::Download::new(
                name.clone(),
                install::Kind::Existing,
                Some(task.abort_handle()),
            ),
        );
        self.changed(cx);
        cx.spawn(async move |this, cx| {
            // The senders live in the task: this loop ends when the task does.
            while let Some((title, done, total)) = events.recv().await {
                let _ = this.update(cx, |shell, cx| {
                    let Some(download) = shell.installs.get_mut(&key) else {
                        return;
                    };
                    let renamed = download.name != title;
                    download.name = title;
                    if download.progress(done, total, std::time::Instant::now()) || renamed {
                        shell.changed(cx);
                    }
                });
            }
            let brought = task.await;
            let _ = this.update(cx, |shell, cx| {
                shell.installs.remove(&key);
                shell.reload(cx);
            });
            use crate::ui::toast;
            let note = match brought {
                Ok(Ok((missing, Ok(())))) if missing.is_empty() => {
                    toast::done(format!("{name} is in! Worlds, mods and settings made it"))
                }
                Ok(Ok((missing, Ok(())))) => toast::heads_up(
                    "Almost everything made it",
                    format!(
                        "Steam doesn't have these mods here, add them from the Workshop: {}",
                        missing.join(", ")
                    ),
                ),
                Ok(Ok((_, Err(why)))) => toast::heads_up("Your mods got stuck on the way", why),
                Ok(Err(why)) => toast::failed_because("Couldn't bring your game over", why),
                // Cancelled from the download manager: nothing to say.
                Err(_) => return,
            };
            let _ = window.update(cx, |_, window, cx| window.push_notification(note, cx));
        })
        .detach();
    }

    /// Opens the create dialog; a created instance is listed, selected and installed.
    pub fn create(shell: WeakEntity<Shell>, window: &mut Window, cx: &mut App) {
        Self::create_from(shell, None, window, cx);
    }

    /// Makes an instance from the `.lata` at `path`, then installs its game and its mods.
    pub fn import_pack(shell: WeakEntity<Shell>, path: PathBuf, window: &mut Window, cx: &mut App) {
        use crate::ui::toast;
        use gpui_kit::component::WindowExt;
        let Ok(root) = shell.read_with(cx, |shell, _| shell.root.clone()) else {
            return;
        };
        let handle = window.window_handle();
        cx.spawn(async move |cx| {
            let made = crate::runtime::blocking(move || crate::pack::import(&root, &path)).await;
            let (instance, index) = match made {
                Ok(Ok(made)) => made,
                Ok(Err(why)) => {
                    let _ = handle.update(cx, |_, window, cx| {
                        window.push_notification(
                            toast::failed_because("That pack wouldn't open", why),
                            cx,
                        )
                    });
                    return;
                }
                Err(_) => return,
            };
            // Steam only serves each branch's current build; an older pack gets that one.
            let current = branches_build(&index.build.branch).await;
            let older = index
                .build
                .id
                .zip(current)
                .is_some_and(|(made, now)| made != now);
            let _ = handle.update(cx, |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.instances = store::list(&shell.root);
                    let ix = shell.instances.iter().position(|i| i.id == instance.id);
                    shell.go(ix.map_or(Page::Library, Page::Instance), cx);
                    let (items, wanted) = index.wanted();
                    if !items.is_empty() {
                        let name = SharedString::from(instance.name.to_string());
                        shell.add_mods(
                            instance.clone(),
                            items,
                            Some(name),
                            Why::Add,
                            Some(wanted),
                            handle,
                            cx,
                        );
                    }
                });
                Shell::play(shell.clone(), instance.id.clone(), window, cx);
                if older {
                    window.push_notification(
                        toast::heads_up(
                            "This pack was made on an older build",
                            format!(
                                "Steam only has the current one of {}, so that's the one coming",
                                index.build.branch
                            ),
                        ),
                        cx,
                    );
                }
            });
        })
        .detach();
    }

    /// The create dialog, already on `collection` when a link brought one.
    pub fn create_from(
        shell: WeakEntity<Shell>,
        collection: Option<instance::mods::Bundle>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Ok(root) = shell.read_with(cx, |shell, _| shell.root.clone()) else {
            return;
        };
        let (existing, pack) = (shell.clone(), shell.clone());
        instance::create::open(
            root,
            collection,
            move |game, window, cx| {
                instance::existing_view::open(game, existing.clone(), window, cx)
            },
            move |path, window, cx| Shell::import_pack(pack.clone(), path, window, cx),
            move |created, mods, window, cx| {
                let handle = window.window_handle();
                let _ = shell.update(cx, |shell, cx| {
                    shell.instances = store::list(&shell.root);
                    let ix = shell.instances.iter().position(|i| i.id == created.id);
                    shell.go(ix.map_or(Page::Library, Page::Instance), cx);
                    // Mods download anonymously, alongside the game.
                    if let Some((collection, items)) = mods {
                        shell.add_mods(
                            created.clone(),
                            items,
                            Some(collection),
                            Why::Add,
                            None,
                            handle,
                            cx,
                        );
                    }
                });
                // A new instance has nothing to run yet: start its install right away, as its
                // main button would (it asks to sign in to Steam first when nobody is).
                Shell::play(shell.clone(), created.id.clone(), window, cx);
                // The mod a link brought before there was any instance gets its dialog now.
                // Deferred: the dialog opens through the window's handle, which can't be used
                // from inside this callback, where the window is already borrowed.
                let (shell, handle) = (shell.clone(), window.window_handle());
                cx.defer(move |cx| {
                    let _ = shell.update(cx, |shell, cx| {
                        if let Some(found) = shell.link_waiting.take() {
                            shell.offer(crate::link::Found::Mod(found), handle, cx);
                        }
                    });
                });
            },
            window,
            cx,
        );
    }
}

/// The build Steam serves now on `branch`; `None` when Steam can't be asked.
async fn branches_build(branch: &str) -> Option<u32> {
    let list = instance::branches::fetch().await.ok()?;
    list.into_iter()
        .find(|b| b.name == branch)
        .map(|b| b.build_id)
}

/// How many links wait their turn behind the open install dialog.
const LINK_QUEUE: usize = 8;

/// Shows a toast from work that has no window of its own (a background check): on the
/// active window, or the only one.
fn notify(cx: &mut App, toast: impl FnOnce() -> crate::ui::toast::Notification) {
    let Some(window) = cx.active_window().or_else(|| cx.windows().first().copied()) else {
        return;
    };
    let _ = window.update(cx, |_, window, cx| {
        use gpui_kit::component::WindowExt;
        window.push_notification(toast(), cx);
    });
}

/// The community's server: the title bar and Settings both send people there.
pub const DISCORD_INVITE: &str = "https://discord.gg/jeGPaMkDbU";

/// What a Workshop download is for, which decides what happens around it and the note it ends
/// with.
#[derive(Clone, Copy, PartialEq)]
pub enum Why {
    /// New items for the instance.
    Add,
    /// Installed items, brought to their current version.
    Update,
    /// Installed items, moved to a version the user picked; `hold` keeps them there.
    Version { hold: bool },
    /// The versions the last update replaced, put back.
    Undo,
}

/// The download manager's key for Workshop item `item` going into instance `instance`.
fn mod_key(instance: &SharedString, item: &str) -> SharedString {
    format!("mod:{instance}:{item}").into()
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let page = match (self.page, &self.detail) {
            (Page::Instance(_), Some(detail)) => detail.clone().into_any_element(),
            (Page::Screenshots, _) => {
                let instances = self.instances.clone();
                self.screenshots
                    .get_or_insert_with(|| {
                        cx.new(|cx| screenshots::Screenshots::new(instances, cx))
                    })
                    .clone()
                    .into_any_element()
            }
            _ => {
                let shell = cx.entity().downgrade();
                self.library
                    .get_or_insert_with(|| cx.new(|cx| Library::new(shell, window, cx)))
                    .clone()
                    .into_any_element()
            }
        };

        crate::ui::smooth_scroll("sidebar-scroll", &self.sidebar_scroll, window, cx);
        // The panel on the right needs a window wide enough for it and the page.
        let discover = window.bounds().size.width >= crate::discover::MIN_WINDOW;
        // Mods downloading into the instance on screen load under the title bar, where the
        // Library's own loading shows, instead of in the middle of the Mods tab.
        let adding = match self.page {
            Page::Instance(ix) => self
                .instances
                .get(ix)
                .is_some_and(|instance| self.adding_mods(&instance.id)),
            _ => false,
        };
        let shell = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(cx.theme().title_bar)
            // A `.lata` dropped anywhere on the window is imported.
            .on_drop(move |dropped: &ExternalPaths, window, cx| {
                let packs = dropped.paths().iter().filter(|path| {
                    path.extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case(crate::pack::EXT))
                });
                for path in packs {
                    Shell::import_pack(shell.clone(), path.clone(), window, cx);
                }
            })
            .text_color(cx.theme().foreground)
            .child(Topbar {
                running: self.running(),
                update: self.update_button(),
                shell: cx.entity().downgrade(),
                downloads: downloads::Pill::new(self, cx.entity().downgrade()),
            })
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(sidebar::render(self, window, cx))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .rounded_tl(px(14.))
                            .border_t_1()
                            .border_l_1()
                            // With the panel beside it the page is an island with both top
                            // corners round; without, it runs to the window's edge.
                            .when(discover, |this| this.rounded_tr(px(14.)).border_r_1())
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .overflow_hidden()
                            .relative()
                            .child(div().flex_1().min_h_0().child(page))
                            // Over the page instead of above it, so nothing shifts when it
                            // shows; inset by the corner radius, since GPUI clips to a plain
                            // rectangle and a full-width bar cut straight through the round
                            // corners.
                            .when(self.loading || self.link_resolving > 0 || adding, |this| {
                                this.child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(px(14.))
                                        .right(px(14.))
                                        .child(Loader),
                                )
                            }),
                    )
                    .when(discover, |this| {
                        this.child(crate::discover::render(self, self.feed.as_ref(), cx))
                    }),
            )
    }
}
