//! Putting a downloaded lata in the place of the one that runs. The new one is unpacked and
//! tried in a folder beside the old (`lata.update`), so the swap itself is only renames on
//! one filesystem, and the old one stays as `lata.old` until the next lata has its window up.

use std::ffi::OsString;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::Version;

/// What a package holds that replaces what is installed: the binary, or on macOS the whole
/// app (its signature covers the folder).
const PAYLOAD: &str = if cfg!(target_os = "macos") {
    "Lata.app"
} else if cfg!(windows) {
    "lata.exe"
} else {
    "lata"
};

/// `target` with `.kind` after its name: `lata.old`, `Lata.app.update`.
fn beside(target: &Path, kind: &str) -> PathBuf {
    let mut name = OsString::from(target.as_os_str());
    name.push(".");
    name.push(kind);
    PathBuf::from(name)
}

/// What an update replaces: this binary (through any link to it), or the `.app` around it on
/// macOS. `None` for a macOS binary outside an app, which no package matches.
pub fn installed() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?.canonicalize().ok()?;
    if !cfg!(target_os = "macos") {
        return Some(exe);
    }
    // Lata.app/Contents/MacOS/lata
    let app = exe.ancestors().nth(3)?;
    (app.extension()? == "app").then(|| app.to_owned())
}

/// Whether a store that updates lata by itself runs this one (Flatpak, Snap): there is
/// nothing to tell the user, the store brings the next version.
pub fn in_store() -> bool {
    std::env::var_os("FLATPAK_ID").is_some() || std::env::var_os("SNAP").is_some()
}

/// Where lata replaces itself. Only Linux for now: the Windows and macOS swaps (renaming a
/// running .exe, replacing a whole .app) have never run on those systems, and an updater
/// that breaks an install is worse than none. There lata says a newer version is out and
/// links it.
const SELF_INSTALL: bool = cfg!(target_os = "linux");

/// [`installed`], when lata may replace it. Not when something else owns the install: a
/// store, an AppImage (one read-only file, mounted while it runs), or a folder this user
/// can't write (a system package under /usr, Program Files, an app macOS runs from a
/// quarantine copy). Never a debug build either: `cargo run` must not overwrite `target/`.
pub fn replaceable() -> Option<PathBuf> {
    let packaged = in_store() || std::env::var_os("APPIMAGE").is_some();
    if !SELF_INSTALL || cfg!(debug_assertions) || packaged {
        return None;
    }
    let target = installed()?;
    // Asking the filesystem beats reading permission bits: it also knows about read-only
    // mounts and access lists. A new file under a name nobody else uses, so nothing that is
    // already there can be emptied or followed.
    let probe = beside(&target, &format!("probe-{}", std::process::id()));
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .ok()?;
    let _ = std::fs::remove_file(probe);
    Some(target)
}

/// Whether anything is at `path`, a dangling link included.
fn exists(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

/// Removes a file, a folder with what is in it, or a link (never what it points to).
fn remove(path: &Path) {
    let _ = match path.symlink_metadata() {
        Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
        Err(_) => Ok(()),
    };
}

/// The file in `lata.update` that says an update of lata's made that folder, and so also
/// the `lata.old` beside it. Without it neither is touched: someone may keep a `lata.old`
/// of their own.
const MARK: &str = "made-by-lata";
/// What a package may unpack to, and in how many entries; the real ones are about 70 MB
/// and a handful of files.
const UNPACKED_BYTES: u64 = 512 * 1024 * 1024;
const ENTRIES: usize = 2000;
/// How long the new lata gets to say its version.
const TRIAL: Duration = Duration::from_secs(10);

/// Unpacks a package (`.zip`, or `.tar.gz` by its name) into `dir`. Both readers refuse
/// entries that would land outside it, and neither unpacks more than [`UNPACKED_BYTES`].
fn unpack(package: &[u8], name: &str, dir: &Path) -> Result<(), String> {
    if name.ends_with(".zip") {
        let mut zip =
            zip::ZipArchive::new(io::Cursor::new(package)).map_err(|err| err.to_string())?;
        let mut bytes = 0u64;
        for ix in 0..zip.len() {
            let entry = zip.by_index_raw(ix).map_err(|err| err.to_string())?;
            bytes = bytes.saturating_add(entry.size());
        }
        if zip.len() > ENTRIES || bytes > UNPACKED_BYTES {
            return Err("the download unpacks to far more than lata is".into());
        }
        zip.extract(dir).map_err(|err| err.to_string())
    } else {
        // Cut off at the limit, the archive ends early and unpacking fails.
        let limited = flate2::read::GzDecoder::new(package).take(UNPACKED_BYTES);
        tar::Archive::new(limited)
            .unpack(dir)
            .map_err(|err| err.to_string())
    }
}

/// `name` in an unpacked package: at the top, or inside its one folder. Never a link.
fn payload(dir: &Path, name: &str) -> Option<PathBuf> {
    let real = |path: &PathBuf| path.symlink_metadata().is_ok_and(|meta| !meta.is_symlink());
    let top = dir.join(name);
    if real(&top) {
        return Some(top);
    }
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(name))
        .find(real)
}

/// Whether the new lata starts on this computer and is the version it should be, asked with
/// `--version`. A binary for another system, one that misses a library, or a file that isn't
/// a program at all stops here, before anything is replaced. It proves the program loads,
/// not that its window opens.
fn runs(payload: &Path, version: &Version) -> Result<(), String> {
    let binary = if payload.is_dir() {
        payload.join("Contents/MacOS/lata")
    } else {
        payload.to_owned()
    };
    let no = || "the new version doesn't start here".to_owned();
    let mut child = Command::new(binary)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("{}: {err}", no()))?;
    let deadline = Instant::now() + TRIAL;
    let ended = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            // Still going (it opened a window, or hangs): stop it and don't trust it.
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    // One short line is all it should say; the rest, if any, is not read.
    let mut said = String::new();
    if let (true, Some(out)) = (ended, child.stdout.take()) {
        let _ = out.take(64).read_to_string(&mut said);
    }
    if ended && said.trim() == format!("lata {version}") {
        Ok(())
    } else {
        Err(no())
    }
}

/// Unpacks a verified package beside `target`, tries it, and says where the new lata is.
/// A download that isn't a package, or a lata that won't start here, stops before anything
/// is replaced, and leaves nothing behind.
pub fn stage(
    package: &[u8],
    name: &str,
    target: &Path,
    version: &Version,
) -> Result<PathBuf, String> {
    stage_as(package, name, target, version, PAYLOAD)
}

fn stage_as(
    package: &[u8],
    name: &str,
    target: &Path,
    version: &Version,
    payload_name: &str,
) -> Result<PathBuf, String> {
    let (old, dir) = (beside(target, "old"), beside(target, "update"));
    // What the last update left goes first. Anything still there after that isn't lata's to
    // remove (or is the old lata, still closing on Windows): stop instead of deleting it.
    tidy(target);
    if let Some(taken) = [&old, &dir].into_iter().find(|path| exists(path)) {
        return Err(format!("{} is in the way", taken.display()));
    }
    let text = |err: io::Error| err.to_string();
    std::fs::create_dir(&dir).map_err(text)?;
    let staged = std::fs::write(dir.join(MARK), version.to_string())
        .map_err(text)
        .and_then(|()| unpack(package, name, &dir.join("unpacked")))
        .and_then(|()| {
            let new = payload(&dir.join("unpacked"), payload_name)
                .ok_or("the download has no lata in it")?;
            runs(&new, version)?;
            Ok(new)
        });
    if staged.is_err() {
        remove(&dir);
    }
    staged
}

/// Puts `new` where `target` is and keeps the old one as `target.old`. If it fails, `target`
/// is what it was, or the error says where the old one is.
pub fn swap(new: &Path, target: &Path) -> io::Result<()> {
    let old = beside(target, "old");
    if exists(&old) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} is in the way", old.display()),
        ));
    }
    // A file on Unix: a second name for the old one, then a rename over the first. That
    // rename is atomic, so there is no moment without a lata at `target`, and a running
    // program keeps the file it was started from. No other way is tried if the link can't
    // be made: an update that could leave nothing at `target` is not worth having here.
    if cfg!(unix) && target.is_file() {
        // On disk before it takes the place, so a power cut can't leave a hollow file there.
        std::fs::File::open(new)?.sync_all()?;
        std::fs::hard_link(target, &old)?;
        std::fs::rename(new, target).inspect_err(|_| {
            let _ = std::fs::remove_file(&old);
        })?;
        if let Some(folder) = target.parent() {
            let _ = std::fs::File::open(folder).and_then(|folder| folder.sync_all());
        }
        return Ok(());
    }
    // A folder (macOS) or Windows (where a running .exe can be renamed but not overwritten):
    // two renames, and the first undone if the second fails.
    // shortcut: between the two renames nothing is at `target`; a crash right there leaves
    // the old one as `target.old`, to rename back by hand. Closing that takes an atomic
    // exchange (renamex_np on macOS) or a helper that outlives lata; until one exists,
    // [`SELF_INSTALL`] keeps these systems from getting here.
    std::fs::rename(target, &old)?;
    std::fs::rename(new, target).map_err(|err| match std::fs::rename(&old, target) {
        Ok(()) => err,
        Err(undo) => io::Error::other(format!(
            "{err}, and the old lata couldn't be put back ({undo}): it is at {}",
            old.display()
        )),
    })
}

/// Clears what an update of lata's left beside `target`: the lata it replaced, and the
/// folder the new one was unpacked in. Nothing without that folder's mark.
pub fn tidy(target: &Path) {
    let (old, dir) = (beside(target, "old"), beside(target, "update"));
    if !dir.join(MARK).is_file() {
        return;
    }
    remove(&old);
    // On Windows the old lata may still be closing and can't be deleted yet; the mark stays
    // with it, so a later start finishes this.
    if !exists(&old) {
        remove(&dir);
    }
}

/// Starts the lata at `target`. On macOS through `open`, so the system treats it as the app
/// being opened and not as a child of this one; `-n` because this one is still around.
pub fn launch(target: &Path, args: &[String]) -> io::Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut open = Command::new("open");
        open.arg("-n").arg(target).arg("--args");
        open
    } else {
        Command::new(target)
    };
    command.args(args).spawn().map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::io::Write;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-update-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// A `.tar.gz` like the Linux package: one folder with `lata` (executable) and a README.
    fn tarball(lata: &[u8]) -> Vec<u8> {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (path, mode, data) in [
            (
                "lata-launcher-v9.9.9-linux-x86_64/README.md",
                0o644,
                &b"hi"[..],
            ),
            ("lata-launcher-v9.9.9-linux-x86_64/lata", 0o755, lata),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(mode);
            header.set_cksum();
            tar.append_data(&mut header, path, data).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    /// A `.zip` with the given files.
    fn zipped(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for (path, data) in files {
            let options = zip::write::SimpleFileOptions::default().unix_permissions(0o755);
            zip.start_file(*path, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn unpacks_each_systems_package_and_finds_lata_in_it() {
        let dir = temp("unpack");
        let (linux, windows, macos) = (dir.join("linux"), dir.join("windows"), dir.join("macos"));
        unpack(&tarball(b"elf"), "lata.tar.gz", &linux).unwrap();
        let found = payload(&linux, "lata").unwrap();
        assert_eq!(fs::read(&found).unwrap(), b"elf");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&found).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "it must still be a program");
        }

        let package = zipped(&[("lata-launcher-v9.9.9-windows-x86_64/lata.exe", b"mz")]);
        unpack(&package, "lata.zip", &windows).unwrap();
        assert_eq!(
            fs::read(payload(&windows, "lata.exe").unwrap()).unwrap(),
            b"mz"
        );

        let package = zipped(&[
            ("Lata.app/Contents/Info.plist", b"plist"),
            ("Lata.app/Contents/MacOS/lata", b"macho"),
        ]);
        unpack(&package, "lata.zip", &macos).unwrap();
        let app = payload(&macos, "Lata.app").unwrap();
        assert_eq!(fs::read(app.join("Contents/MacOS/lata")).unwrap(), b"macho");

        assert_eq!(payload(&macos, "lata.exe"), None);
        assert!(unpack(b"not an archive", "lata.zip", &dir.join("bad")).is_err());
        assert!(unpack(b"not an archive", "lata.tar.gz", &dir.join("bad")).is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_package_cannot_write_outside_its_folder() {
        let dir = temp("escape");
        let inside = dir.join("inside");
        let package = zipped(&[("../escaped", b"x"), ("/abs", b"x")]);
        let _ = unpack(&package, "lata.zip", &inside);
        assert!(!dir.join("escaped").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn swaps_a_file_and_keeps_the_old_one() {
        let dir = temp("swap-file");
        let (target, new) = (dir.join("lata"), dir.join("new"));
        fs::write(&target, "old").unwrap();
        fs::write(&new, "new").unwrap();
        // Someone's own `lata.old` is not lata's to overwrite: nothing moves.
        fs::write(dir.join("lata.old"), "mine").unwrap();
        assert!(swap(&new, &target).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(fs::read_to_string(dir.join("lata.old")).unwrap(), "mine");
        fs::remove_file(dir.join("lata.old")).unwrap();
        swap(&new, &target).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), "new");
        assert_eq!(fs::read_to_string(dir.join("lata.old")).unwrap(), "old");
        assert!(!new.exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn swaps_a_folder_like_a_macos_app() {
        let dir = temp("swap-dir");
        let (target, new) = (dir.join("Lata.app"), dir.join("stage/Lata.app"));
        for (app, text) in [(&target, "old"), (&new, "new")] {
            fs::create_dir_all(app.join("Contents")).unwrap();
            fs::write(app.join("Contents/lata"), text).unwrap();
        }
        swap(&new, &target).unwrap();
        let read = |app: &str| fs::read_to_string(dir.join(app).join("Contents/lata")).unwrap();
        assert_eq!(read("Lata.app"), "new");
        assert_eq!(read("Lata.app.old"), "old");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_swap_leaves_the_old_one_in_place() {
        let dir = temp("swap-fail");
        let file = dir.join("lata");
        fs::write(&file, "old").unwrap();
        assert!(swap(&dir.join("missing"), &file).is_err());
        assert_eq!(fs::read_to_string(&file).unwrap(), "old");

        let app = dir.join("Lata.app");
        fs::create_dir_all(&app).unwrap();
        fs::write(app.join("lata"), "old").unwrap();
        assert!(swap(&dir.join("missing"), &app).is_err());
        assert_eq!(fs::read_to_string(app.join("lata")).unwrap(), "old");
        assert!(!dir.join("Lata.app.old").exists(), "the rename was undone");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// The whole way on a temp folder, with a shell script standing in for lata.
    #[cfg(unix)]
    #[test]
    fn stages_tries_swaps_and_tidies() {
        let dir = temp("stage");
        let target = dir.join("lata");
        fs::write(&target, "old").unwrap();
        let script = b"#!/bin/sh\necho lata 9.9.9\n";

        // It says another version than the release claims: not staged, nothing left behind.
        let err = stage_as(&tarball(script), "l.tar.gz", &target, &v("9.9.8"), "lata");
        assert!(err.is_err());
        assert!(!dir.join("lata.update").exists());
        // Not a program at all, and one that says the right thing but fails.
        assert!(stage_as(&tarball(b"junk"), "l.tar.gz", &target, &v("9.9.9"), "lata").is_err());
        let failing = b"#!/bin/sh\necho lata 9.9.9\nexit 3\n";
        assert!(stage_as(&tarball(failing), "l.tar.gz", &target, &v("9.9.9"), "lata").is_err());
        assert!(!dir.join("lata.update").exists());

        // A `lata.update` that lata didn't make is left alone, and stops the update.
        fs::create_dir(dir.join("lata.update")).unwrap();
        fs::write(dir.join("lata.update/notes"), "mine").unwrap();
        assert!(stage_as(&tarball(script), "l.tar.gz", &target, &v("9.9.9"), "lata").is_err());
        tidy(&target);
        assert_eq!(
            fs::read_to_string(dir.join("lata.update/notes")).unwrap(),
            "mine"
        );
        fs::remove_dir_all(dir.join("lata.update")).unwrap();

        let new = stage_as(&tarball(script), "l.tar.gz", &target, &v("9.9.9"), "lata").unwrap();
        // Still the old one: staging replaces nothing.
        assert_eq!(fs::read_to_string(&target).unwrap(), "old");

        swap(&new, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), script);
        assert_eq!(fs::read_to_string(dir.join("lata.old")).unwrap(), "old");
        // The new one starts: the old one and the staging folder go.
        tidy(&target);
        assert!(!dir.join("lata.old").exists());
        assert!(!dir.join("lata.update").exists());
        assert_eq!(fs::read(&target).unwrap(), script);
        // With no update of lata's behind it, a `lata.old` stays.
        fs::write(dir.join("lata.old"), "mine").unwrap();
        tidy(&target);
        assert!(dir.join("lata.old").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
