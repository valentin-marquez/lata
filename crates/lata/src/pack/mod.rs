//! `.lata` packs: an instance in a can, to share. A zip shaped like Modrinth's `.mrpack` (an
//! index plus `overrides/`) with our own index, since `.mrpack` requires `game: "minecraft"`
//! and download URLs on Modrinth's domains.
//!
//! Workshop mods travel as references, item and manifest (an exact version), the way
//! `.mrpack` lists files by hash and CDN URL. What the player picks of the instance's
//! `zomboid/` goes in `overrides/`: server and sandbox settings, mod options, and the mods
//! that aren't on the Workshop. Format notes in `docs/pack.md`.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use zip::ZipArchive;
use zip::write::SimpleFileOptions;

use crate::instance::mods::{self, Active, Pin};
use crate::instance::{Icon, Instance, store};

#[path = "export.view.rs"]
pub mod export;

pub const EXT: &str = "lata";
const INDEX: &str = "lata.index.json";
const OVERRIDES: &str = "overrides";
/// Bumped when a change would make an older lata misread a pack.
const FORMAT: u32 = 1;
const GAME: &str = "projectzomboid";
/// More index than any real pack needs; a bigger one isn't read.
const INDEX_MAX: u64 = 4 << 20;

/// Checked when the export dialog opens: what makes the pack play the same.
pub const PICKED: [&str; 4] = ["Lua", "Sandbox Presets", "Server", "mods"];

/// Never offered: the game or lata rebuilds them, or they only say something about this
/// computer. The active mod list travels in the index instead of `mods/default.txt`.
const NEVER: [&str; 6] = [
    "Logs",
    "Workshop",
    "console.txt",
    "logs.zip",
    "version.txt",
    "mods/default.txt",
];

/// What a folder or file of `zomboid/` is for, for players who don't know the game's
/// folders. Read from a real `~/Zomboid` and the game's `ZomboidFileSystem`.
pub fn about(path: &Path) -> Option<&'static str> {
    Some(match path.to_str()? {
        "Lua" => {
            "Mod options and in-game settings: key bindings, the UI layout and characters saved \
             in the creation screen."
        }
        "mods" => {
            "Mods added by hand, not from the Workshop. Workshop mods go as links, so they \
             don't show here."
        }
        "Saves" => "Your worlds. They can weigh gigabytes, and friends usually start their own.",
        "Server" => {
            "Hosting settings: server rules, sandbox options and spawn points. Passwords and \
             tokens are left out."
        }
        "Sandbox Presets" => {
            "Sandbox settings saved in the game's setup screen, like zombie count and loot."
        }
        "db" => "Your saved server list. It can hold server passwords, so keep it to yourself.",
        "Crafting" => "A list of recipes the game writes for itself. It makes it again.",
        "Screenshots" => "Pictures you took in the game.",
        "options.ini" => {
            "Your own game options: screen, sound and controls. Everyone keeps their own."
        }
        _ => return None,
    })
}

/// Keys of a server's settings that are secrets: they go in a pack blank, in any file under
/// `Server/` (a backup copy of the `.ini` holds them too) and in any case.
const SECRETS: [&str; 3] = ["Password", "RCONPassword", "DiscordToken"];

/// A server's settings file with the values of [`SECRETS`] taken out, keys kept.
/// Works on bytes, so a file in another encoding than UTF-8 is cleaned the same.
fn without_secrets(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for line in bytes.split_inclusive(|b| *b == b'\n') {
        let key = line.iter().position(|b| *b == b'=').map(|at| &line[..at]);
        match key {
            Some(key)
                if SECRETS
                    .iter()
                    .any(|s| s.as_bytes().eq_ignore_ascii_case(key.trim_ascii())) =>
            {
                out.extend_from_slice(key);
                out.push(b'=');
                // Keep the line's own ending.
                let ending = if line.ends_with(b"\r\n") {
                    &b"\r\n"[..]
                } else if line.ends_with(b"\n") {
                    &b"\n"[..]
                } else {
                    &b""[..]
                };
                out.extend_from_slice(ending);
            }
            _ => out.extend_from_slice(line),
        }
    }
    out
}

/// `lata.index.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Index {
    pub format: u32,
    pub game: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub summary: String,
    pub build: Build,
    /// The instance's Workshop items, each at the version it had.
    #[serde(default)]
    pub workshop: Vec<Pin>,
    /// Active mod ids, in load order.
    #[serde(default)]
    pub mods: Vec<String>,
    /// Active maps, in order.
    #[serde(default)]
    pub maps: Vec<String>,
    #[serde(default)]
    pub settings: Settings,
    /// An uploaded picture sits at the top of the zip under the name it gives.
    #[serde(default)]
    pub icon: Option<Icon>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Build {
    pub branch: String,
    /// The build it was made on.
    #[serde(default)]
    pub id: Option<u32>,
    /// That build's manifest for every OS depot, when the pack keeps the game at that build.
    /// Left out, the pack follows its branch: an import installs the branch's current build.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depots: Vec<Depot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Depot {
    pub id: u32,
    pub manifest: u64,
}

/// The depots that keep a pack of `instance` at its build: empty when its manifests aren't
/// known (installed before lata recorded them, and missing from the catalog).
pub fn depots(instance: &Instance) -> Vec<Depot> {
    let Some(build) = instance.build else {
        return Vec::new();
    };
    crate::instance::build::manifests(build, instance.game().as_deref())
        .into_iter()
        .map(|(id, manifest)| Depot { id, manifest })
        .collect()
}

/// The instance settings a pack carries. Java and game arguments stay out: one like
/// `-javaagent` would run whatever the pack bundles as soon as someone presses Play.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub memory_gb: Option<u32>,
}

/// What a Workshop download at chosen versions needs besides the items: the version of each,
/// and for a pack the mods to have on once they are down (an install turns on everything it
/// brings). Without `active`, what is on stays as it was.
pub struct Wanted {
    pub exact: Vec<(u64, u64)>,
    pub active: Option<Active>,
}

impl Index {
    /// The Workshop items to download, and what goes with them.
    pub fn wanted(&self) -> (Vec<u64>, Wanted) {
        let ids = self.workshop.iter().map(|pin| pin.id).collect();
        let wanted = Wanted {
            exact: self
                .workshop
                .iter()
                .map(|pin| (pin.id, pin.manifest))
                .collect(),
            active: Some(Active {
                mods: self.mods.clone(),
                maps: self.maps.clone(),
            }),
        };
        (ids, wanted)
    }
}

/// What the export dialog asks for besides the files.
pub struct About {
    pub name: String,
    pub version: String,
    pub summary: String,
    /// Keep the game at the instance's build, rather than the branch's newest.
    pub exact: bool,
}

/// A file or folder of the instance's `zomboid/`, as the export dialog lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// Relative to `zomboid/`.
    pub path: PathBuf,
    pub name: String,
    /// In bytes; a folder's is everything in it that can go.
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// `None` for a file.
    pub children: Option<Vec<Node>>,
}

/// What of the instance's `zomboid/` can go in a pack, folders first and by name.
pub fn tree(instance: &Instance) -> Vec<Node> {
    walk(
        &instance.dir.join("zomboid"),
        Path::new(""),
        &skipped(instance),
    )
}

fn walk(root: &Path, rel: &Path, skip: &[PathBuf]) -> Vec<Node> {
    let mut nodes: Vec<Node> = std::fs::read_dir(root.join(rel))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = rel.join(entry.file_name());
            // Links stay out: they could point anywhere on this computer.
            let meta = entry.metadata().ok().filter(|meta| !meta.is_symlink())?;
            if skip.contains(&path) {
                return None;
            }
            let (name, modified) = (
                entry.file_name().to_string_lossy().into_owned(),
                meta.modified().ok(),
            );
            Some(if meta.is_dir() {
                let children = walk(root, &path, skip);
                Node {
                    size: children.iter().map(|child| child.size).sum(),
                    children: Some(children),
                    path,
                    name,
                    modified,
                }
            } else {
                Node {
                    size: meta.len(),
                    children: None,
                    path,
                    name,
                    modified,
                }
            })
        })
        .collect();
    nodes.sort_by_cached_key(|node| (node.children.is_none(), node.name.to_lowercase()));
    nodes
}

/// What [`NEVER`] names, plus the folders of Workshop mods (they go as references) and the
/// game's `reset-mods-*.txt` markers.
fn skipped(instance: &Instance) -> Vec<PathBuf> {
    let mods = instance.dir.join("zomboid/mods");
    let markers = std::fs::read_dir(&mods)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("reset-mods-"));
    NEVER
        .iter()
        .map(PathBuf::from)
        .chain(
            mods::pins(instance)
                .into_iter()
                .flat_map(|pin| pin.folders)
                .chain(markers)
                .map(|name| Path::new("mods").join(name)),
        )
        .collect()
}

/// The files under `nodes` that go: the ones in `picked`, and everything in a picked folder.
fn gather(nodes: &[Node], picked: &[PathBuf], inside: bool, files: &mut Vec<PathBuf>) {
    for node in nodes {
        let goes = inside || picked.contains(&node.path);
        match &node.children {
            Some(children) => gather(children, picked, goes, files),
            None if goes => files.push(node.path.clone()),
            None => {}
        }
    }
}

/// A path in the zip: forward slashes on every system.
fn zip_path(rel: &Path) -> String {
    rel.components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn zipped(err: zip::result::ZipError) -> io::Error {
    io::Error::other(err)
}

fn invalid(why: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, why.to_string())
}

/// Writes `instance` as a pack at `out`, with the files and folders of `picked` (paths from
/// [`tree`]). The pack is written beside `out` first, so a failed export leaves nothing.
pub fn export(
    instance: &Instance,
    about: &About,
    picked: &[PathBuf],
    out: &Path,
) -> io::Result<()> {
    let mut files = Vec::new();
    gather(&tree(instance), picked, false, &mut files);
    let active = mods::active(instance);
    let index = Index {
        format: FORMAT,
        game: GAME.into(),
        name: about.name.trim().into(),
        version: about.version.trim().into(),
        summary: about.summary.trim().into(),
        build: Build {
            branch: instance.branch.to_string(),
            id: instance.build,
            depots: match about.exact {
                true => depots(instance),
                false => Vec::new(),
            },
        },
        workshop: mods::pins(instance),
        mods: active.mods,
        maps: active.maps,
        settings: Settings {
            memory_gb: instance.memory_gb,
        },
        icon: instance.icon.clone(),
    };
    let part = out.with_extension("lata-part");
    let written = (|| {
        let mut zip = zip::ZipWriter::new(BufWriter::new(File::create(&part)?));
        let options = SimpleFileOptions::default();
        if let Some(Icon::Image { file }) = &index.icon {
            zip.start_file(file.as_str(), options).map_err(zipped)?;
            io::copy(&mut File::open(instance.dir.join(file))?, &mut zip)?;
        }
        let root = instance.dir.join("zomboid");
        for rel in &files {
            zip.start_file(format!("{OVERRIDES}/{}", zip_path(rel)), options)
                .map_err(zipped)?;
            // Anything under `Server/` loses its secrets, whatever the file's name or encoding.
            if rel.starts_with("Server") {
                zip.write_all(&without_secrets(&std::fs::read(root.join(rel))?))?;
            } else {
                io::copy(&mut File::open(root.join(rel))?, &mut zip)?;
            }
        }
        zip.start_file(INDEX, options).map_err(zipped)?;
        zip.write_all(&serde_json::to_vec_pretty(&index).map_err(io::Error::other)?)?;
        zip.finish().map_err(zipped)?.flush()
    })();
    match written {
        Ok(()) => std::fs::rename(&part, out),
        Err(err) => {
            let _ = std::fs::remove_file(&part);
            Err(err)
        }
    }
}

/// The name an uploaded picture may have in a pack: `icon.<ext>`, as the instance keeps it.
fn icon_file(icon: &Option<Icon>) -> Option<&str> {
    let Some(Icon::Image { file }) = icon else {
        return None;
    };
    let ext = file.strip_prefix("icon.")?;
    (!ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric())).then_some(file)
}

/// The index of the pack at `path`, after checking that every file in it stays inside the
/// instance: nothing absolute and no `..` that climbs out.
pub fn read(path: &Path) -> io::Result<Index> {
    let mut zip = ZipArchive::new(File::open(path)?).map_err(|_| invalid("it isn't a pack"))?;
    for ix in 0..zip.len() {
        let file = zip.by_index_raw(ix).map_err(zipped)?;
        // `enclosed_name` turns `/etc/x` into `etc/x`; a pack made by lata has none of those.
        let absolute = file
            .name_raw()
            .first()
            .is_some_and(|c| matches!(c, b'/' | b'\\'))
            || file.name_raw().contains(&b':');
        if absolute || file.enclosed_name().is_none() {
            return Err(invalid("it has a file that points outside the instance"));
        }
    }
    let mut text = String::new();
    zip.by_name(INDEX)
        .map_err(|_| invalid("it has no lata.index.json"))?
        .take(INDEX_MAX)
        .read_to_string(&mut text)?;
    let index: Index = serde_json::from_str(&text).map_err(invalid)?;
    if index.game != GAME {
        return Err(invalid("it's for another game"));
    }
    if index.format > FORMAT {
        return Err(invalid("a newer lata made it, update to open it"));
    }
    Ok(index)
}

/// Makes an instance under `root` from the pack at `path`, with its settings, icon, files and
/// active mods. The game and the Workshop mods are left to download. Nothing stays when it
/// fails halfway.
pub fn import(root: &Path, path: &Path) -> io::Result<(Instance, Index)> {
    let index = read(path)?;
    let name = match index.name.trim() {
        "" => "Shared pack",
        name => name,
    };
    let instance = store::create(root, name, &index.build.branch)?;
    let made = (|| {
        store::configure(
            &instance.dir,
            &store::Settings {
                name: name.into(),
                memory_gb: index.settings.memory_gb,
                jvm_args: String::new(),
                launch_args: String::new(),
            },
        )?;
        let mut zip = ZipArchive::new(File::open(path)?).map_err(zipped)?;
        let zomboid = instance.dir.join("zomboid");
        let icon = icon_file(&index.icon);
        for ix in 0..zip.len() {
            let mut file = zip.by_index(ix).map_err(zipped)?;
            let Some(name) = file.enclosed_name() else {
                continue;
            };
            let target = match name.strip_prefix(OVERRIDES) {
                Ok(rel) if file.is_file() && !rel.as_os_str().is_empty() => zomboid.join(rel),
                _ if icon.is_some_and(|icon| name == Path::new(icon)) => instance.dir.join(name),
                _ => continue,
            };
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            io::copy(&mut file, &mut File::create(&target)?)?;
        }
        // An item icon is just data; a picture only when its file came along.
        let icon = match &index.icon {
            Some(Icon::Image { .. }) if icon.is_none() => None,
            Some(Icon::Image { file }) if !instance.dir.join(file).is_file() => None,
            other => other.clone(),
        };
        store::set_icon(&instance.dir, icon)?;
        if let Some(build) = index.build.id.filter(|_| !index.build.depots.is_empty()) {
            let depots = index.build.depots.iter().map(|d| (d.id, d.manifest));
            store::set_target(
                &instance.dir,
                Some(store::Target {
                    build,
                    depots: depots.collect(),
                }),
            )?;
        }
        mods::activate(
            &instance,
            &Active {
                mods: index.mods.clone(),
                maps: index.maps.clone(),
            },
        )
    })();
    if let Err(err) = made {
        let _ = std::fs::remove_dir_all(&instance.dir);
        return Err(err);
    }
    let instance = store::list(root)
        .into_iter()
        .find(|found| found.id == instance.id)
        .unwrap_or(instance);
    Ok((instance, index))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lata-pack-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, file: &str, text: &str) {
        let path = dir.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// An instance with settings, a picture, files of each kind, a Workshop mod and a local one.
    fn instance(root: &Path) -> Instance {
        let made = store::create(root, "Server with friends", "unstable").unwrap();
        store::configure(
            &made.dir,
            &store::Settings {
                name: "Server with friends".into(),
                memory_gb: Some(8),
                jvm_args: "-XX:+UseZGC".into(),
                launch_args: String::new(),
            },
        )
        .unwrap();
        put(&made.dir, "icon.png", "png");
        store::set_icon(
            &made.dir,
            Some(Icon::Image {
                file: "icon.png".into(),
            }),
        )
        .unwrap();
        let z = made.dir.join("zomboid");
        put(
            &z,
            "Server/servertest.ini",
            "PVP=false\nPassword=hunter2\nRCONPassword=secret\nDiscordToken=abc\n",
        );
        put(&z, "Server/servertest.ini.bak", "password = hunter2\n");
        put(&z, "Lua/options.ini", "x=1");
        put(&z, "Saves/Sandbox/World/map_ver.bin", "save");
        put(&z, "options.ini", "fullscreen=true");
        put(&z, "console.txt", "log");
        put(&z, "mods/Local/42/mod.info", "id=Local\nname=Local");
        put(&z, "mods/Brita/42/mod.info", "id=Brita\nname=Brita");
        put(&z, "mods/reset-mods-42_00.txt", "");
        std::fs::write(
            made.dir.join("mods.json"),
            r#"{"items":[{"id":2392709985,"manifest":4123456789012,"title":"Brita's Weapon Pack","folders":["Brita"]}]}"#,
        )
        .unwrap();
        mods::activate(
            &made,
            &Active {
                mods: vec!["Brita".into(), "Local".into()],
                maps: vec!["Muldraugh, KY".into()],
            },
        )
        .unwrap();
        // Runs build 7, whose record keeps every OS's manifest.
        store::set_build(&made.dir, 7).unwrap();
        let game = store::builds(root).join("7");
        put(
            &game,
            "build.json",
            r#"{"build":7,"version":null,"branches":["unstable"],"seen":1,"manifests":{"108603":31,"108604":41}}"#,
        );
        store::list(root).into_iter().next().unwrap()
    }

    #[test]
    fn offers_what_can_go_and_leaves_the_rest_out() {
        let root = temp("tree");
        let instance = instance(&root);
        let names = |nodes: &[Node]| nodes.iter().map(|n| n.name.clone()).collect::<Vec<_>>();
        let tree = tree(&instance);
        // Folders first; logs, the version file and the active list never show.
        assert_eq!(
            names(&tree),
            ["Lua", "mods", "Saves", "Server", "options.ini"]
        );
        // Workshop mods go as references and the game's marker stays behind.
        let mods = tree.iter().find(|n| n.name == "mods").unwrap();
        assert_eq!(names(mods.children.as_ref().unwrap()), ["Local"]);
        assert_eq!(mods.size, "id=Local\nname=Local".len() as u64);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_pack_brings_the_instance_back() {
        let root = temp("export");
        let instance = instance(&root);
        let out = root.join("friends.lata");
        let about = About {
            name: " Server with friends ".into(),
            version: "1.0.0".into(),
            summary: String::new(),
            exact: true,
        };
        let picked: Vec<PathBuf> = PICKED.iter().map(PathBuf::from).collect();
        export(&instance, &about, &picked, &out).unwrap();
        assert!(!root.join("friends.lata-part").exists());

        let index = read(&out).unwrap();
        assert_eq!(index.name, "Server with friends");
        assert_eq!(index.build.branch, "unstable");
        assert_eq!(index.build.id, Some(7));
        assert_eq!(
            index.build.depots,
            [
                Depot {
                    id: 108603,
                    manifest: 31
                },
                Depot {
                    id: 108604,
                    manifest: 41
                }
            ]
        );
        assert_eq!(index.workshop[0].manifest, 4123456789012);
        assert_eq!(index.mods, ["Brita", "Local"]);
        assert_eq!(index.maps, ["Muldraugh, KY"]);
        assert_eq!(index.settings.memory_gb, Some(8));

        let other = temp("import");
        let (made, _) = import(&other, &out).unwrap();
        let z = made.dir.join("zomboid");
        assert_eq!(made.name.as_ref(), "Server with friends");
        assert_eq!(made.branch.as_ref(), "unstable");
        assert_eq!(made.memory_gb, Some(8));
        // Arguments never travel: they could make the game run what the pack bundles.
        assert_eq!(made.jvm_args, "");
        assert!(made.dir.join("icon.png").is_file());
        // The server's settings come, its secrets don't.
        assert_eq!(
            std::fs::read_to_string(z.join("Server/servertest.ini")).unwrap(),
            "PVP=false\nPassword=\nRCONPassword=\nDiscordToken=\n"
        );
        assert_eq!(
            std::fs::read_to_string(z.join("Server/servertest.ini.bak")).unwrap(),
            "password =\n"
        );
        assert!(z.join("Lua/options.ini").is_file());
        assert!(z.join("mods/Local/42/mod.info").is_file());
        // Not picked, or not the pack's to carry.
        assert!(!z.join("Saves").exists());
        assert!(!z.join("options.ini").exists());
        assert!(!z.join("mods/Brita").exists());
        assert_eq!(mods::active(&made).mods, ["Brita", "Local"]);
        // Its first install asks for build 7, not the branch's current one.
        assert_eq!(
            store::target(&made.dir),
            Some(store::Target {
                build: 7,
                depots: [(108603, 31), (108604, 41)].into(),
            })
        );
        store::set_build(&made.dir, 7).unwrap();
        assert_eq!(store::target(&made.dir), None, "settled once installed");

        // Following the branch: no depots, nothing asked for.
        let follow = About {
            exact: false,
            ..about
        };
        export(&instance, &follow, &picked, &out).unwrap();
        assert!(read(&out).unwrap().build.depots.is_empty());
        let (made, _) = import(&other, &out).unwrap();
        assert_eq!(store::target(&made.dir), None);
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(&other).unwrap();
    }

    #[test]
    fn explains_every_folder_checked_from_the_start() {
        for picked in PICKED {
            assert!(about(Path::new(picked)).is_some(), "{picked}");
        }
    }

    #[test]
    fn blanks_secrets_in_any_encoding() {
        // Latin-1 with Windows line endings: not UTF-8, cleaned all the same.
        let file = b"PVP=false\r\nPassword=\xf1and\xfa\r\nRCONPassword = x\nPublic=true";
        assert_eq!(
            without_secrets(file),
            b"PVP=false\r\nPassword=\r\nRCONPassword =\nPublic=true"
        );
    }

    #[test]
    fn refuses_files_that_climb_out_of_the_instance() {
        let root = temp("evil");
        let index = Index {
            format: FORMAT,
            game: GAME.into(),
            name: "Evil".into(),
            version: String::new(),
            summary: String::new(),
            build: Build {
                branch: "public".into(),
                id: None,
                depots: Vec::new(),
            },
            workshop: Vec::new(),
            mods: Vec::new(),
            maps: Vec::new(),
            settings: Settings::default(),
            icon: None,
        };
        for bad in ["overrides/../../evil.txt", "/etc/evil.txt"] {
            let path = root.join("evil.lata");
            let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
            zip.start_file(bad, SimpleFileOptions::default()).unwrap();
            zip.write_all(b"x").unwrap();
            zip.start_file(INDEX, SimpleFileOptions::default()).unwrap();
            zip.write_all(&serde_json::to_vec(&index).unwrap()).unwrap();
            zip.finish().unwrap();
            assert!(read(&path).is_err(), "{bad}");
            assert!(import(&root.join("instances"), &path).is_err(), "{bad}");
            assert!(!root.join("instances").exists(), "{bad}");
        }
        std::fs::remove_dir_all(&root).unwrap();
    }
}
