//! Characters shared by every instance: the presets the game's character screen saves, kept
//! in lata's data folder and copied into each instance around a game session.
//!
//! Three writers touch them: each instance's game, and the Characters page. A note next to
//! each instance's files (`saved_outfits.txt.lata`) holds the presets lata and that instance
//! last agreed on, so a sync can tell who changed or deleted what since.

#[path = "catalog.model.rs"]
mod catalog;
#[path = "character.model.rs"]
mod character;
#[path = "editor.view.rs"]
mod editor;
#[path = "page.view.rs"]
mod page;
#[path = "presets.model.rs"]
pub mod presets;
#[path = "preview.view.rs"]
mod preview;

pub use page::Characters;

use std::io;
use std::path::{Path, PathBuf};

use presets::{Kind, Presets};

/// The shared presets, in the game's own two files.
fn store() -> PathBuf {
    crate::instance::store::data().join("characters")
}

/// Where the game keeps an instance's presets.
fn lua(instance: &Path) -> PathBuf {
    instance.join("zomboid").join("Lua")
}

/// The shared presets of `kind`, empty when there are none yet.
pub fn shared(kind: Kind) -> io::Result<Presets> {
    Ok(read(&store().join(kind.file()), kind)?.unwrap_or_else(|| Presets::parse(kind, "")))
}

/// Replaces the shared presets of their kind; instances pick the change up on their next launch.
pub fn keep(presets: &Presets) -> io::Result<()> {
    keep_in(&store(), presets)
}

/// Also notes what this drops (the name and its last value) in the store's list of deleted
/// presets, so an instance with no note of its own (its last session ran before notes were
/// kept, or it never ran) still lets them go.
// ponytail: the list only grows, one line per deleted preset; trim it if that ever shows.
fn keep_in(store: &Path, presets: &Presets) -> io::Result<()> {
    let kind = presets.kind();
    let path = store.join(kind.file());
    let before = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
    let mut gone = deleted(store, kind)?;
    let mut changed = false;
    for (name, value) in before.iter() {
        if presets.get(name).is_none() && gone.get(name) != Some(value) {
            gone.set(name, value);
            changed = true;
        }
    }
    for (name, _) in presets.iter() {
        changed |= gone.remove(name);
    }
    std::fs::create_dir_all(store)?;
    if changed {
        write(&store.join(graveyard(kind)), &gone)?;
    }
    write(&path, presets)
}

/// Presets deleted on the Characters page, with the value each had then.
fn deleted(store: &Path, kind: Kind) -> io::Result<Presets> {
    Ok(read(&store.join(graveyard(kind)), kind)?.unwrap_or_else(|| Presets::parse(kind, "")))
}

fn graveyard(kind: Kind) -> String {
    format!("{}.deleted", kind.file())
}

/// Before a launch: first reads back what the instance's last session left, if lata didn't
/// (the game ended while lata was closed). Then adds the shared presets to the instance,
/// replacing its own of the same name, and drops those deleted in lata since its last session,
/// noting what it shared so [`pull`] can tell what changed in game.
pub fn push(instance: &Path) -> io::Result<()> {
    launch(&store(), &lua(instance))
}

fn launch(store: &Path, lua: &Path) -> io::Result<()> {
    each(|kind| {
        // Only with a note: without one lata can't tell the player's changes from stale copies.
        if lua.join(note(kind)).exists() {
            collect_kind(lua, store, kind)?;
        }
        share_kind(store, lua, kind)
    })
}

/// Runs `sync` for each kind, so one that fails (a file that isn't UTF-8) doesn't stop the
/// other; the first error is returned.
fn each(mut sync: impl FnMut(Kind) -> io::Result<()>) -> io::Result<()> {
    let mut first = Ok(());
    for kind in Kind::ALL {
        if let Err(err) = sync(kind)
            && first.is_ok()
        {
            first = Err(err);
        }
    }
    first
}

#[cfg(test)]
fn share(store: &Path, lua: &Path) -> io::Result<()> {
    each(|kind| share_kind(store, lua, kind))
}

fn share_kind(store: &Path, lua: &Path, kind: Kind) -> io::Result<()> {
    let Some(shared) = read(&store.join(kind.file()), kind)? else {
        return Ok(());
    };
    let path = lua.join(kind.file());
    let mut here = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
    let mut changed = false;
    // In the note but no longer shared: deleted in lata (on the Characters page, or in
    // another instance's game). Only an untouched copy goes; one changed in game since is
    // new again.
    let noted = read(&lua.join(note(kind)), kind)?;
    let gone = deleted(store, kind)?;
    for list in noted.iter().chain([&gone]) {
        for (name, value) in list.iter() {
            if shared.get(name).is_none() && here.get(name) == Some(value) {
                changed |= here.remove(name);
            }
        }
    }
    if noted.is_none() {
        // The instance's first share: a preset it kept from before, under a name another
        // instance shared differently, stays under a free name instead of being replaced.
        let clashes: Vec<(String, String)> = here
            .iter()
            .filter(|(name, value)| shared.get(name).is_some_and(|other| other != *value))
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect();
        for (name, value) in clashes {
            let free = (2..)
                .map(|n| format!("{name} {n}"))
                .find(|c| here.get(c).is_none() && shared.get(c).is_none())
                .unwrap_or_default();
            here.remove(&name);
            here.set(&free, &value);
            changed = true;
        }
    }
    changed |= here.merge(&shared);
    std::fs::create_dir_all(lua)?;
    if changed {
        write(&path, &here)?;
    }
    write(&lua.join(note(kind)), &shared)
}

/// After the game exits: shares the presets saved or changed in game, the last one saved
/// winning by name, and drops from the store those the player deleted. A preset the game left
/// as lata shared it isn't copied back, so a change or a delete made in lata meanwhile stays.
// ponytail: two games open at once can put back an older version of a preset the other changed.
pub fn pull(instance: &Path) -> io::Result<()> {
    collect(&lua(instance), &store())
}

fn collect(lua: &Path, store: &Path) -> io::Result<()> {
    each(|kind| collect_kind(lua, store, kind))
}

fn collect_kind(lua: &Path, store: &Path, kind: Kind) -> io::Result<()> {
    let noted = read(&lua.join(note(kind)), kind)?;
    let found = read(&lua.join(kind.file()), kind)?;
    if noted.is_none() && found.is_none() {
        return Ok(());
    }
    let here = found.unwrap_or_else(|| Presets::parse(kind, ""));
    let path = store.join(kind.file());
    let mut shared = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
    let mut gone = deleted(store, kind)?;
    let (mut changed, mut buried) = (false, false);
    // A file with no presets left (the game crashed while saving it, or a format lata doesn't
    // read) says nothing about what the player deleted, and taking it at its word would
    // delete every shared preset.
    // ponytail: so deleting every preset in game doesn't share; it's the rare case.
    let readable = here.iter().next().is_some();
    if let Some(noted) = noted.as_ref().filter(|_| readable) {
        for (name, value) in noted.iter() {
            // Only the copy lata shared goes: one changed elsewhere since is newer than the
            // delete. It goes to the deleted list too, for instances that have no note.
            if here.get(name).is_none() && shared.get(name) == Some(value) {
                shared.remove(name);
                gone.set(name, value);
                (changed, buried) = (true, true);
            }
        }
    }
    for (name, value) in here.iter() {
        // As lata shared it, or as it was when deleted on the Characters page.
        let untouched = noted.as_ref().and_then(|n| n.get(name)) == Some(value)
            || gone.get(name) == Some(value);
        if !untouched && shared.get(name) != Some(value) {
            shared.set(name, value);
            changed = true;
            buried |= gone.remove(name);
        }
    }
    if changed || buried {
        std::fs::create_dir_all(store)?;
    }
    if buried {
        write(&store.join(graveyard(kind)), &gone)?;
    }
    if changed {
        write(&path, &shared)?;
    }
    // What the instance holds is now what lata knows it holds, so its next launch can
    // drop the ones deleted in lata meanwhile.
    std::fs::create_dir_all(lua)?;
    write(&lua.join(note(kind)), &here)
}

/// Next to a preset file, the presets lata and the instance last agreed on, in the same format.
fn note(kind: Kind) -> String {
    format!("{}.lata", kind.file())
}

/// Writes then renames, so a crash never leaves a half-written file.
fn write(path: &Path, presets: &Presets) -> io::Result<()> {
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    std::fs::write(&tmp, presets.to_string())?;
    std::fs::rename(&tmp, path)
}

/// A preset file, `None` if there isn't one. A file that isn't UTF-8 is an error, so it is
/// never rewritten from a lossy copy.
fn read(path: &Path, kind: Kind) -> io::Result<Option<Presets>> {
    match std::fs::read_to_string(path) {
        // Some editors put a byte order mark first; the game's own files have none.
        Ok(text) => Ok(Some(Presets::parse(
            kind,
            text.strip_prefix('\u{feff}').unwrap_or(&text),
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("lata-characters-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        root
    }

    fn get(dir: &Path, kind: Kind, name: &str) -> Option<String> {
        let presets = read(&dir.join(kind.file()), kind).unwrap()?;
        presets.get(name).map(str::to_owned)
    }

    #[test]
    fn a_preset_saved_in_one_instance_reaches_the_others() {
        let root = scratch("reach");
        let (store, lua_a, lua_b) = (
            root.join("store"),
            lua(&root.join("a")),
            lua(&root.join("b")),
        );

        // B already has a preset of its own, saved before sharing existed.
        std::fs::create_dir_all(&lua_b).unwrap();
        std::fs::write(
            lua_b.join(Kind::Outfits.file()),
            "VERSION=1\nMine:gender=2;\n",
        )
        .unwrap();

        // A is launched with nothing shared yet, and the player saves a look and a build.
        share(&store, &lua_a).unwrap();
        assert!(!lua_a.exists(), "nothing to write");
        std::fs::create_dir_all(&lua_a).unwrap();
        std::fs::write(
            lua_a.join(Kind::Outfits.file()),
            "VERSION=1\nKate:gender=1;voiceStyle=Female_1;voicePitch=0.15;\n",
        )
        .unwrap();
        std::fs::write(
            lua_a.join(Kind::Builds.file()),
            "Nurse:nurse;FastLearner;\n",
        )
        .unwrap();
        collect(&lua_a, &store).unwrap();

        // B's launch gets both, and keeps its own; after it exits, its own is shared too.
        share(&store, &lua_b).unwrap();
        assert_eq!(
            std::fs::read_to_string(lua_b.join(Kind::Outfits.file())).unwrap(),
            "VERSION=1\nMine:gender=2;\nKate:gender=1;voiceStyle=Female_1;voicePitch=0.15;\n"
        );
        assert_eq!(
            get(&lua_b, Kind::Builds, "Nurse").as_deref(),
            Some("nurse;FastLearner;")
        );

        // B changes Kate in game; after it exits, A's next launch gets the change.
        std::fs::write(
            lua_b.join(Kind::Outfits.file()),
            "VERSION=1\nKate:gender=1;voicePitch=0.5;\nMine:gender=2;\n",
        )
        .unwrap();
        collect(&lua_b, &store).unwrap();
        share(&store, &lua_a).unwrap();
        assert_eq!(
            get(&lua_a, Kind::Outfits, "Kate").as_deref(),
            Some("gender=1;voicePitch=0.5;")
        );
        assert_eq!(
            get(&lua_a, Kind::Outfits, "Mine").as_deref(),
            Some("gender=2;")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_preset_deleted_in_game_stays_deleted() {
        let root = scratch("del");
        let (store, lua_a) = (root.join("store"), lua(&root.join("a")));
        std::fs::create_dir_all(&store).unwrap();
        let outfits = Kind::Outfits.file();
        std::fs::write(
            store.join(outfits),
            "VERSION=1\nKate:gender=1;\nBob:gender=0;\n",
        )
        .unwrap();

        // Launch: both reach the instance, and lata notes it shared them.
        share(&store, &lua_a).unwrap();
        // In game the player deletes Bob and saves Ann.
        std::fs::write(
            lua_a.join(outfits),
            "VERSION=1\nKate:gender=1;\nAnn:gender=1;\n",
        )
        .unwrap();
        collect(&lua_a, &store).unwrap();

        assert_eq!(get(&store, Kind::Outfits, "Bob"), None, "deleted in game");
        assert_eq!(
            get(&store, Kind::Outfits, "Kate").as_deref(),
            Some("gender=1;")
        );
        assert_eq!(
            get(&store, Kind::Outfits, "Ann").as_deref(),
            Some("gender=1;")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// The Characters page is the third writer: what it deletes or changes, while a game runs
    /// or between sessions, must not come back from an instance that still has the old copy.
    #[test]
    fn a_preset_changed_or_deleted_in_lata_stays_that_way() {
        let root = scratch("lata");
        let (store, lua_a, lua_b) = (
            root.join("store"),
            lua(&root.join("a")),
            lua(&root.join("b")),
        );
        std::fs::create_dir_all(&store).unwrap();
        let builds = Kind::Builds.file();
        std::fs::write(store.join(builds), "Bob:nurse;\nKate:chef;\nAnn:burglar;\n").unwrap();
        share(&store, &lua_a).unwrap();
        share(&store, &lua_b).unwrap();
        collect(&lua_b, &store).unwrap();

        // While A's game runs, the page deletes Bob and changes Kate.
        let mut edited = Presets::parse(Kind::Builds, "Kate:doctor;\nAnn:burglar;\n");
        write(&store.join(builds), &edited).unwrap();
        // A's game saves a new one, then exits: Bob and the old Kate don't come back.
        std::fs::write(
            lua_a.join(builds),
            "Bob:nurse;\nKate:chef;\nAnn:burglar;\nZed:carpenter;\n",
        )
        .unwrap();
        collect(&lua_a, &store).unwrap();
        assert_eq!(get(&store, Kind::Builds, "Bob"), None);
        assert_eq!(
            get(&store, Kind::Builds, "Kate").as_deref(),
            Some("doctor;")
        );
        assert_eq!(
            get(&store, Kind::Builds, "Zed").as_deref(),
            Some("carpenter;")
        );

        // Between sessions the page deletes Ann. Each instance's next launch drops Bob and
        // Ann, takes the new Kate, and an exit after that brings none of it back.
        edited = read(&store.join(builds), Kind::Builds).unwrap().unwrap();
        edited.remove("Ann");
        write(&store.join(builds), &edited).unwrap();
        for lua in [&lua_a, &lua_b] {
            share(&store, lua).unwrap();
            assert_eq!(get(lua, Kind::Builds, "Bob"), None);
            assert_eq!(get(lua, Kind::Builds, "Ann"), None);
            assert_eq!(get(lua, Kind::Builds, "Kate").as_deref(), Some("doctor;"));
            collect(lua, &store).unwrap();
        }
        let left = read(&store.join(builds), Kind::Builds).unwrap().unwrap();
        let names: Vec<_> = left.iter().map(|(name, _)| name).collect();
        assert_eq!(names, ["Kate", "Zed"]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// An instance whose last session ended before notes were kept has a copy lata can't
    /// place: what the page deleted must still go from it, and not come back.
    #[test]
    fn a_preset_deleted_in_lata_leaves_an_instance_without_a_note() {
        let root = scratch("old");
        let (store, lua_a) = (root.join("store"), lua(&root.join("a")));
        let builds = Kind::Builds.file();
        std::fs::create_dir_all(&store).unwrap();
        std::fs::create_dir_all(&lua_a).unwrap();
        std::fs::write(store.join(builds), "Bob:nurse;\nKate:chef;\n").unwrap();
        std::fs::write(lua_a.join(builds), "Bob:nurse;\nKate:chef;\n").unwrap();

        keep_in(&store, &Presets::parse(Kind::Builds, "Kate:chef;\n")).unwrap();
        share(&store, &lua_a).unwrap();
        assert_eq!(get(&lua_a, Kind::Builds, "Bob"), None);
        collect(&lua_a, &store).unwrap();
        assert_eq!(get(&store, Kind::Builds, "Bob"), None);

        // Made again, it is shared again.
        keep_in(
            &store,
            &Presets::parse(Kind::Builds, "Kate:chef;\nBob:nurse;\n"),
        )
        .unwrap();
        share(&store, &lua_a).unwrap();
        assert_eq!(get(&lua_a, Kind::Builds, "Bob").as_deref(), Some("nurse;"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    fn put(dir: &Path, kind: Kind, text: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(kind.file()), text).unwrap();
    }

    /// The player usually closes lata after pressing Play, so the game ends unseen; the next
    /// launch reads it back before sharing.
    #[test]
    fn a_game_that_ended_while_lata_was_closed_keeps_its_changes() {
        let root = scratch("closed");
        let (store, a) = (root.join("store"), root.join("a"));
        put(
            &store,
            Kind::Outfits,
            "VERSION=1\nKate:gender=1;\nBob:gender=0;\n",
        );
        share(&store, &lua(&a)).unwrap();
        put(&lua(&a), Kind::Outfits, "VERSION=1\nKate:gender=2;\n");

        launch(&store, &lua(&a)).unwrap();
        assert_eq!(
            get(&lua(&a), Kind::Outfits, "Kate").as_deref(),
            Some("gender=2;")
        );
        assert_eq!(get(&lua(&a), Kind::Outfits, "Bob"), None);
        assert_eq!(
            get(&store, Kind::Outfits, "Kate").as_deref(),
            Some("gender=2;")
        );
        assert_eq!(get(&store, Kind::Outfits, "Bob"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// Upgrading with several instances that each saved their own "Kate": the second one's
    /// first launch keeps its Kate under another name.
    #[test]
    fn a_first_share_keeps_the_instances_own_preset_of_a_taken_name() {
        let root = scratch("clash");
        let (store, c) = (root.join("store"), lua(&root.join("c")));
        put(&store, Kind::Builds, "Kate:chef;\n");
        put(&c, Kind::Builds, "Kate:nurse;\nSame:chef;\n");
        share(&store, &c).unwrap();
        assert_eq!(get(&c, Kind::Builds, "Kate").as_deref(), Some("chef;"));
        assert_eq!(get(&c, Kind::Builds, "Kate 2").as_deref(), Some("nurse;"));
        collect(&c, &store).unwrap();
        assert_eq!(
            get(&store, Kind::Builds, "Kate 2").as_deref(),
            Some("nurse;")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_delete_in_game_leaves_a_newer_edit_made_elsewhere() {
        let root = scratch("newer");
        let (store, a) = (root.join("store"), lua(&root.join("a")));
        put(&store, Kind::Builds, "Kate:chef;\n");
        share(&store, &a).unwrap();
        put(&store, Kind::Builds, "Kate:doctor;\n");
        put(&a, Kind::Builds, "Other:nurse;\n");
        collect(&a, &store).unwrap();
        assert_eq!(
            get(&store, Kind::Builds, "Kate").as_deref(),
            Some("doctor;")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_delete_in_game_also_leaves_instances_without_a_note() {
        let root = scratch("bury");
        let (store, a, c) = (
            root.join("store"),
            lua(&root.join("a")),
            lua(&root.join("c")),
        );
        put(&store, Kind::Builds, "Kate:chef;\nAnn:nurse;\n");
        put(&c, Kind::Builds, "Kate:chef;\n");
        share(&store, &a).unwrap();
        put(&a, Kind::Builds, "Ann:nurse;\n");
        collect(&a, &store).unwrap();
        share(&store, &c).unwrap();
        assert_eq!(get(&c, Kind::Builds, "Kate"), None);
        collect(&c, &store).unwrap();
        assert_eq!(get(&store, Kind::Builds, "Kate"), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A save cut short by a crash, or a file lata can't read, deletes nothing.
    #[test]
    fn an_empty_or_unknown_file_deletes_nothing() {
        for (name, text) in [("empty", ""), ("v2", "VERSION=2\nKate:gender=1;\n")] {
            let root = scratch(name);
            let (store, a) = (root.join("store"), lua(&root.join("a")));
            put(&store, Kind::Outfits, "VERSION=1\nKate:gender=1;\n");
            share(&store, &a).unwrap();
            put(&a, Kind::Outfits, text);
            collect(&a, &store).unwrap();
            assert_eq!(
                get(&store, Kind::Outfits, "Kate").as_deref(),
                Some("gender=1;")
            );
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn reads_a_file_with_a_byte_order_mark() {
        let root = scratch("bom");
        put(&root, Kind::Outfits, "\u{feff}VERSION=1\nKate:gender=1;\n");
        assert_eq!(
            get(&root, Kind::Outfits, "Kate").as_deref(),
            Some("gender=1;")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// B41 on Windows may write its files in the system code page: the builds still sync.
    #[test]
    fn one_unreadable_kind_doesnt_stop_the_other() {
        let root = scratch("latin1");
        let (store, a) = (root.join("store"), lua(&root.join("a")));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::write(
            a.join(Kind::Outfits.file()),
            b"VERSION=1\nJos\xe9:gender=1;\n",
        )
        .unwrap();
        put(&a, Kind::Builds, "Kate:chef;\n");
        assert!(collect(&a, &store).is_err());
        assert_eq!(get(&store, Kind::Builds, "Kate").as_deref(), Some("chef;"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
