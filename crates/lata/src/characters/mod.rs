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
    let store = store();
    std::fs::create_dir_all(&store)?;
    write(&store.join(presets.kind().file()), presets)
}

/// Before a launch: adds the shared presets to the instance, replacing its own of the same name,
/// and drops those deleted in lata since its last session. Then notes what it shared, so
/// [`pull`] can tell what changed in game.
// ponytail: a game that ended while lata was closed isn't read back, so its new presets are
// shared only after its next session; read the instance first here if that ever matters.
pub fn push(instance: &Path) -> io::Result<()> {
    share(&store(), &lua(instance))
}

fn share(store: &Path, lua: &Path) -> io::Result<()> {
    for kind in Kind::ALL {
        let Some(shared) = read(&store.join(kind.file()), kind)? else {
            continue;
        };
        let path = lua.join(kind.file());
        let mut here = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
        let mut changed = false;
        // In the note but no longer shared: deleted in lata (on the Characters page, or in
        // another instance's game). Only an untouched copy goes; one changed in game since is
        // new again.
        if let Some(noted) = read(&lua.join(note(kind)), kind)? {
            for (name, value) in noted.iter() {
                if shared.get(name).is_none() && here.get(name) == Some(value) {
                    changed |= here.remove(name);
                }
            }
        }
        changed |= here.merge(&shared);
        std::fs::create_dir_all(lua)?;
        if changed {
            write(&path, &here)?;
        }
        write(&lua.join(note(kind)), &shared)?;
    }
    Ok(())
}

/// After the game exits: shares the presets saved or changed in game, the last one saved
/// winning by name, and drops from the store those the player deleted. A preset the game left
/// as lata shared it isn't copied back, so a change or a delete made in lata meanwhile stays.
// ponytail: two games open at once can put back an older version of a preset the other changed.
pub fn pull(instance: &Path) -> io::Result<()> {
    collect(&lua(instance), &store())
}

fn collect(lua: &Path, store: &Path) -> io::Result<()> {
    for kind in Kind::ALL {
        let noted = read(&lua.join(note(kind)), kind)?;
        let found = read(&lua.join(kind.file()), kind)?;
        if noted.is_none() && found.is_none() {
            continue;
        }
        let here = found.unwrap_or_else(|| Presets::parse(kind, ""));
        let path = store.join(kind.file());
        let mut shared = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
        let mut changed = false;
        if let Some(noted) = &noted {
            for (name, _) in noted.iter() {
                if here.get(name).is_none() {
                    changed |= shared.remove(name);
                }
            }
        }
        for (name, value) in here.iter() {
            let untouched = noted.as_ref().and_then(|n| n.get(name)) == Some(value);
            if !untouched && shared.get(name) != Some(value) {
                shared.set(name, value);
                changed = true;
            }
        }
        if changed {
            std::fs::create_dir_all(store)?;
            write(&path, &shared)?;
        }
        // What the instance holds is now what lata knows it holds, so its next launch can
        // drop the ones deleted in lata meanwhile.
        std::fs::create_dir_all(lua)?;
        write(&lua.join(note(kind)), &here)?;
    }
    Ok(())
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
        Ok(text) => Ok(Some(Presets::parse(kind, &text))),
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
}
