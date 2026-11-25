//! lata updating itself from its GitHub releases, the way Discord does: before the main
//! window a small one checks for a newer release, downloads this system's package, checks it
//! against the release's checksums, tries it, swaps it in and starts it. A lata that is
//! already open only says a newer one is out; restarting goes through that same window.
//! Where something else owns the install (a package manager, a Flatpak) nothing is replaced.

#[path = "can.view.rs"]
mod can;
#[path = "release.model.rs"]
mod release;
#[path = "splash.view.rs"]
mod splash;
#[path = "swap.model.rs"]
mod swap;

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gpui_kit::{App, AsyncApp, Entity};
use sha2::{Digest, Sha256};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

pub use release::{Release, Version};
use splash::{Splash, Step};

use crate::link::Incoming;
use crate::runtime;

/// How often a lata left open asks again.
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long the main window waits for GitHub's answer at startup. Offline, or GitHub slow,
/// must not keep anyone from their game; the check in the open app catches up later.
const CHECK: Duration = Duration::from_secs(3);
/// How long the open app waits for that answer, so a check that hangs can't stop the ones
/// after it.
const ASK: Duration = Duration::from_secs(30);
/// A download that sends nothing for this long is given up.
const STALL: Duration = Duration::from_secs(20);
/// And so is one that isn't done by now, however steadily it trickles.
const DOWNLOAD: Duration = Duration::from_secs(10 * 60);
/// How long a failure stays on the splash before the main window opens.
const FAILED: Duration = Duration::from_secs(3);
/// No package is anywhere near this (they are about 30 MB); a release claiming more is wrong.
const MAX_BYTES: u64 = 512 * 1024 * 1024;
/// `SHA256SUMS` is a few lines.
const SUMS_BYTES: u64 = 64 * 1024;
/// How many pending links go to the new lata as arguments; the main window queues no more
/// than this either, and a command line has a length limit.
const LINKS: usize = 8;
/// In a debug build, makes lata say it is another version (`LATA_UPDATE_AS=0.5.0-dev`) and
/// turns the checks on, to try the notice. Release builds don't read it: nothing in the
/// environment should be able to make an installed lata update.
const AS: &str = "LATA_UPDATE_AS";
/// How "Restart to update" starts lata: the one before it is still closing, so this one
/// must not hand over to it and leave.
pub const RESTARTED: &str = "--restarted";
/// How a lata that just replaced itself starts the new one: like [`RESTARTED`], and it
/// doesn't look for an update again, so nothing can make it update in a loop.
pub const UPDATED: &str = "--updated";

/// The version this lata is.
pub fn current() -> Version {
    cfg!(debug_assertions)
        .then(|| std::env::var(AS).ok())
        .flatten()
        .and_then(|text| Version::parse(&text))
        .or_else(|| Version::parse(env!("CARGO_PKG_VERSION")))
        .expect("the crate's version is a semantic version")
}

/// Whether lata looks for updates: the setting, not under a store that updates it, and never
/// from a debug build unless asked (a rebuild on every save would run into GitHub's limit of
/// 60 requests an hour).
pub fn enabled() -> bool {
    !crate::settings::load().updates_off
        && !swap::in_store()
        && (!cfg!(debug_assertions) || std::env::var_os(AS).is_some())
}

/// A newer release, for the lata that is already open.
pub struct Found {
    pub release: Release,
    /// Whether a restart would install it; otherwise all lata can do is link its page.
    pub installable: bool,
}

/// Asks for a release newer than this lata.
pub async fn check() -> Result<Option<Found>, String> {
    let found =
        runtime::spawn(async { tokio::time::timeout(ASK, release::check(&current())).await })
            .await
            .map_err(|err| err.to_string())?
            .map_err(|_| t!("GitHub took too long"))??;
    let Some(release) = found else {
        return Ok(None);
    };
    // Touches the disk (it tries to write beside lata), so not on the UI thread.
    let installable =
        release.package.is_some() && runtime::blocking(swap::replaceable).await?.is_some();
    Ok(Some(Found {
        release,
        installable,
    }))
}

/// Starts lata again through the update window; the caller then quits.
pub fn restart() -> Result<(), String> {
    let target = swap::installed().ok_or(t!("lata can't tell where it is installed"))?;
    swap::launch(&target, &[RESTARTED.to_owned()]).map_err(|err| err.to_string())
}

/// Once the main window is up: clears what an update left behind. The lata that was
/// replaced is kept until here, so one that can't even open its window leaves the old one
/// beside it to put back.
pub fn tidy() {
    // A debug build never updates, and must not touch what sits beside `target/`.
    if !cfg!(debug_assertions)
        && let Some(target) = swap::installed()
    {
        swap::tidy(&target);
    }
}

/// What lata does first: looks for an update behind the splash window and installs it, or
/// calls `main` to open the main window, with what arrived from outside meanwhile.
///
/// `incoming` holds the links lata was started with and those other latas hand over while
/// the splash shows. They go to `main`, or, when lata replaces itself, to the new lata as
/// arguments, so a link clicked in the browser survives the update. `outside` is the other
/// end of it, to put links back if the new lata doesn't start.
pub fn start(
    args: &[String],
    outside: UnboundedSender<Incoming>,
    incoming: UnboundedReceiver<Incoming>,
    main: impl FnOnce(UnboundedReceiver<Incoming>, &mut App) + 'static,
    cx: &mut App,
) {
    if let Some(step) = splash::pinned() {
        let _ = splash::open(step, cx);
        return;
    }
    let just_updated = args.iter().any(|arg| arg == UPDATED);
    // No window at all where lata won't install anything: the open app says what is new.
    let Some(target) = (!just_updated && enabled())
        .then(swap::replaceable)
        .flatten()
    else {
        return main(incoming, cx);
    };
    let Ok((window, view)) = splash::open(Step::Checking, cx) else {
        return main(incoming, cx);
    };
    cx.spawn(async move |cx| {
        let mut incoming = incoming;
        let outcome = match update(&view, target.clone(), cx).await {
            Ok(true) => {
                show(&view, Step::Launching, cx);
                let mut links = Vec::new();
                while let Ok(message) = incoming.try_recv() {
                    if let Incoming::Link(link) = message {
                        links.push(link);
                    }
                }
                let mut args = vec![UPDATED.to_owned()];
                args.extend(links.iter().take(LINKS).cloned());
                match swap::launch(&target, &args) {
                    Ok(()) => return cx.update(|cx| cx.quit()),
                    Err(err) => {
                        // This lata goes on, so the links are its to open again.
                        for link in links {
                            let _ = outside.send(Incoming::Link(link));
                        }
                        Err(t!(
                            "The new version is in place but didn't open ({err}). Close lata and open it again",
                            err = err
                        ))
                    }
                }
            }
            Ok(false) => Ok(()),
            Err(why) => Err(why),
        };
        match outcome {
            Ok(()) => show(&view, Step::Launching, cx),
            Err(why) => {
                show(&view, Step::Failed(why.into()), cx);
                cx.background_executor().timer(FAILED).await;
            }
        }
        cx.update(|cx| {
            // The main window first: an app with no window left might take that for the end.
            main(incoming, cx);
            let _ = window.update(cx, |_, window, _| window.remove_window());
        });
    })
    .detach();
}

/// Moves the splash to `step`. Skipping is only offered while downloading, so leaving that
/// step takes the button away.
fn show(view: &Entity<Splash>, step: Step, cx: &mut AsyncApp) {
    view.update(cx, |splash, cx| {
        if !matches!(step, Step::Downloading { .. }) {
            splash.on_skip = None;
        }
        splash.step = step;
        cx.notify();
    });
}

/// The splash's work. `Ok(true)`: a newer lata is in place. `Ok(false)`: nothing to do (no
/// update, no answer in time, a release this lata can't install by itself, or the download
/// was skipped). `Err`: an update was found and couldn't be put in; the lata that runs is
/// untouched.
async fn update(view: &Entity<Splash>, target: PathBuf, cx: &mut AsyncApp) -> Result<bool, String> {
    let asked =
        runtime::spawn(async { tokio::time::timeout(CHECK, release::check(&current())).await });
    let Ok(Ok(Ok(Some(release)))) = asked.await else {
        return Ok(false);
    };
    let Some((_, total)) = release.package else {
        return Ok(false);
    };
    show(view, Step::Downloading { done: 0, total }, cx);
    // Skipping and installing race for this: whoever sets it first wins, so a skip either
    // stops the download or comes too late to matter, and never lands in the middle of a
    // swap.
    let decided = Arc::new(AtomicBool::new(false));
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let work = {
        let decided = decided.clone();
        runtime::spawn(async move { install(&release, target, &events_tx, &decided).await })
    };
    let stop = work.abort_handle();
    view.update(cx, |splash, cx| {
        splash.on_skip = Some(Rc::new(move || {
            if !decided.swap(true, Ordering::SeqCst) {
                stop.abort();
            }
        }));
        cx.notify();
    });
    // Ends when the work does: it holds the only sender.
    while let Some(step) = events.recv().await {
        show(view, step, cx);
    }
    match work.await {
        Ok(done) => done.map(|()| true),
        Err(stopped) if stopped.is_cancelled() => Ok(false),
        Err(err) => Err(err.to_string()),
    }
}

/// Reads a reply's body, refusing more than `cap` bytes and giving up when nothing arrives
/// for [`STALL`]. `each` hears how many bytes there are so far.
async fn body(
    mut reply: steam::reqwest::Response,
    cap: u64,
    mut each: impl FnMut(u64),
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    loop {
        let chunk = tokio::time::timeout(STALL, reply.chunk())
            .await
            .map_err(|_| t!("the download stalled"))?
            .map_err(|err| err.to_string())?;
        let Some(chunk) = chunk else {
            return Ok(bytes);
        };
        if (bytes.len() + chunk.len()) as u64 > cap {
            return Err(t!("the download is bigger than it should be").into());
        }
        bytes.extend_from_slice(&chunk);
        each(bytes.len() as u64);
    }
}

/// Downloads `release`'s package, verifies it, tries it and puts it in `target`'s place,
/// reporting the steps on `events`. `decided` is set before anything is unpacked; if a skip
/// set it first, nothing is.
async fn install(
    release: &Release,
    target: PathBuf,
    events: &UnboundedSender<Step>,
    decided: &AtomicBool,
) -> Result<(), String> {
    let (name, total) = release
        .package
        .clone()
        .ok_or(t!("no download for this system"))?;
    if total > MAX_BYTES {
        return Err(t!("the download is far bigger than lata is").into());
    }
    let http = steam::http();
    let get = async |file: &str, cap: u64, each: &mut (dyn FnMut(u64) + Send)| {
        let reply = http
            .get(release.file(file))
            .header("User-Agent", release::AGENT)
            .send();
        let reply = tokio::time::timeout(STALL, reply)
            .await
            .map_err(|_| t!("the download stalled").to_owned())?
            .and_then(|reply| reply.error_for_status())
            .map_err(|err| err.to_string())?;
        body(reply, cap, each).await
    };
    // shortcut: the package is kept in memory (about 30 MB) instead of in a file; write it
    // to disk as it arrives if packages ever grow past a few hundred MB.
    let downloaded = tokio::time::timeout(DOWNLOAD, async {
        let sums = get(release::SUMS, SUMS_BYTES, &mut |_| ()).await?;
        let package = get(&name, total, &mut |done| {
            let _ = events.send(Step::Downloading { done, total });
        })
        .await?;
        Ok::<_, String>((sums, package))
    })
    .await
    .map_err(|_| t!("the download took too long"))?;
    let (sums, package) = downloaded?;
    let expected = release::checksum(&String::from_utf8_lossy(&sums), &name)
        .ok_or(t!("the release lists no checksum for this download"))?;
    let got: String = Sha256::digest(&package)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != expected {
        return Err(t!("the download doesn't match the release's checksum").into());
    }
    if decided.swap(true, Ordering::SeqCst) {
        return Err(t!("skipped").into());
    }
    let _ = events.send(Step::Installing);
    let version = release.version.clone();
    runtime::blocking(move || {
        let new = swap::stage(&package, &name, &target, &version)?;
        swap::swap(&new, &target).map_err(|err| err.to_string())
    })
    .await?
}
