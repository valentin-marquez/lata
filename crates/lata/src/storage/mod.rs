//! Where the instance library lives (`instances/` and `builds/`), and moving it: the drives to
//! pick from, and the move itself, which keeps the old copy until the new one is complete.

#[path = "drives.model.rs"]
pub mod drives;
#[path = "picker.view.rs"]
pub mod picker;

use std::io;
use std::path::Path;

use crate::instance::store;

/// The library's folders, the only things a move takes.
const PARTS: [&str; 2] = ["instances", "builds"];

/// Bytes the library takes and how many instances it holds.
pub fn size(library: &Path) -> (u64, usize) {
    let bytes = PARTS
        .iter()
        .filter_map(|part| store::files(&library.join(part)).ok())
        .map(|(_, total)| total)
        .sum();
    (bytes, store::list(&library.join("instances")).len())
}

/// Why `to` can't take the library from `from`; `None` when it can. Leaves no folder behind.
pub fn check(from: &Path, to: &Path) -> Option<String> {
    if to == from {
        return Some(t!("Your library is already here").into());
    }
    if PARTS
        .iter()
        .any(|part| to.starts_with(from.join(part)) || from.join(part).starts_with(to))
    {
        return Some(t!("That folder is inside your library, pick one outside it").into());
    }
    if PARTS.iter().any(|part| !empty(&to.join(part))) {
        return Some(t!("That folder already has a lata library in it").into());
    }
    let (probe, made) = (to.join(".lata-probe"), !to.exists());
    let writable = std::fs::create_dir_all(to).is_ok() && std::fs::write(&probe, b"").is_ok();
    let _ = std::fs::remove_file(probe);
    if made {
        let _ = std::fs::remove_dir(to);
    }
    (!writable).then(|| t!("lata can't write in that folder, pick one you own").into())
}

/// Why a download of `need` bytes into `dir` can't start; `None` when it fits (or the OS
/// doesn't say). Checked up front so a download doesn't run out of space halfway.
pub fn room(dir: &Path, need: u64) -> Option<String> {
    let free = drives::free(dir)?;
    (need > free).then(|| {
        use crate::instance::install::size;
        t!(
            "Not enough space: this needs {need} and the drive only has {free} free. Clear some up, or move your library in Settings",
            need = size(need),
            free = size(free)
        )
    })
}

fn empty(dir: &Path) -> bool {
    std::fs::read_dir(dir).map_or(true, |mut entries| entries.next().is_none())
}

/// Moves the library from `from` to `to`. On the same filesystem that is a rename; across
/// drives every file is copied (`progress` gets bytes done and in all), and the old copy is
/// deleted only once the new one is whole. A failed copy deletes what it made and leaves the
/// library where it was.
pub fn relocate(from: &Path, to: &Path, mut progress: impl FnMut(u64, u64)) -> io::Result<()> {
    if let Some(why) = check(from, to) {
        return Err(io::Error::other(why));
    }
    let present: Vec<&str> = PARTS
        .into_iter()
        .filter(|part| from.join(part).exists())
        .collect();
    // Renames first: when one works, the drive is the same and nothing needs copying.
    std::fs::create_dir_all(to)?;
    let (mut renamed, mut copy) = (Vec::new(), Vec::new());
    for part in present {
        match std::fs::rename(from.join(part), to.join(part)) {
            Ok(()) => renamed.push(part),
            Err(_) => copy.push(part),
        }
    }
    let copied = (|| {
        let total: u64 = copy
            .iter()
            .map(|part| store::files(&from.join(part)).map(|(_, t)| t))
            .sum::<io::Result<u64>>()?;
        let mut done = 0;
        for part in &copy {
            tree(
                &from.join(part),
                &to.join(part),
                &mut done,
                total,
                &mut progress,
            )?;
        }
        progress(total, total);
        Ok(())
    })();
    if let Err(err) = copied {
        for part in &copy {
            let _ = std::fs::remove_dir_all(to.join(part));
        }
        // Never a library split across two places.
        for part in renamed {
            let _ = std::fs::rename(to.join(part), from.join(part));
        }
        return Err(err);
    }
    for part in copy {
        std::fs::remove_dir_all(from.join(part))?;
    }
    // A folder made only for the library goes with it; one with other things in it stays.
    let _ = std::fs::remove_dir(from);
    Ok(())
}

/// Copies `from` into `to`, folders included even when empty.
// ponytail: links are copied as what they point to; nothing in a library is a link today.
fn tree(
    from: &Path,
    to: &Path,
    done: &mut u64,
    total: u64,
    progress: &mut impl FnMut(u64, u64),
) -> io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            tree(&entry.path(), &target, done, total, progress)?;
        } else {
            *done += std::fs::copy(entry.path(), &target)?;
            progress(*done, total);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-storage-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn moves_the_library_and_refuses_bad_targets() {
        let base = temp("move");
        let (from, to) = (base.join("old"), base.join("new"));
        std::fs::create_dir_all(from.join("instances/a/zomboid/empty")).unwrap();
        std::fs::write(from.join("instances/a/instance.json"), b"{}").unwrap();
        std::fs::create_dir_all(from.join("builds/1")).unwrap();
        std::fs::write(from.join("builds/1/game.jar"), b"jar").unwrap();
        std::fs::write(from.join("settings.json"), b"stays").unwrap();

        assert!(check(&from, &from).is_some());
        assert!(check(&from, &from.join("instances/x")).is_some());

        relocate(&from, &to, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(to.join("builds/1/game.jar")).unwrap(), b"jar");
        assert!(to.join("instances/a/zomboid/empty").is_dir());
        assert!(!from.join("instances").exists() && !from.join("builds").exists());
        assert!(
            from.join("settings.json").is_file(),
            "only the library moves"
        );
        relocate(&to, &base.join("third"), |_, _| {}).unwrap();
        assert!(!to.exists(), "an emptied folder is not left behind");

        // Back again is refused only while the old place still holds a library.
        std::fs::create_dir_all(from.join("instances/b")).unwrap();
        assert!(check(&to, &from).is_some());

        // Copying across drives can't be faked here; the copy path still runs on its own.
        let (mut done, copy) = (0, base.join("copy"));
        tree(
            &base.join("third/builds"),
            &copy,
            &mut done,
            3,
            &mut |_, _| {},
        )
        .unwrap();
        assert_eq!(done, 3);
        std::fs::remove_dir_all(base).unwrap();
    }
}
