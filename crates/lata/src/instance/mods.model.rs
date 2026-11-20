//! The mods an instance holds: what is in `zomboid/mods/`, which of them the game loads
//! (`default.txt`), and installing one from the Workshop.
//!
//! The game reads `<cachedir>/mods/<folder>`. Build 42 uses `common/` plus the highest version
//! folder (`42`, `42.13`) that is not above the running game; Build 41 uses the folder itself.
//! A mod is active when its id is listed in `mods/default.txt`, which the game empties unless a
//! `reset-mods-*.txt` sits beside it. Checked by hand and read from the game's
//! `ZomboidFileSystem` (notes in `research/game/MODS.md`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::{APP, Instance};

/// A game or mod-folder version, down to what the game compares: build and major (`42.13`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub build: u32,
    pub major: u32,
}

impl Version {
    /// `42` is 42.0, `42.13` is 42.13 and `42.13.1` is 42.13; anything else is no version.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let build = parts.next()?.parse().ok()?;
        let major = match parts.next() {
            Some(major) => major.parse().ok()?,
            None => 0,
        };
        parts
            .all(|part| part.parse::<u32>().is_ok())
            .then_some(Self { build, major })
    }
}

/// The version of the game the instance last ran, from the `version.txt` it leaves in the
/// cachedir (`42.22.2 3d39148846`). `None` before the first run.
pub fn game_version(cachedir: &Path) -> Option<Version> {
    let text = std::fs::read_to_string(cachedir.join("version.txt")).ok()?;
    Version::parse(text.split_whitespace().next()?)
}

/// Whether the instance's game uses Build 42's mod layout and `default.txt` format.
fn modern(instance: &Instance, version: Option<Version>) -> bool {
    match (version, instance.game()) {
        (Some(version), _) => version.build >= 42,
        // Not run yet: Build 42 ships one jar, Build 41 loose class files.
        (None, Some(game)) => ships_one_jar(&game),
        (None, None) => branch_is_modern(&instance.branch),
    }
}

/// Whether the game files are Build 42's, which keep `projectzomboid.jar` at the top (its
/// `ProjectZomboid64.json` puts it in the classpath there); some layouts nest it in a
/// `projectzomboid` folder. Build 41 keeps its classes under `java/`.
fn ships_one_jar(game: &Path) -> bool {
    game.join("projectzomboid.jar").is_file()
        || game.join("projectzomboid/projectzomboid.jar").is_file()
}

/// Whether Steam branch `branch` is Build 42 or newer. `public` and `pre-release` are, as
/// are the numbered and unstable ones; only the legacy branch is 41.
pub fn branch_is_modern(branch: &str) -> bool {
    !branch.contains("41")
}

/// Whether a Workshop item with these tags is made for the instance's build.
pub fn fits(instance: &Instance, tags: &[String]) -> bool {
    for_build(tags, is_modern(instance))
}

/// Whether a Workshop item with these tags is made for a game of this build line. Items are
/// tagged `Build 41`, `Build 42` or both; one with neither says nothing and passes.
/// The one build a mod's tags tie it to: `Some(true)` for Build 42 only, `Some(false)` for
/// Build 41 only, `None` when it is marked for both or neither.
pub fn only_build(tags: &[String]) -> Option<bool> {
    let has = |tag: &str| tags.iter().any(|t| t == tag);
    match (has("Build 41"), has("Build 42")) {
        (true, false) => Some(false),
        (false, true) => Some(true),
        _ => None,
    }
}

fn for_build(tags: &[String], modern: bool) -> bool {
    let has = |tag: &str| tags.iter().any(|t| t == tag);
    match (has("Build 41"), has("Build 42")) {
        (false, false) | (true, true) => true,
        (_, for_42) => for_42 == modern,
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mod {
    pub id: String,
    pub name: String,
    pub author: String,
    pub description: String,
    /// Its folder under `zomboid/mods/`.
    pub folder: String,
    /// That folder on disk.
    pub dir: PathBuf,
    /// The version folder the game reads it from (`42.13`); `None` for `common/` or Build 41.
    pub version: Option<String>,
    /// The Workshop item it was installed from, when lata installed it.
    pub workshop: Option<u64>,
    /// The manifest of that item it was downloaded at.
    pub manifest: Option<u64>,
    /// Kept at that version on purpose: updates pass it by ([`hold`]).
    pub held: bool,
    /// A Java mod: it ships a jar that a loader runs inside the game (`java`).
    pub java: bool,
    /// `icon=` (or the first `poster=`) of its `mod.info`, when the file exists.
    pub icon: Option<PathBuf>,
    /// Ids of the mods it needs.
    pub requires: Vec<String>,
    /// Ids of mods it wants to load after, without needing them (`loadModAfter=`).
    pub load_after: Vec<String>,
    /// Ids of mods it wants to load before (`loadModBefore=`).
    pub load_before: Vec<String>,
    pub enabled: bool,
    /// When it came into the instance, and when its files last changed there (an update or a
    /// version change). From `mods.json`; the folder's date for mods lata didn't install or
    /// installed before it kept dates.
    pub added: Option<SystemTime>,
    pub updated: Option<SystemTime>,
}

/// What a `mod.info` says, with only the keys lata uses.
#[derive(Debug, Default, PartialEq)]
struct Info {
    id: String,
    name: String,
    author: String,
    description: String,
    icon: Option<String>,
    requires: Vec<String>,
    load_after: Vec<String>,
    load_before: Vec<String>,
    /// It names a jar to load (`javaJarFile=`): a Java mod.
    java: bool,
}

/// The ids in a comma separated `mod.info` value. Build 42 writes them with a leading backslash.
fn ids(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(|id| id.trim().trim_start_matches('\\').to_owned())
        .filter(|id| !id.is_empty())
        .collect()
}

fn parse_info(text: &str) -> Option<Info> {
    let mut info = Info::default();
    let mut poster = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "id" => info.id = value.to_owned(),
            "name" => info.name = value.to_owned(),
            "author" => info.author = value.to_owned(),
            "description" => info.description = value.to_owned(),
            "icon" if !value.is_empty() => info.icon = Some(value.to_owned()),
            "poster" if poster.is_none() && !value.is_empty() => poster = Some(value.to_owned()),
            "require" => info.requires = ids(value),
            "loadmodafter" => info.load_after = ids(value),
            "loadmodbefore" => info.load_before = ids(value),
            "javajarfile" => info.java = !value.is_empty(),
            _ => {}
        }
    }
    info.icon = info.icon.or(poster);
    if info.name.is_empty() {
        info.name = info.id.clone();
    }
    (!info.id.is_empty()).then_some(info)
}

/// The folder of `dir` whose `mod.info` the game reads, or `None` when this game can't load
/// the mod. `version` is the game's, when known; unknown takes the newest folder.
fn info_dir(dir: &Path, modern: bool, version: Option<Version>) -> Option<PathBuf> {
    if !modern {
        return dir.join("mod.info").is_file().then(|| dir.to_owned());
    }
    let floor = Version {
        build: 42,
        major: 0,
    };
    let chosen = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let found = Version::parse(&entry.file_name().to_string_lossy())?;
            (found >= floor && version.is_none_or(|game| found <= game))
                .then(|| (found, entry.path()))
        })
        .max_by_key(|(found, _)| *found)
        .map(|(_, path)| path);
    // The chosen version folder wins; `common/` may carry the `mod.info` instead.
    chosen
        .into_iter()
        .chain([dir.join("common")])
        .find(|folder| folder.join("mod.info").is_file())
}

/// The ids `default.txt` lists, and the maps block kept as it was.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Active {
    pub mods: Vec<String>,
    pub maps: Vec<String>,
}

impl Active {
    pub fn parse(text: &str) -> Self {
        let mut active = Self::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim().trim_end_matches(',').trim();
            let value = value.trim_start_matches('\\').to_owned();
            match key.trim() {
                "mod" if !value.is_empty() => active.mods.push(value),
                "map" if !value.is_empty() => active.maps.push(value),
                _ => {}
            }
        }
        active
    }

    /// The file as the game writes it; Build 42 puts a backslash before each mod id.
    fn write(&self, modern: bool) -> String {
        let prefix = if modern { "\\" } else { "" };
        let block = |name: &str, key: &str, prefix: &str, entries: &[String]| {
            let lines: String = entries
                .iter()
                .map(|entry| format!("    {key} = {prefix}{entry},\n"))
                .collect();
            format!("{name}\n{{\n{lines}}}\n")
        };
        format!(
            "VERSION = 1,\n\n{}\n{}",
            block("mods", "mod", prefix, &self.mods),
            block("maps", "map", "", &self.maps)
        )
    }
}

pub(super) fn mods_dir(instance: &Instance) -> PathBuf {
    instance.dir.join("zomboid/mods")
}

fn read_active(instance: &Instance) -> Active {
    std::fs::read_to_string(mods_dir(instance).join("default.txt"))
        .map(|text| Active::parse(&text))
        .unwrap_or_default()
}

fn write_active(instance: &Instance, active: &Active, modern: bool) -> io::Result<()> {
    let dir = mods_dir(instance);
    std::fs::create_dir_all(&dir)?;
    // Without a `reset-mods-*.txt` the game empties `default.txt` when it starts. One it left
    // itself is the right one; before the first run, write the name its build looks for.
    // ponytail: the name is fixed per build line (42_00, 41_51); a future build that looks for
    // another one resets the list once, and after that run its own file is there.
    let has_reset = std::fs::read_dir(&dir)?.flatten().any(|entry| {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        name.starts_with("reset-mods-") && name.ends_with(".txt")
    });
    if !has_reset {
        let name = if modern {
            "reset-mods-42_00.txt"
        } else {
            "reset-mods-41_51.txt"
        };
        std::fs::write(
            dir.join(name),
            "If this file does not exist, default.txt will be reset to empty (no mods active).",
        )?;
    }
    std::fs::write(dir.join("default.txt"), active.write(modern))
}

/// Makes `active` the instance's list of mods to load, in its game's format: what an instance
/// made from a game someone already plays starts with.
pub fn activate(instance: &Instance, active: &Active) -> io::Result<()> {
    write_active(instance, active, is_modern(instance))
}

/// Every id the mod in `folder` goes by, whichever build reads it: Build 41 keeps `mod.info`
/// at the top, Build 42 in `common/` or a version folder, and one mod can carry both.
pub fn ids_in(folder: &Path) -> Vec<String> {
    let inner = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path());
    let mut ids: Vec<String> = std::iter::once(folder.to_owned())
        .chain(inner)
        .filter_map(|dir| std::fs::read_to_string(dir.join("mod.info")).ok())
        .filter_map(|text| parse_info(&text))
        .map(|info| info.id)
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The mods in the instance that its game can load, by name.
pub fn list(instance: &Instance) -> Vec<Mod> {
    let version = game_version(&instance.dir.join("zomboid"));
    let modern = modern(instance, version);
    let active = read_active(instance);
    let pins = read_pins(instance);
    let Ok(entries) = std::fs::read_dir(mods_dir(instance)) else {
        return Vec::new();
    };
    let mut mods: Vec<Mod> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let folder = entry.file_name().to_string_lossy().into_owned();
            // The game skips its own template the same way.
            if folder.eq_ignore_ascii_case("examplemod") {
                return None;
            }
            let from = info_dir(&entry.path(), modern, version)?;
            let info = parse_info(&std::fs::read_to_string(from.join("mod.info")).ok()?)?;
            let icon = info
                .icon
                .map(|file| from.join(file))
                .filter(|path| path.is_file());
            let version = from
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| Version::parse(name).is_some());
            let pin = pins.items.iter().find(|pin| pin.folders.contains(&folder));
            let at = |secs: Option<u64>| secs.map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
            let touched = entry.metadata().and_then(|m| m.modified()).ok();
            Some(Mod {
                added: at(pin.and_then(|pin| pin.added)).or(touched),
                updated: at(pin.and_then(|pin| pin.updated)).or(touched),
                enabled: active.mods.contains(&info.id),
                id: info.id,
                name: info.name,
                author: info.author,
                description: info.description,
                workshop: pin.map(|pin| pin.id),
                manifest: pin.map(|pin| pin.manifest),
                held: pin.is_some_and(|pin| pin.held),
                java: info.java,
                dir: entry.path(),
                version,
                folder,
                icon,
                requires: info.requires,
                load_after: info.load_after,
                load_before: info.load_before,
            })
        })
        .collect();
    mods.sort_by_key(|m| m.name.to_lowercase());
    mods
}

/// Names of the mods in the instance made only for a newer game than it runs: every version
/// folder they carry is above its version, so the game skips them and [`list`] leaves them
/// out. A mod update can do that to an instance that hasn't updated its game yet.
pub fn too_new(instance: &Instance) -> Vec<String> {
    let Some(version) = game_version(&instance.dir.join("zomboid")) else {
        return Vec::new();
    };
    if version.build < 42 {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(mods_dir(instance)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|dir| dir.is_dir() && info_dir(dir, true, Some(version)).is_none())
        .filter_map(|dir| {
            // The newest folder is there, or `info_dir` would have found it without a version.
            let newest = info_dir(&dir, true, None)?;
            let info = parse_info(&std::fs::read_to_string(newest.join("mod.info")).ok()?)?;
            Some(if info.name.is_empty() {
                info.id
            } else {
                info.name
            })
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// The ids of `mods` in an order that gives every mod what it asks for: after the mods it
/// requires and the ones it loads after (`loadModAfter=`), before the ones it loads before
/// (`loadModBefore=`), and by id where nothing decides. Ids that aren't in `mods` ask for
/// nothing.
///
/// The game's own AUTO button (`ModLoadOrderPanel:autoSort`, Build 42.21) doesn't go this
/// far: it places mods by id and lets an earlier id win over `loadModAfter` and
/// `loadModBefore`, so its result can ignore both. Any order from here passes the game's
/// check, which only looks at `require`.
///
/// When mods ask for a circle, the wishes give way first and requirements last; mods that
/// require each other go by id.
pub fn auto_sort(mods: &[Mod]) -> Vec<String> {
    let mut left: Vec<&Mod> = mods.iter().collect();
    left.sort_by(|a, b| a.id.cmp(&b.id));
    left.dedup_by(|a, b| a.id == b.id);
    let mut order = Vec::with_capacity(left.len());
    while !left.is_empty() {
        // Whether `first` has to load before `m`, counting wishes or only requirements.
        let waits = |m: &Mod, first: &Mod, wishes: bool| {
            m.id != first.id
                && (m.requires.contains(&first.id)
                    || wishes
                        && (m.load_after.contains(&first.id) || first.load_before.contains(&m.id)))
        };
        let free = |wishes: bool| {
            left.iter()
                .position(|m| !left.iter().any(|first| waits(m, first, wishes)))
        };
        let next = free(true).or_else(|| free(false)).unwrap_or(0);
        order.push(left.remove(next).id.clone());
    }
    order
}

/// The first mod in `order` that loads before one it requires, with that one: the check the
/// game runs before it accepts an order (`ModLoadOrderPanel:isCorrectOrder`). A requirement
/// that isn't in `order` at all is left alone here, where the game would refuse every order:
/// no arrangement can fix it, and the Mods tab already says so when the mod is turned on.
pub fn out_of_order<'a>(order: &[&'a Mod]) -> Option<(&'a Mod, &'a Mod)> {
    order.iter().enumerate().find_map(|(at, m)| {
        let late = order[at + 1..]
            .iter()
            .find(|after| m.requires.contains(&after.id))?;
        Some((*m, *late))
    })
}

/// The mods the game loads and the maps block, as `default.txt` has them.
pub fn active(instance: &Instance) -> Active {
    read_active(instance)
}

/// Makes `mods` the order the game loads the active mods in, and `maps` the order of their
/// maps when one is given. Only the order changes: an id that isn't active is left out, and
/// a mod turned on since `mods` was read stays, at the end.
pub fn set_order(instance: &Instance, mods: &[String], maps: Option<&[String]>) -> io::Result<()> {
    let mut active = read_active(instance);
    let mut order: Vec<String> = Vec::new();
    for id in mods.iter().chain(&active.mods) {
        if active.mods.contains(id) && !order.contains(id) {
            order.push(id.clone());
        }
    }
    active.mods = order;
    if let Some(maps) = maps {
        active.maps = maps.to_vec();
    }
    write_active(instance, &active, is_modern(instance))
}

/// When the instance's pins and active list last changed.
pub type Stamp = [Option<std::time::SystemTime>; 3];

/// The [`Stamp`] of the instance now: three `stat` calls, cheap enough for the UI thread.
/// Every install, removal and turn on or off rewrites one of the first two files; the
/// instance's own settings (Java mods on or off) are the third.
pub fn stamp(instance: &Instance) -> Stamp {
    let changed = |path: PathBuf| std::fs::metadata(path).and_then(|m| m.modified()).ok();
    [
        changed(instance.dir.join("mods.json")),
        changed(mods_dir(instance).join("default.txt")),
        changed(instance.dir.join("instance.json")),
    ]
}

/// Turns mod `id` on or off. Turning one on also turns on the installed mods it requires;
/// the ids it requires that aren't installed come back so the caller can say so.
pub fn set_enabled(instance: &Instance, id: &str, on: bool) -> io::Result<Vec<String>> {
    let version = game_version(&instance.dir.join("zomboid"));
    let modern = modern(instance, version);
    let installed = list(instance);
    let mut active = read_active(instance);
    let mut missing = Vec::new();
    if on {
        // Required mods go before the ones that need them.
        let mut wanted = vec![id.to_owned()];
        while let Some(next) = wanted.pop() {
            if active.mods.contains(&next) {
                continue;
            }
            match installed.iter().find(|m| m.id == next) {
                Some(found) => {
                    let at = active
                        .mods
                        .iter()
                        .position(|other| {
                            installed
                                .iter()
                                .any(|m| m.id == *other && m.requires.contains(&next))
                        })
                        .unwrap_or(active.mods.len());
                    active.mods.insert(at, next);
                    wanted.extend(found.requires.iter().cloned());
                }
                None => missing.push(next),
            }
        }
    } else {
        active.mods.retain(|other| other != id);
    }
    write_active(instance, &active, modern)?;
    Ok(missing)
}

/// Deletes the mod in `folder` and takes it off the active list.
pub fn remove(instance: &Instance, folder: &str) -> io::Result<()> {
    // One plain folder name, never a path out of `mods/`.
    if Path::new(folder).file_name() != Some(std::ffi::OsStr::new(folder)) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a mod folder",
        ));
    }
    let gone: Vec<String> = list(instance)
        .into_iter()
        .filter(|m| m.folder == folder)
        .map(|m| m.id)
        .collect();
    std::fs::remove_dir_all(mods_dir(instance).join(folder))?;
    let version = game_version(&instance.dir.join("zomboid"));
    let mut active = read_active(instance);
    active.mods.retain(|id| !gone.contains(id));
    write_active(instance, &active, modern(instance, version))?;
    let mut pins = read_pins(instance);
    for pin in &mut pins.items {
        pin.folders.retain(|kept| kept != folder);
    }
    pins.items.retain(|pin| !pin.folders.is_empty());
    write_pins(instance, &pins)
}

/// A Workshop item installed in the instance, pinned to the version that was downloaded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Pin {
    pub id: u64,
    pub manifest: u64,
    pub title: String,
    /// The folders under `zomboid/mods/` that came from it.
    pub folders: Vec<String>,
    /// The user picked this version: updates leave the item alone until they pick the newest.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub held: bool,
    /// Unix seconds of its first install here and of the last time its files changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub added: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<u64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Pins {
    #[serde(default)]
    items: Vec<Pin>,
}

/// The Workshop items the instance has, at the version each was downloaded.
pub fn pins(instance: &Instance) -> Vec<Pin> {
    read_pins(instance).items
}

fn read_pins(instance: &Instance) -> Pins {
    std::fs::read_to_string(instance.dir.join("mods.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_pins(instance: &Instance, pins: &Pins) -> io::Result<()> {
    let text = serde_json::to_string_pretty(pins).map_err(io::Error::other)?;
    std::fs::write(instance.dir.join("mods.json"), text)
}

/// Keeps Workshop items `ids` at the version they are on (`on`), or lets updates reach them
/// again. A mod rolled back because its newest version broke would otherwise come right back
/// with the next "Update all".
pub fn hold(instance: &Instance, ids: &[u64], on: bool) -> io::Result<()> {
    let mut pins = read_pins(instance);
    for pin in pins.items.iter_mut().filter(|pin| ids.contains(&pin.id)) {
        pin.held = on;
    }
    write_pins(instance, &pins)
}

/// The versions the last update replaced, to go back to all of them at once: breakage
/// usually comes from what changed together, not from one mod.
const UNDO: &str = "mods.undo.json";

/// Before updating `ids`: remembers the version each one is at, replacing what an earlier
/// update left. Only versions are kept, not files; Steam still serves old ones.
pub fn remember(instance: &Instance, ids: &[u64]) -> io::Result<()> {
    let before: Vec<(u64, u64)> = read_pins(instance)
        .items
        .iter()
        .filter(|pin| ids.contains(&pin.id))
        .map(|pin| (pin.id, pin.manifest))
        .collect();
    let text = serde_json::to_string(&before).map_err(io::Error::other)?;
    std::fs::write(instance.dir.join(UNDO), text)
}

/// What [`remember`] kept that still differs from what is installed: each item with the
/// version to go back to. Empty when there is nothing to undo.
pub fn undo(instance: &Instance) -> Vec<(u64, u64)> {
    let before: Vec<(u64, u64)> = std::fs::read_to_string(instance.dir.join(UNDO))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    let pins = read_pins(instance);
    before
        .into_iter()
        .filter(|(id, manifest)| {
            pins.items
                .iter()
                .any(|pin| pin.id == *id && pin.manifest != *manifest)
        })
        .collect()
}

/// Drops what [`remember`] kept once all of it has been undone. Undoing only some items
/// keeps the file: the ones put back already match it, so [`undo`] stops offering them.
pub fn forget(instance: &Instance) {
    if undo(instance).is_empty() {
        let _ = std::fs::remove_file(instance.dir.join(UNDO));
    }
}

/// Moves the mod folders of a downloaded Workshop item (`<item>/mods/<folder>`) into the
/// instance, replacing older copies, and returns their names.
fn adopt(instance: &Instance, item: &Path) -> io::Result<Vec<String>> {
    let target = mods_dir(instance);
    std::fs::create_dir_all(&target)?;
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(item.join("mods"))?.flatten() {
        if !entry.path().is_dir() {
            continue;
        }
        let name = entry.file_name();
        let to = target.join(&name);
        if to.exists() {
            std::fs::remove_dir_all(&to)?;
        }
        std::fs::rename(entry.path(), &to)?;
        folders.push(name.to_string_lossy().into_owned());
    }
    folders.sort();
    Ok(folders)
}

/// Whether the instance runs Build 42 or newer (its mod layout and Workshop tag).
pub fn is_modern(instance: &Instance) -> bool {
    modern(instance, game_version(&instance.dir.join("zomboid")))
}

/// A Workshop item as shown before installing it.
#[derive(Debug, Clone)]
pub struct Listing {
    pub item: steam::workshop::Item,
    /// Its Workshop picture, downloaded; `None` when it has none or it couldn't be fetched.
    pub picture: Option<PathBuf>,
}

/// The file extension of a picture, from how its bytes start.
fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xFF, 0xD8, 0xFF, ..] => Some("jpg"),
        [0x89, b'P', b'N', b'G', ..] => Some("png"),
        [b'G', b'I', b'F', b'8', ..] => Some("gif"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("webp"),
        _ => None,
    }
}

/// `item`'s Workshop picture, downloaded once into the cache next to the instances (like the
/// avatars). `None` when it has none, it couldn't be fetched, or it is in a format that
/// isn't shown.
pub async fn picture(http: &steam::reqwest::Client, url: Option<&str>) -> Option<PathBuf> {
    let cache = super::store::data().join("cache").join("workshop");
    let url = url?;
    // Named by the picture's own address, which changes when its author replaces it.
    let name: String = url
        .trim_end_matches('/')
        .rsplit('/')
        .next()?
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(40)
        .collect();
    for ext in ["jpg", "png", "gif", "webp"] {
        let path = cache.join(format!("{name}.{ext}"));
        if path.is_file() {
            return Some(path);
        }
    }
    let bytes = steam::workshop::preview(http, url).await.ok()?;
    let path = cache.join(format!("{name}.{}", picture_ext(&bytes)?));
    std::fs::create_dir_all(&cache).ok()?;
    std::fs::write(&path, bytes).ok()?;
    Some(path)
}

/// What the Workshop says about item `id`, with its picture.
pub async fn lookup(id: u64) -> Result<Listing, String> {
    let found = crate::runtime::spawn(async move {
        let http = steam::http();
        let mut client = steam::Client::anonymous(&http).await?;
        let item = steam::workshop::details(&mut client, &[id])
            .await?
            .into_iter()
            .next()
            .filter(|item| item.app == APP && !item.title.is_empty());
        let Some(item) = item else { return Ok(None) };
        let picture = picture(&http, item.preview.as_deref()).await;
        Ok::<_, steam::Error>(Some(Listing { item, picture }))
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())?;
    found.ok_or_else(|| "Steam has no Project Zomboid item with that id".to_owned())
}

/// Turns several mods on or off; the ids they need that aren't installed come back.
pub fn set_enabled_all(instance: &Instance, ids: &[String], on: bool) -> io::Result<Vec<String>> {
    let mut missing = Vec::new();
    for id in ids {
        missing.extend(set_enabled(instance, id, on)?);
    }
    missing.sort();
    missing.dedup();
    Ok(missing)
}

/// What an install brought in.
#[derive(Debug, PartialEq)]
pub struct Installed {
    /// Title of the first item that was asked for.
    pub title: String,
    /// Titles of the other Workshop items installed with it: the rest of what was asked for,
    /// then what those require.
    pub with: Vec<String>,
    /// Titles of the items asked for at a version Steam no longer serves, which came at their
    /// current one instead.
    pub newer: Vec<String>,
}

/// The most items one install follows through `requires` beyond the ones asked for; a mod
/// needs a handful at most.
const MAX_REQUIRED: usize = 50;
/// Items asked of the Workshop in one request.
const BATCH: usize = 50;

/// Workshop items downloading at once. Each one already fetches 32 chunks at a time, so this
/// is about overlapping the per-item round trips with the downloads, not about bandwidth.
const PARALLEL_ITEMS: usize = 4;

/// What [`latest`] gives for an item the Workshop no longer has (removed, or hidden).
pub const GONE: u64 = 0;

/// What a download task ended with.
fn joined(done: Result<Result<(), String>, tokio::task::JoinError>) -> Result<(), String> {
    done.map_err(|err| err.to_string())?
}

/// The current manifest of every Workshop item installed in the instance, by item id. A mod is
/// out of date when its [`Mod::manifest`] isn't this one; 0 means the Workshop no longer has
/// the item ([`GONE`]).
pub async fn latest(instance: &Instance) -> Result<std::collections::HashMap<u64, u64>, String> {
    let ids: Vec<u64> = read_pins(instance).items.iter().map(|pin| pin.id).collect();
    crate::runtime::spawn(async move {
        let http = steam::http();
        let mut client = steam::Client::anonymous(&http).await?;
        let mut latest = std::collections::HashMap::new();
        for batch in ids.chunks(BATCH) {
            for item in steam::workshop::details(&mut client, batch).await? {
                // Gone or private items answer with no manifest: kept as 0, which says Steam
                // was asked and doesn't have it. An item missing here just wasn't asked about
                // (installed after this check).
                latest.insert(item.id, item.manifest);
            }
        }
        Ok::<_, steam::Error>(latest)
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())
}

/// Moves a downloaded item from `staging` into the instance, pins it and turns on the mods it
/// brought that weren't there before; an update leaves each mod on or off as it was.
fn land(instance: &Instance, item: &steam::workshop::Item, staging: &Path) -> io::Result<()> {
    let installed = list(instance);
    let before: Vec<&str> = installed.iter().map(|m| m.id.as_str()).collect();
    let mut pins = read_pins(instance);
    let secs = |at: SystemTime| at.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
    // An item already here keeps the day it came; one pinned before dates were kept takes its
    // folder's, read before the new files replace it.
    let added = pins
        .items
        .iter()
        .find(|pin| pin.id == item.id)
        .and_then(|pin| {
            pin.added.or_else(|| {
                let old = installed.iter().find(|m| pin.folders.contains(&m.folder))?;
                secs(old.added?)
            })
        });
    let now = secs(SystemTime::now());
    let folders = adopt(instance, staging)?;
    let _ = std::fs::remove_dir_all(staging);
    // A folder belongs to the item that brought it last. Another item with a folder of the
    // same name just replaced it, so the first one no longer owns what is there: keeping it
    // listed would let, say, an item with a `ZombieBuddy` folder pass for the Java loader.
    // Compared without case: on Windows and macOS `zombiebuddy` lands on top of `ZombieBuddy`,
    // and two names that differ only in case must not both count as owners anywhere.
    let lower: Vec<String> = folders.iter().map(|f| f.to_lowercase()).collect();
    for pin in pins.items.iter_mut().filter(|pin| pin.id != item.id) {
        pin.folders
            .retain(|folder| !lower.contains(&folder.to_lowercase()));
    }
    // Folders the new version no longer has would stay behind as mods of no item.
    for pin in pins.items.iter().filter(|pin| pin.id == item.id) {
        for old in pin.folders.iter().filter(|old| !folders.contains(old)) {
            let _ = std::fs::remove_dir_all(mods_dir(instance).join(old));
        }
    }
    pins.items.retain(|pin| pin.id != item.id);
    pins.items.push(Pin {
        id: item.id,
        manifest: item.manifest,
        title: item.title.clone(),
        folders: folders.clone(),
        held: false,
        added: added.or(now),
        updated: now,
    });
    write_pins(instance, &pins)?;
    // On as soon as it lands, so a cancelled install leaves what it got usable. A mod whose
    // requirement comes later is fine: turning that one on puts it first.
    // ponytail: each turn-on rescans the mods folder, quadratic over a collection (325 mods:
    // about 100k small reads); keep the list in memory if it shows.
    for found in list(instance)
        .iter()
        .filter(|m| folders.contains(&m.folder) && !before.contains(&m.id.as_str()))
    {
        set_enabled(instance, &found.id, true)?;
    }
    Ok(())
}

/// Downloads Workshop items `ids` into the instance with the items their authors marked as
/// required (and theirs), and turns all their mods on. Items the instance already has are
/// left alone. `progress` gets the title being downloaded with the bytes done and in total,
/// over everything. Meant to run on the background runtime as one task, so aborting it stops
/// the download; each item is moved into the instance as soon as it is down, so what finished
/// before a cancel stays. Items in `exact` come at the manifest it gives (a pack's versions)
/// when Steam still serves it.
// ponytail: each instance keeps its own copy of a mod; share them like game builds if disk
// use becomes a problem.
pub async fn install(
    instance: Instance,
    ids: Vec<u64>,
    exact: Vec<(u64, u64)>,
    progress: impl FnMut(&str, u64, u64) + Send + 'static,
) -> Result<Installed, String> {
    let fail = |err: steam::Error| err.to_string();
    let incoming = instance.dir.join(".incoming");
    let http = steam::http();
    let mut client = steam::Client::anonymous(&http).await.map_err(fail)?;

    // The items asked for first (downloaded again if here, which updates them), then what
    // they require, each once; required items already here are skipped.
    let have: Vec<u64> = read_pins(&instance)
        .items
        .iter()
        .map(|pin| pin.id)
        .collect();
    let mut seen: Vec<u64> = Vec::new();
    for id in &ids {
        if !seen.contains(id) {
            seen.push(*id);
        }
    }
    let mut queue = seen.clone();
    let mut items = Vec::new();
    while !queue.is_empty() && seen.len() <= ids.len() + MAX_REQUIRED {
        let take = queue.len().min(BATCH);
        let batch: Vec<u64> = queue.drain(..take).collect();
        let found = steam::workshop::details(&mut client, &batch)
            .await
            .map_err(fail)?;
        for item in found {
            if item.app != APP || item.manifest == 0 {
                continue;
            }
            for needed in &item.requires {
                if !seen.contains(needed) && !have.contains(needed) {
                    seen.push(*needed);
                    queue.push(*needed);
                }
            }
            items.push(item);
        }
    }
    if !items.iter().any(|item| ids.contains(&item.id)) {
        return Err(if ids.len() == 1 {
            "That isn't a Project Zomboid mod. Collections get installed when you create an instance"
                .into()
        } else {
            "None of those are Project Zomboid mods Steam still has".into()
        });
    }

    let _ = std::fs::remove_dir_all(&incoming);
    let total: u64 = items.iter().map(|item| item.size).sum();
    if let Some(why) = crate::storage::room(&instance.dir, total) {
        return Err(why);
    }
    let source = steam::workshop::source(&mut client, APP)
        .await
        .map_err(fail)?;
    // Bytes done per item, summed for the overall progress by every download in flight.
    let report = Arc::new(Mutex::new((vec![0u64; items.len()], progress)));
    // Moving an item in and turning it on edits `mods.json` and `default.txt`: one at a time.
    let landing = Arc::new(Mutex::new(()));
    let mut running = tokio::task::JoinSet::new();
    let mut newer = Vec::new();
    for (ix, item) in items.iter_mut().enumerate() {
        // The manifest needs the Steam client, so manifests come one by one while the items
        // before download. Per item that is most of the time: a 370 KB mod took 400 ms of
        // manifest and 40 ms of files.
        let pinned = exact
            .iter()
            .find(|(id, manifest)| *id == item.id && *manifest != item.manifest);
        let old = match pinned {
            Some((_, manifest)) => {
                let old = steam::workshop::Item {
                    manifest: *manifest,
                    ..item.clone()
                };
                ask(&mut client, &http, &source, &old)
                    .await
                    .ok()
                    .map(|found| (found, old.manifest))
            }
            None => None,
        };
        if pinned.is_some() && old.is_none() {
            newer.push(item.title.clone());
        }
        let manifest = match old {
            // The pin `land` writes says which version came.
            Some((found, number)) => {
                item.manifest = number;
                found
            }
            None => ask(&mut client, &http, &source, item).await.map_err(fail)?,
        };
        if running.len() >= PARALLEL_ITEMS
            && let Some(done) = running.join_next().await
        {
            joined(done)?;
        }
        let (http, source, report, landing) = (
            http.clone(),
            source.clone(),
            report.clone(),
            landing.clone(),
        );
        let (instance, item, staging) = (
            instance.clone(),
            item.clone(),
            incoming.join(item.id.to_string()),
        );
        running.spawn(async move {
            let tell = |done: u64| {
                let mut report = report.lock().unwrap_or_else(PoisonError::into_inner);
                report.0[ix] = done;
                let sum = report.0.iter().sum();
                (report.1)(&item.title, sum, total);
            };
            steam::workshop::fetch(&http, &source, &manifest, &staging, &tell)
                .await
                .map_err(fail)?;
            tell(item.size);
            crate::runtime::blocking(move || {
                let _one = landing.lock().unwrap_or_else(PoisonError::into_inner);
                land(&instance, &item, &staging)
            })
            .await?
            .map_err(|err| err.to_string())
        });
    }
    while let Some(done) = running.join_next().await {
        joined(done)?;
    }
    drop(client);
    let _ = std::fs::remove_dir_all(&incoming);
    let mut titles = items.into_iter().map(|item| item.title);
    Ok(Installed {
        title: titles.next().unwrap_or_default(),
        with: titles.collect(),
        newer,
    })
}

/// [`steam::workshop::manifest`], reconnecting while the network is down: a batch asks for
/// manifests while files download, and changing networks kills the Steam session under it.
async fn ask(
    client: &mut steam::Client,
    http: &steam::reqwest::Client,
    source: &steam::workshop::Source,
    item: &steam::workshop::Item,
) -> Result<steam::content::Manifest, steam::Error> {
    let started = std::time::Instant::now();
    loop {
        match steam::workshop::manifest(client, http, source, item).await {
            Err(err) if err.is_network() && started.elapsed() < OFFLINE => {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if let Ok(fresh) = steam::Client::anonymous(http).await {
                    *client = fresh;
                }
            }
            done => return done,
        }
    }
}

/// How long a batch waits for the network to come back before giving up.
const OFFLINE: std::time::Duration = std::time::Duration::from_secs(180);

/// A Workshop collection as the create dialog shows it before installing.
#[derive(Debug, Clone)]
pub struct Bundle {
    pub id: u64,
    pub title: String,
    /// Its Workshop picture (or its first mod's, when it has none), downloaded, as
    /// [`Listing::picture`].
    pub picture: Option<PathBuf>,
    /// Its mods with the ones they require, each once.
    pub mods: Vec<steam::workshop::Item>,
}

impl Bundle {
    /// The mods made for a game of this build line, and the ones tagged for the other.
    pub fn split(
        &self,
        modern: bool,
    ) -> (Vec<&steam::workshop::Item>, Vec<&steam::workshop::Item>) {
        self.mods
            .iter()
            .filter(|item| item.app == APP && item.manifest != 0)
            .partition(|item| for_build(&item.tags, modern))
    }
}

/// What the Workshop says is in collection `id`.
pub async fn collection(id: u64) -> Result<Bundle, String> {
    crate::runtime::spawn(async move {
        let http = steam::http();
        let mut client = steam::Client::anonymous(&http).await?;
        let found = steam::workshop::collection(&mut client, id).await?;
        // Many collections have no picture of their own; the first mod with one stands in.
        let url = found
            .preview
            .as_deref()
            .or_else(|| found.mods.iter().find_map(|item| item.preview.as_deref()));
        let picture = picture(&http, url).await;
        Ok::<_, steam::Error>(Bundle {
            id: found.id,
            title: found.title,
            picture,
            mods: found.mods,
        })
    })
    .await
    .map_err(|err| err.to_string())?
    .map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `mod.info` of Workshop item 3832274571, as downloaded.
    const INFO: &str = "name=Floor Attack Highlight Fix\nid=FloorAttackHighlightFix\nauthor=wget\ndescription=fixes misleading melee combat outlines on ground targets like crawlers and knocked-down zombies\nposter=poster.png\nicon=icon.png\n";

    /// `default.txt` as Build 42.19 left it with that mod active.
    const DEFAULT: &str =
        "VERSION = 1,\n\nmods\n{\n    mod = \\FloorAttackHighlightFix,\n}\n\nmaps\n{\n}\n";

    fn instance(dir: &Path) -> Instance {
        Instance {
            id: "a".into(),
            dir: dir.to_owned(),
            name: "A".into(),
            branch: "public".into(),
            build: None,
            last_played: None,
            icon: None,
            memory_gb: None,
            jvm_args: String::new(),
            launch_args: String::new(),
        }
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-mods-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, file: &str, text: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn tells_the_build_of_a_game_that_never_ran_by_its_files() {
        let dir = temp("jar");
        // Build 41 keeps its classes under `java/`.
        put(&dir, "java/projectzomboid.jar", "");
        assert!(!ships_one_jar(&dir));
        // Build 42 on Windows: the jar at the top, as its launcher config says.
        put(&dir, "projectzomboid.jar", "");
        assert!(ships_one_jar(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
        // The nested layout.
        let nested = temp("jar-nested");
        put(&nested, "projectzomboid/projectzomboid.jar", "");
        assert!(ships_one_jar(&nested));
        std::fs::remove_dir_all(&nested).unwrap();
    }

    #[test]
    fn fits_an_item_to_the_instance_by_its_build_tags() {
        let dir = temp("fits");
        let tags = |list: &[&str]| list.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        // Never installed: the branch decides, and only the legacy one is Build 41.
        let mut new = instance(&dir);
        for (branch, modern) in [("public", true), ("pre-release", true), ("legacy41", false)] {
            new.branch = branch.into();
            assert_eq!(is_modern(&new), modern, "{branch}");
            assert_eq!(fits(&new, &tags(&["Build 42", "QOL"])), modern, "{branch}");
            assert_eq!(fits(&new, &tags(&["Build 41"])), !modern, "{branch}");
            assert!(fits(&new, &tags(&["Build 41", "Build 42"])), "{branch}");
            assert!(
                fits(&new, &tags(&["Maps"])),
                "no build tag passes: {branch}"
            );
        }
        // Once the game has run, what it ran wins over the branch's name.
        put(&dir, "zomboid/version.txt", "41.78.16 abc\n");
        new.branch = "public".into();
        assert!(!is_modern(&new));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_collection_keeps_the_mods_for_the_chosen_build() {
        let item = |id, tags: &[&str], manifest| steam::workshop::Item {
            id,
            app: APP,
            title: id.to_string(),
            author: 0,
            summary: String::new(),
            manifest,
            file_url: None,
            size: 1,
            updated: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            preview: None,
            requires: Vec::new(),
            subscriptions: 0,
            file_type: 0,
        };
        let bundle = Bundle {
            id: 1,
            title: "Pack".into(),
            picture: None,
            mods: vec![
                item(10, &["Build 42"], 5),
                item(11, &["Build 41"], 5),
                item(12, &["Maps"], 5),
                item(13, &["Build 41", "Build 42"], 5),
                // No manifest: nothing to download.
                item(14, &["Build 42"], 0),
            ],
        };
        let ids = |modern| {
            let (fit, other) = bundle.split(modern);
            let ids = |items: Vec<&steam::workshop::Item>| items.iter().map(|i| i.id).collect();
            (ids(fit), ids(other))
        };
        assert_eq!(ids(true), (vec![10, 12, 13], vec![11]));
        assert_eq!(ids(false), (vec![11, 12, 13], vec![10]));
    }

    #[test]
    fn tells_a_picture_by_its_first_bytes() {
        assert_eq!(picture_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(picture_ext(b"\x89PNG\r\n"), Some("png"));
        assert_eq!(picture_ext(b"GIF89a"), Some("gif"));
        assert_eq!(picture_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(picture_ext(b"<html>"), None);
        assert_eq!(picture_ext(&[]), None);
    }

    #[test]
    fn versions_keep_build_and_major() {
        let v = |build, major| Some(Version { build, major });
        assert_eq!(Version::parse("42"), v(42, 0));
        assert_eq!(Version::parse("42.13"), v(42, 13));
        assert_eq!(Version::parse("42.13.1"), v(42, 13));
        for not in ["", "common", "media", "42.x", "v42"] {
            assert_eq!(Version::parse(not), None, "{not}");
        }
        assert!(Version::parse("42.9") < Version::parse("42.13"));
    }

    #[test]
    fn reads_what_a_mod_info_says() {
        let info = parse_info(INFO).unwrap();
        assert_eq!(info.id, "FloorAttackHighlightFix");
        assert_eq!(info.name, "Floor Attack Highlight Fix");
        assert_eq!(info.icon.as_deref(), Some("icon.png"));
        assert_eq!(info.author, "wget");
        assert!(info.description.starts_with("fixes misleading"));
        assert!(info.requires.is_empty());

        let needy = parse_info("id=B\nrequire=\\A, C\nposter=one.png\nposter=two.png").unwrap();
        assert_eq!(needy.name, "B", "the id stands in for a missing name");
        assert_eq!(needy.requires, ["A", "C"]);
        assert_eq!(needy.icon.as_deref(), Some("one.png"));
        assert_eq!(parse_info("name=No id"), None);
    }

    #[test]
    fn reads_where_a_mod_wants_to_load() {
        let info = parse_info(
            "id=B\nrequire=\\Q\nloadModAfter=\\A, C\nLoadModBefore = Z ,\\Y\nloadModAfter2=x",
        )
        .unwrap();
        assert_eq!(info.requires, ["Q"]);
        assert_eq!(info.load_after, ["A", "C"]);
        assert_eq!(info.load_before, ["Z", "Y"]);
        let plain = parse_info("id=P").unwrap();
        assert!(plain.load_after.is_empty() && plain.load_before.is_empty());
    }

    /// A mod that only says what it requires and where it wants to load.
    fn node(id: &str, requires: &[&str], after: &[&str], before: &[&str]) -> Mod {
        let list = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect();
        Mod {
            id: id.into(),
            name: id.into(),
            author: String::new(),
            description: String::new(),
            folder: id.into(),
            dir: PathBuf::new(),
            version: None,
            workshop: None,
            manifest: None,
            held: false,
            java: false,
            icon: None,
            requires: list(requires),
            load_after: list(after),
            load_before: list(before),
            enabled: true,
            added: None,
            updated: None,
        }
    }

    #[test]
    fn auto_sort_gives_every_mod_what_it_asks_for() {
        let sorted = |mods: &[Mod]| auto_sort(mods).join(" ");
        let b = || node("b", &[], &[], &[]);
        let c = || node("c", &[], &[], &[]);
        // Nothing asked: by id, capitals first.
        assert_eq!(sorted(&[c(), node("a", &[], &[], &[]), b()]), "a b c");
        assert!(auto_sort(&[]).is_empty());
        let cased = [
            node("Zeta", &[], &[], &[]),
            node("alpha", &[], &[], &[]),
            node("Beta", &[], &[], &[]),
        ];
        assert_eq!(sorted(&cased), "Beta Zeta alpha");
        assert_eq!(sorted(&[node("a", &["c"], &[], &[]), b(), c()]), "b c a");
        assert_eq!(
            sorted(&[node("a", &[], &["c", "b"], &[]), b(), c()]),
            "b c a"
        );
        // The game's AUTO leaves `a` first here; `c` asked to load before it.
        let before = [node("a", &[], &[], &[]), b(), node("c", &[], &[], &["a"])];
        assert_eq!(sorted(&before), "b c a");
        let chain = [
            node("a", &["b"], &[], &[]),
            node("b", &["c"], &[], &[]),
            node("c", &["d"], &[], &[]),
            node("d", &[], &[], &[]),
        ];
        assert_eq!(sorted(&chain), "d c b a");
        // The game's AUTO answers "a d b c", with `d` ahead of the `c` it loads after.
        let late = [
            node("a", &[], &[], &[]),
            node("b", &["d"], &[], &[]),
            node("c", &["a"], &[], &[]),
            node("d", &[], &["c"], &[]),
        ];
        assert_eq!(sorted(&late), "a c d b");
        let mixed = [
            node("lib", &[], &[], &[]),
            node("map", &["lib"], &[], &["zz"]),
            node("zz", &[], &[], &[]),
            node("aa", &["lib"], &["zz"], &[]),
        ];
        assert_eq!(sorted(&mixed), "lib map zz aa");
        // Whatever comes out passes the game's own check.
        for list in [&chain[..], &late[..], &mixed[..]] {
            let order = auto_sort(list);
            let placed: Vec<&Mod> = order
                .iter()
                .map(|id| list.iter().find(|m| m.id == *id).unwrap())
                .collect();
            assert!(out_of_order(&placed).is_none(), "{order:?}");
        }
    }

    #[test]
    fn auto_sort_keeps_every_mod_when_they_ask_for_a_circle() {
        let sorted = |mods: &[Mod]| auto_sort(mods).join(" ");
        // Wishes in a circle give way: by id.
        let wishes = [node("a", &[], &["b"], &[]), node("b", &[], &["a"], &[])];
        assert_eq!(sorted(&wishes), "a b");
        // A wish against a requirement: the requirement holds.
        let mixed = [node("a", &["b"], &[], &[]), node("b", &[], &["a"], &[])];
        assert_eq!(sorted(&mixed), "b a");
        let needs = [
            node("a", &["b"], &[], &[]),
            node("b", &["a"], &[], &[]),
            node("c", &[], &[], &[]),
        ];
        assert_eq!(sorted(&needs), "c a b", "what can be placed goes first");
        // Asking for a mod that isn't there, or for itself, is nothing.
        let loose = [
            node("a", &["gone"], &["gone"], &["gone"]),
            node("b", &[], &[], &[]),
        ];
        assert_eq!(sorted(&loose), "a b");
        assert_eq!(sorted(&[node("a", &["a"], &["a"], &["a"])]), "a");
        let twice = [node("a", &[], &[], &[]), node("a", &[], &[], &[])];
        assert_eq!(sorted(&twice), "a");
    }

    #[test]
    fn an_order_is_wrong_when_a_mod_loads_before_what_it_requires() {
        let (lib, map) = (
            node("lib", &[], &[], &[]),
            node("map", &["lib", "gone"], &["x"], &["y"]),
        );
        let ids =
            |found: Option<(&Mod, &Mod)>| found.map(|(m, late)| (m.id.clone(), late.id.clone()));
        assert_eq!(ids(out_of_order(&[&lib, &map])), None);
        assert_eq!(
            ids(out_of_order(&[&map, &lib])),
            Some(("map".into(), "lib".into()))
        );
        // What it requires isn't in the list: no order can fix that.
        assert_eq!(ids(out_of_order(&[&map])), None);
        assert_eq!(ids(out_of_order(&[])), None);
    }

    #[test]
    fn set_order_only_moves_what_is_active() {
        let dir = temp("order");
        let instance = instance(&dir);
        let list = |mods: &[&str], maps: &[&str]| Active {
            mods: mods.iter().map(|id| id.to_string()).collect(),
            maps: maps.iter().map(|map| map.to_string()).collect(),
        };
        activate(&instance, &list(&["a", "b", "c"], &["West", "East"])).unwrap();
        // `x` isn't active and `b` wasn't listed: one is left out, the other kept at the end.
        let order = [
            "c".to_owned(),
            "x".to_owned(),
            "a".to_owned(),
            "c".to_owned(),
        ];
        set_order(&instance, &order, None).unwrap();
        assert_eq!(active(&instance), list(&["c", "a", "b"], &["West", "East"]));
        set_order(
            &instance,
            &[],
            Some(&["East".to_owned(), "West".to_owned()]),
        )
        .unwrap();
        assert_eq!(active(&instance), list(&["c", "a", "b"], &["East", "West"]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_and_writes_the_active_list_in_both_formats() {
        let active = Active::parse(DEFAULT);
        assert_eq!(active.mods, ["FloorAttackHighlightFix"]);
        assert_eq!(active.write(true), DEFAULT, "Build 42 round trip");

        let old = "VERSION = 1,\n\nmods\n{\n    mod = A,\n    mod = B,\n}\n\nmaps\n{\n    map = Town,\n}\n";
        let active = Active::parse(old);
        assert_eq!(
            (active.mods.len(), active.maps.as_slice()),
            (2, &["Town".to_owned()][..])
        );
        assert_eq!(active.write(false), old, "Build 41 round trip");
        assert_eq!(
            Active::parse("").write(true),
            "VERSION = 1,\n\nmods\n{\n}\n\nmaps\n{\n}\n"
        );
    }

    #[test]
    fn picks_the_folder_the_game_would() {
        let dir = temp("folders");
        let game = |major| Some(Version { build: 42, major });
        for folder in ["42", "42.13", "42.22"] {
            put(&dir, &format!("multi/{folder}/mod.info"), "id=multi");
        }
        assert_eq!(
            info_dir(&dir.join("multi"), true, game(19)),
            Some(dir.join("multi/42.13"))
        );
        assert_eq!(
            info_dir(&dir.join("multi"), true, game(22)),
            Some(dir.join("multi/42.22"))
        );
        assert_eq!(
            info_dir(&dir.join("multi"), true, None),
            Some(dir.join("multi/42.22"))
        );

        // Only a newer folder: this game can't load it.
        put(&dir, "newer/42.21/mod.info", "id=newer");
        assert_eq!(info_dir(&dir.join("newer"), true, game(19)), None);

        // `mod.info` in common, files in a version folder.
        put(&dir, "shared/common/mod.info", "id=shared");
        put(&dir, "shared/42/media/x.lua", "");
        assert_eq!(
            info_dir(&dir.join("shared"), true, game(19)),
            Some(dir.join("shared/common"))
        );

        // Build 41 layout: only Build 41 sees it.
        put(&dir, "old/mod.info", "id=old");
        assert_eq!(info_dir(&dir.join("old"), true, game(19)), None);
        assert_eq!(
            info_dir(&dir.join("old"), false, None),
            Some(dir.join("old"))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn finds_mods_made_for_a_newer_game() {
        let dir = temp("too-new");
        let instance = instance(&dir);
        put(&dir, "zomboid/mods/Newer/42.21/mod.info", "id=Newer");
        // Before the first run the version is unknown: nothing to compare.
        assert!(too_new(&instance).is_empty());
        put(&dir, "zomboid/version.txt", "42.19.2 99198cd6ef\n");
        put(&dir, "zomboid/mods/Fits/42/mod.info", "id=Fits");
        put(
            &dir,
            "zomboid/mods/Shiny/42.22/mod.info",
            "id=Shiny\nname=Shiny Cars",
        );
        // Build 41's layout is the other build line, not a newer game.
        put(&dir, "zomboid/mods/Old/mod.info", "id=Old");
        put(&dir, "zomboid/mods/NotAMod/readme.txt", "");
        assert_eq!(too_new(&instance), ["Newer", "Shiny Cars"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keeps_when_a_mod_came_and_when_it_last_changed() {
        let dir = temp("dates");
        let instance = instance(&dir);
        put(&dir, "zomboid/version.txt", "42.19.2 99198cd6ef\n");
        let item = steam::workshop::Item {
            id: 7,
            app: APP,
            title: "seven".into(),
            author: 0,
            summary: String::new(),
            manifest: 70,
            file_url: None,
            size: 0,
            updated: None,
            tags: Vec::new(),
            preview: None,
            requires: Vec::new(),
            subscriptions: 0,
            file_type: 0,
        };
        let bring = || {
            put(&dir, ".incoming/7/mods/Seven/42/mod.info", "id=Seven");
            land(&instance, &item, &dir.join(".incoming/7")).unwrap();
            read_pins(&instance).items.remove(0)
        };
        let first = bring();
        assert!(first.added.is_some());
        assert_eq!(first.added, first.updated);
        // Updated later: the day it came stays.
        let mut pins = read_pins(&instance);
        pins.items[0].added = Some(5);
        pins.items[0].updated = Some(5);
        write_pins(&instance, &pins).unwrap();
        let again = bring();
        assert_eq!(again.added, Some(5));
        assert!(again.updated > Some(5));
        // A mod lata didn't install goes by its folder's date.
        put(&dir, "zomboid/mods/Loose/42/mod.info", "id=Loose");
        let loose = list(&instance)
            .into_iter()
            .find(|m| m.id == "Loose")
            .unwrap();
        assert!(loose.added.is_some() && loose.added == loose.updated);
        let seven = list(&instance)
            .into_iter()
            .find(|m| m.id == "Seven")
            .unwrap();
        assert_eq!(seven.added, Some(UNIX_EPOCH + Duration::from_secs(5)));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// An item can ship a folder named like another item's. The folder then holds the newcomer's
    /// files, so the first item must stop owning it: the Java loader is hooked by its folder.
    #[test]
    fn a_folder_belongs_to_the_item_that_brought_it_last() {
        let dir = temp("owner");
        let instance = instance(&dir);
        let item = |id: u64| steam::workshop::Item {
            id,
            app: APP,
            title: format!("item {id}"),
            author: 0,
            summary: String::new(),
            manifest: id,
            file_url: None,
            size: 0,
            updated: None,
            tags: Vec::new(),
            preview: None,
            requires: Vec::new(),
            subscriptions: 0,
            file_type: 0,
        };
        let bring = |id: u64, jar: &str| {
            let staging = dir.join(format!(".incoming/{id}"));
            put(
                &dir,
                &format!(".incoming/{id}/mods/ZombieBuddy/42/mod.info"),
                "id=ZombieBuddy",
            );
            put(
                &dir,
                &format!(".incoming/{id}/mods/ZombieBuddy/libs/ZombieBuddy.jar"),
                jar,
            );
            land(&instance, &item(id), &staging).unwrap();
        };
        let loader = super::super::java::LOADER;
        bring(loader, "the real one");
        std::fs::write(
            dir.join("instance.json"),
            r#"{"name":"x","branch":"public","created":0,"java_mods":true}"#,
        )
        .unwrap();
        assert!(super::super::java::on(&instance));

        bring(999, "someone else's");
        let folders = |id: u64| {
            let pins = read_pins(&instance).items;
            pins.into_iter().find(|p| p.id == id).unwrap().folders
        };
        assert!(
            folders(loader).is_empty(),
            "the loader no longer owns the folder"
        );
        assert_eq!(folders(999), ["ZombieBuddy"]);
        assert!(
            !super::super::java::on(&instance),
            "a stranger's jar is never hooked"
        );

        // The same with a name that differs only in case, which is the same folder on
        // Windows and macOS.
        bring(loader, "the real one");
        let staging = dir.join(".incoming/998");
        put(&dir, ".incoming/998/mods/zombiebuddy/42/mod.info", "id=zb");
        land(&instance, &item(998), &staging).unwrap();
        assert!(folders(loader).is_empty());
        assert!(!super::super::java::on(&instance));

        // The real item again (an update, or installed again) takes the folder back.
        bring(loader, "the real one");
        assert!(super::super::java::on(&instance));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn holds_a_version_and_undoes_an_update() {
        let dir = temp("undo");
        let instance = instance(&dir);
        let pin = |id, manifest| Pin {
            id,
            manifest,
            title: format!("mod {id}"),
            folders: Vec::new(),
            held: false,
            added: None,
            updated: None,
        };
        let items = vec![pin(1, 10), pin(2, 20), pin(3, 30)];
        write_pins(&instance, &Pins { items }).unwrap();
        assert!(undo(&instance).is_empty(), "nothing was updated yet");

        // An update of 1 and 2 is about to run; then 1 really moves and 2 stays.
        remember(&instance, &[1, 2]).unwrap();
        assert!(undo(&instance).is_empty(), "nothing changed yet");
        let mut pins = read_pins(&instance);
        pins.items[0].manifest = 11;
        write_pins(&instance, &pins).unwrap();
        assert_eq!(undo(&instance), [(1, 10)]);
        // Only some of it undone: the rest stays to undo later.
        pins.items[1].manifest = 21;
        write_pins(&instance, &pins).unwrap();
        pins.items[0].manifest = 10;
        write_pins(&instance, &pins).unwrap();
        forget(&instance);
        assert_eq!(undo(&instance), [(2, 20)]);
        pins.items[1].manifest = 20;
        write_pins(&instance, &pins).unwrap();
        forget(&instance);
        assert!(!instance.dir.join(UNDO).exists());

        hold(&instance, &[2], true).unwrap();
        let held: Vec<bool> = pins_of(&instance).iter().map(|p| p.held).collect();
        assert_eq!(held, [false, true, false]);
        // Packs and older latas read the file too: an item that isn't held has no extra key.
        let text = std::fs::read_to_string(instance.dir.join("mods.json")).unwrap();
        assert_eq!(text.matches("held").count(), 1);
        hold(&instance, &[2], false).unwrap();
        assert!(pins_of(&instance).iter().all(|p| !p.held));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn pins_of(instance: &Instance) -> Vec<Pin> {
        read_pins(instance).items
    }

    #[test]
    fn lists_enables_and_removes_mods() {
        let dir = temp("list");
        let instance = instance(&dir);
        put(&dir, "zomboid/version.txt", "42.19.2 99198cd6ef\n");
        put(
            &dir,
            "zomboid/mods/FloorAttackHighlightFix/42/mod.info",
            INFO,
        );
        put(
            &dir,
            "zomboid/mods/FloorAttackHighlightFix/42/icon.png",
            "png",
        );
        put(
            &dir,
            "zomboid/mods/Lib/42/mod.info",
            "id=Lib\nname=A library",
        );
        put(
            &dir,
            "zomboid/mods/Needy/42/mod.info",
            "id=Needy\nname=Needy\nrequire=\\Lib,\\Gone",
        );
        put(&dir, "zomboid/mods/examplemod/42/mod.info", "id=examplemod");
        put(&dir, "zomboid/mods/TooNew/42.21/mod.info", "id=TooNew");

        let names: Vec<_> = list(&instance).into_iter().map(|m| m.id).collect();
        assert_eq!(names, ["Lib", "FloorAttackHighlightFix", "Needy"]);
        assert!(list(&instance).iter().all(|m| !m.enabled));

        // Turning one on writes the list and the file that keeps the game from emptying it.
        assert!(
            set_enabled(&instance, "FloorAttackHighlightFix", true)
                .unwrap()
                .is_empty()
        );
        let mods = dir.join("zomboid/mods");
        assert_eq!(
            std::fs::read_to_string(mods.join("default.txt")).unwrap(),
            DEFAULT
        );
        assert!(mods.join("reset-mods-42_00.txt").is_file());
        let fix = list(&instance)
            .into_iter()
            .find(|m| m.id == "FloorAttackHighlightFix")
            .unwrap();
        assert!(fix.enabled);
        assert_eq!((fix.version.as_deref(), fix.workshop), (Some("42"), None));
        assert_eq!(
            fix.icon,
            Some(mods.join("FloorAttackHighlightFix/42/icon.png"))
        );

        // What it requires comes on with it, before it; what isn't installed is reported.
        assert_eq!(set_enabled(&instance, "Needy", true).unwrap(), ["Gone"]);
        assert_eq!(
            read_active(&instance).mods,
            ["FloorAttackHighlightFix", "Lib", "Needy"]
        );

        set_enabled(&instance, "Lib", false).unwrap();
        assert_eq!(
            read_active(&instance).mods,
            ["FloorAttackHighlightFix", "Needy"]
        );

        remove(&instance, "Needy").unwrap();
        assert!(!mods.join("Needy").exists());
        assert_eq!(read_active(&instance).mods, ["FloorAttackHighlightFix"]);
        assert!(remove(&instance, "../zomboid").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn adopts_a_downloaded_item_replacing_the_old_copy() {
        let dir = temp("adopt");
        let instance = instance(&dir);
        put(&dir, "zomboid/mods/One/42/mod.info", "id=One\nname=Old");
        put(&dir, ".incoming/7/mods/One/42/mod.info", "id=One\nname=New");
        put(&dir, ".incoming/7/mods/Two/42/mod.info", "id=Two");
        assert_eq!(
            adopt(&instance, &dir.join(".incoming/7")).unwrap(),
            ["One", "Two"]
        );
        let names: Vec<_> = list(&instance).into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["New", "Two"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_update_keeps_mods_on_or_off_and_drops_old_folders() {
        let dir = temp("update");
        let instance = instance(&dir);
        let item = |manifest| steam::workshop::Item {
            id: 7,
            app: APP,
            title: "Seven".into(),
            author: 0,
            summary: String::new(),
            manifest,
            file_url: None,
            size: 1,
            updated: None,
            tags: Vec::new(),
            preview: None,
            requires: Vec::new(),
            subscriptions: 0,
            file_type: 0,
        };
        put(&dir, ".incoming/7/mods/One/42/mod.info", "id=One");
        put(&dir, ".incoming/7/mods/Old/42/mod.info", "id=Old");
        land(&instance, &item(1), &dir.join(".incoming/7")).unwrap();
        set_enabled(&instance, "One", false).unwrap();

        put(&dir, ".incoming/7/mods/One/42/mod.info", "id=One");
        put(&dir, ".incoming/7/mods/New/42/mod.info", "id=New");
        land(&instance, &item(2), &dir.join(".incoming/7")).unwrap();
        let mods: Vec<_> = list(&instance)
            .into_iter()
            .map(|m| (m.id, m.enabled, m.manifest))
            .collect();
        assert_eq!(
            mods,
            [
                ("New".to_owned(), true, Some(2)),
                ("One".to_owned(), false, Some(2))
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Downloads a real, small Build 42 mod from Steam and finds it listed and on.
    /// What "Change version" and "Undo update" do end to end: an older version of an item
    /// replaces the installed one, the mod stays off if it was off, and the update is undone.
    #[test]
    #[ignore = "needs network access to Steam"]
    fn goes_back_to_an_older_version_of_a_mod() {
        // Updated often, so it has old versions to go back to.
        const ITEM: u64 = 3833349401;
        let dir = temp("rollback");
        let instance = instance(&dir);
        put(&dir, "zomboid/version.txt", "42.22.2 3d39148846\n");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let get = |exact: Vec<(u64, u64)>| {
            runtime
                .block_on(install(instance.clone(), vec![ITEM], exact, |_, _, _| {}))
                .expect("the item installs")
        };
        let history = runtime
            .block_on(async {
                let http = steam::http();
                let mut client = steam::Client::anonymous(&http).await?;
                steam::workshop::history(&mut client, ITEM).await
            })
            .expect("its versions");
        let (newest, older) = (history[0].manifest, history[1].manifest);
        // It may bring items it requires: its own pin, wherever it is in the list.
        let at = || {
            let pins = read_pins(&instance).items;
            pins.iter().find(|pin| pin.id == ITEM).unwrap().manifest
        };

        get(Vec::new());
        assert_eq!(at(), newest);
        // Off before the change: it has to stay off after.
        let first = list(&instance)
            .into_iter()
            .find(|m| m.workshop == Some(ITEM))
            .unwrap();
        set_enabled(&instance, &first.id, false).unwrap();

        // The update button would now `remember`; here the version is picked by hand.
        let back = get(vec![(ITEM, older)]);
        assert!(back.newer.is_empty(), "Steam still serves it: {back:?}");
        assert_eq!(at(), older);
        assert!(
            !list(&instance)
                .iter()
                .any(|m| m.id == first.id && m.enabled)
        );
        hold(&instance, &[ITEM], true).unwrap();
        let mine = |m: &&Mod| m.workshop == Some(ITEM);
        assert!(list(&instance).iter().filter(mine).all(|m| m.held));

        // Updating again and undoing it lands on `older` once more.
        remember(&instance, &[ITEM]).unwrap();
        get(Vec::new());
        assert_eq!(undo(&instance), [(ITEM, older)]);
        get(undo(&instance));
        assert_eq!(at(), older);
        assert!(undo(&instance).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    #[ignore = "needs network access to Steam"]
    fn installs_a_workshop_mod_into_an_instance() {
        let dir = temp("install");
        let instance = instance(&dir);
        put(&dir, "zomboid/version.txt", "42.22.2 3d39148846\n");
        let run = |id| {
            let seen = std::sync::Arc::new(std::sync::Mutex::new((0, 0)));
            let report = seen.clone();
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(install(
                    instance.clone(),
                    vec![id],
                    Vec::new(),
                    move |_, done, total| {
                        *report.lock().unwrap() = (done, total);
                    },
                ));
            (result, *seen.lock().unwrap())
        };
        let (title, (done, total)) = run(3832274571);
        let title = title.expect("the mod installs");
        assert!(
            total > 0 && done == total,
            "progress reached the end: {done} of {total}"
        );
        assert_eq!(title.title, "floor attack highlight fix");
        assert!(title.with.is_empty(), "it requires nothing: {title:?}");

        let mods = list(&instance);
        assert_eq!(mods.len(), 1, "{mods:?}");
        assert_eq!(read_pins(&instance).items[0].id, 3832274571);
        assert_eq!(mods[0].id, "FloorAttackHighlightFix");
        assert!(mods[0].enabled && mods[0].icon.is_some());
        assert_eq!(mods[0].workshop, Some(3832274571));
        assert_eq!(read_pins(&instance).items[0].id, 3832274571);
        assert_ne!(read_pins(&instance).items[0].manifest, 0);
        assert!(!dir.join(".incoming").exists());

        // "Project Cook" needs a framework its author lists as required: it comes along, and
        // every mod it turned on finds what it needs.
        let (cook, _) = run(3490188370);
        let cook = cook.expect("the mod installs with what it requires");
        assert!(!cook.with.is_empty(), "{cook:?}");
        let installed = list(&instance);
        let ids: Vec<&str> = installed.iter().map(|m| m.id.as_str()).collect();
        for item in installed.iter().filter(|m| m.enabled) {
            for needed in &item.requires {
                assert!(
                    ids.contains(&needed.as_str()),
                    "{} needs {needed}: {ids:?}",
                    item.id
                );
            }
        }

        // A Workshop listing comes with its picture, saved where the dialog shows it from.
        let listing = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            // "Project Cook", whose picture is a PNG. The mod installed above has an animated
            // AVIF, which isn't one of the formats shown, so it gets the placeholder.
            .block_on(lookup(3490188370))
            .expect("the listing is found");
        assert!(
            listing.item.tags.iter().any(|tag| tag == "Build 42"),
            "{listing:?}"
        );
        let picture = listing
            .picture
            .clone()
            .unwrap_or_else(|| panic!("it has a picture: {listing:?}"));
        assert!(
            std::fs::metadata(&picture).unwrap().len() > 1000,
            "{picture:?}"
        );

        // A collection, or anything that isn't a mod, is refused.
        let (not, _) = run(3834236342);
        assert!(not.is_err(), "{not:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
