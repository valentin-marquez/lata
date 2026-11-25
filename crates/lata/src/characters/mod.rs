//! Characters shared by every instance: the presets the game's character screen saves, kept
//! in lata's data folder and copied into each instance around a game session.

#[path = "presets.model.rs"]
pub mod presets;

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

/// Before a launch: adds the shared presets to the instance, replacing its own of the same name.
// ponytail: a game that ended while lata was closed isn't read back, so its new presets are
// shared only after its next session; read the instance first here if that ever matters.
pub fn push(instance: &Path) -> io::Result<()> {
    sync(&store(), &lua(instance))
}

/// After the game exits: shares the presets saved in game, the last one saved winning by name.
// ponytail: a preset deleted in game stays in the store and comes back on the next launch;
// propagating deletes needs a record of what was pushed, add it with the editor (#35).
// Two games open at once can also put back an older version of a preset the other changed.
pub fn pull(instance: &Path) -> io::Result<()> {
    sync(&lua(instance), &store())
}

/// Merges both preset files of `from` into those of `to`, writing only what changed.
fn sync(from: &Path, to: &Path) -> io::Result<()> {
    for kind in Kind::ALL {
        let Some(source) = read(&from.join(kind.file()), kind)? else {
            continue;
        };
        let path = to.join(kind.file());
        let mut target = read(&path, kind)?.unwrap_or_else(|| Presets::parse(kind, ""));
        if target.merge(&source) {
            std::fs::create_dir_all(to)?;
            // Write then rename, so a crash never leaves a half-written file.
            let tmp = path.with_extension("txt.tmp");
            std::fs::write(&tmp, target.to_string())?;
            std::fs::rename(&tmp, &path)?;
        }
    }
    Ok(())
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

    #[test]
    fn a_preset_saved_in_one_instance_reaches_the_others() {
        let root = std::env::temp_dir().join(format!("lata-characters-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (store, a, b) = (root.join("store"), root.join("a"), root.join("b"));
        let (lua_a, lua_b) = (lua(&a), lua(&b));

        // B already has a preset of its own, saved before sharing existed.
        std::fs::create_dir_all(&lua_b).unwrap();
        std::fs::write(
            lua_b.join(Kind::Outfits.file()),
            "VERSION=1\nMine:gender=2;\n",
        )
        .unwrap();

        // A is launched with nothing shared yet, and the player saves a look and a build.
        sync(&store, &lua_a).unwrap();
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
        sync(&lua_a, &store).unwrap();

        // B's launch gets both, and keeps its own.
        sync(&store, &lua_b).unwrap();
        assert_eq!(
            std::fs::read_to_string(lua_b.join(Kind::Outfits.file())).unwrap(),
            "VERSION=1\nMine:gender=2;\nKate:gender=1;voiceStyle=Female_1;voicePitch=0.15;\n"
        );
        assert_eq!(
            std::fs::read_to_string(lua_b.join(Kind::Builds.file())).unwrap(),
            "Nurse:nurse;FastLearner;\n"
        );

        // B changes Kate in game; after it exits, A's next launch gets the change.
        std::fs::write(
            lua_b.join(Kind::Outfits.file()),
            "VERSION=1\nKate:gender=1;voicePitch=0.5;\nMine:gender=2;\n",
        )
        .unwrap();
        sync(&lua_b, &store).unwrap();
        sync(&store, &lua_a).unwrap();
        let outfits = std::fs::read_to_string(lua_a.join(Kind::Outfits.file())).unwrap();
        let outfits = Presets::parse(Kind::Outfits, &outfits);
        assert_eq!(outfits.get("Kate"), Some("gender=1;voicePitch=0.5;"));
        assert_eq!(outfits.get("Mine"), Some("gender=2;"));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
