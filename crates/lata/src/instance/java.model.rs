//! Java mods: mods that ship a `.jar` and change the game's own code, which Lua mods can't.
//! The game doesn't load them; a loader does, hooked into the Java VM when the game starts.
//! Its own install does that through Steam's launch options and the Steam copy of the game,
//! neither of which lata uses, so lata hooks it itself, per instance and only when asked: a
//! Java mod can do anything on the computer.
//!
//! lata never takes a launch argument from a mod. It only builds the path of a file it
//! downloaded from Steam itself, for a loader it knows by its Workshop item (anyone can name a
//! mod "ZombieBuddy"; the item id can't be faked).

use std::io;
use std::path::PathBuf;

use super::mods::{self, Mod};
use super::{Instance, store};

/// ZombieBuddy, the loader Build 42's Java mods are written for.
pub const LOADER: u64 = 3619862853;
const JAR: &str = "ZombieBuddy.jar";
/// Windows can't load a Java agent from a jar the way the others do; this starts it there.
#[cfg_attr(not(windows), expect(dead_code, reason = "copied on Windows only"))]
const NATIVE: &str = "zbNative.dll";

/// What an instance's mods say about Java mods.
#[derive(Debug, Clone, PartialEq)]
pub enum State {
    /// No Java mods here.
    None,
    /// These mods are Java mods, and the loader isn't installed.
    NeedsLoader(Vec<String>),
    /// The loader is here and the instance hasn't been told to hook it.
    Off,
    On,
}

/// The loader's `libs` folder in the instance, when lata installed it from its Workshop item.
fn libs(instance: &Instance) -> Option<PathBuf> {
    let pin = mods::pins(instance).into_iter().find(|p| p.id == LOADER)?;
    pin.folders
        .iter()
        .map(|folder| mods::mods_dir(instance).join(folder).join("libs"))
        .find(|libs| libs.join(JAR).is_file())
}

/// Whether the instance's own Java arguments already hook an agent: someone set it up by
/// hand, and lata stays out of the way.
fn by_hand(instance: &Instance) -> bool {
    let typed = &instance.jvm_args;
    typed.contains("-javaagent") || typed.contains("-agentlib")
}

pub fn state(instance: &Instance, installed: &[Mod]) -> State {
    if by_hand(instance) {
        return State::None;
    }
    if libs(instance).is_some() {
        return match store::java_mods(&instance.dir) {
            true => State::On,
            false => State::Off,
        };
    }
    let mut java: Vec<String> = installed
        .iter()
        .filter(|m| m.java)
        .map(|m| m.name.clone())
        .collect();
    java.dedup();
    match java.is_empty() {
        true => State::None,
        false => State::NeedsLoader(java),
    }
}

/// Whether the loader is to be hooked at launch: turned on, installed, and not already hooked
/// by hand in the instance's own Java arguments.
fn wanted(instance: &Instance) -> Option<PathBuf> {
    if by_hand(instance) {
        return None;
    }
    libs(instance).filter(|_| store::java_mods(&instance.dir))
}

/// Whether the instance has the loader, so Java mods can be turned on or off (and nobody
/// hooked one by hand).
pub fn ready(instance: &Instance) -> bool {
    !by_hand(instance) && libs(instance).is_some()
}

/// Whether the game starts with the loader hooked in for `instance`.
pub fn on(instance: &Instance) -> bool {
    wanted(instance).is_some()
}

/// The Java VM argument that hooks the loader, for a launch of `instance`.
pub fn arg(instance: &Instance) -> Option<String> {
    let libs = wanted(instance)?;
    if cfg!(windows) {
        // Found next to the game by name; see `prepare`.
        Some("-agentlib:zbNative".into())
    } else {
        Some(format!("-javaagent:{}", libs.join(JAR).display()))
    }
}

/// What has to be in place before the game starts. On Windows the loader's two files have to
/// sit next to the game (the VM finds the native one by name there, and it loads the jar from
/// beside itself); elsewhere the jar is used where it is.
pub fn prepare(instance: &Instance) -> io::Result<()> {
    #[cfg(windows)]
    if let (Some(libs), Some(game)) = (wanted(instance), instance.game()) {
        for file in [JAR, NATIVE] {
            let (from, to) = (libs.join(file), game.join(file));
            // Copied again only when the mod's copy changed (an update of the mod).
            let stamp = |path: &std::path::Path| {
                let meta = std::fs::metadata(path).ok()?;
                Some((meta.len(), meta.modified().ok()?))
            };
            let same = match (stamp(&from), stamp(&to)) {
                (Some((size, changed)), Some((copy, copied))) => size == copy && copied >= changed,
                _ => false,
            };
            if !same {
                std::fs::copy(&from, &to)?;
            }
        }
    }
    #[cfg(not(windows))]
    let _ = instance;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(dir: &std::path::Path, file: &str, text: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn hooks_the_loader_only_when_turned_on_and_known_by_its_item() {
        let root = std::env::temp_dir().join(format!("lata-java-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let mut instance = store::create(&root, "Java", "public").unwrap();
        let dir = instance.dir.clone();
        let list = |instance: &Instance| mods::list(instance);

        // A Java mod with no loader: it says what it needs.
        put(
            &dir,
            "zomboid/mods/Fast/42/mod.info",
            "id=Fast\nname=Fast\njavaJarFile=a.jar",
        );
        assert_eq!(
            state(&instance, &list(&instance)),
            State::NeedsLoader(vec!["Fast".into()])
        );
        assert_eq!(arg(&instance), None);

        // A mod that only calls itself ZombieBuddy is not the loader: no pin of the item.
        put(
            &dir,
            "zomboid/mods/ZombieBuddy/42/mod.info",
            "id=ZombieBuddy\nname=ZombieBuddy",
        );
        put(&dir, "zomboid/mods/ZombieBuddy/libs/ZombieBuddy.jar", "jar");
        assert!(matches!(
            state(&instance, &list(&instance)),
            State::NeedsLoader(_)
        ));
        store::set_java_mods(&dir, true).unwrap();
        assert_eq!(
            arg(&instance),
            None,
            "turned on, but nothing lata knows to hook"
        );
        store::set_java_mods(&dir, false).unwrap();

        // Installed from its Workshop item: off until the user says so.
        put(
            &dir,
            "mods.json",
            &format!(
                r#"{{"items":[{{"id":{LOADER},"manifest":1,"title":"ZombieBuddy","folders":["ZombieBuddy"]}}]}}"#
            ),
        );
        assert_eq!(state(&instance, &list(&instance)), State::Off);
        assert_eq!(arg(&instance), None);
        store::set_java_mods(&dir, true).unwrap();
        assert_eq!(state(&instance, &list(&instance)), State::On);
        if !cfg!(windows) {
            let jar = dir.join("zomboid/mods/ZombieBuddy/libs/ZombieBuddy.jar");
            assert_eq!(
                arg(&instance),
                Some(format!("-javaagent:{}", jar.display()))
            );
        }

        // Hooked by hand in the instance's own arguments: lata adds nothing and says nothing.
        instance.jvm_args = "-javaagent:/somewhere/ZombieBuddy.jar".into();
        assert_eq!(arg(&instance), None);
        assert_eq!(state(&instance, &list(&instance)), State::None);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
