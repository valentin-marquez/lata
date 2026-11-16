//! The drives an instance library can go on, as the OS reports them, with what they mean for
//! the game: fast or slow, fixed or removable, roomy or full.

use std::path::{Path, PathBuf};

use sysinfo::{DiskKind, Disks};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Nvme,
    Ssd,
    Hdd,
    /// A USB stick or card: the instances vanish while it is unplugged.
    Removable,
    Unknown,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Drive {
    /// What people call it: the volume's label, "System" for Linux's `/`, else the mount's name.
    pub name: String,
    pub mount: PathBuf,
    pub total: u64,
    pub free: u64,
    pub kind: Kind,
}

/// Mounts that hold the system's own things, never a library.
const SKIP: [&str; 9] = [
    "/boot", "/efi", "/snap", "/proc", "/sys", "/dev", "/var/lib", "/var/log", "/tmp",
];

/// Filesystems that are read only or live in memory.
const SKIP_FS: [&str; 6] = [
    "squashfs", "tmpfs", "overlay", "efivarfs", "devtmpfs", "ramfs",
];

/// Smaller than this can't hold a game build.
const TINY: u64 = 2 << 30;

/// The drives worth offering, the system one first. One per device: Linux mounts a btrfs
/// volume many times (subvolumes), and only its shortest mount is shown.
pub fn list() -> Vec<Drive> {
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<(String, Drive)> = Vec::new();
    for disk in disks.list() {
        let mount = disk.mount_point().to_owned();
        let fs = disk.file_system().to_string_lossy();
        if disk.total_space() < TINY
            || SKIP_FS.contains(&fs.as_ref())
            || SKIP.iter().any(|skip| mount.starts_with(skip))
        {
            continue;
        }
        let device = disk.name().to_string_lossy().into_owned();
        let drive = Drive {
            name: label(&device, &mount),
            kind: kind(&device, disk.kind(), disk.is_removable()),
            total: disk.total_space(),
            free: disk.available_space(),
            mount,
        };
        match out
            .iter_mut()
            .find(|(d, _)| *d == device && !device.is_empty())
        {
            Some((_, kept)) if drive.mount.as_os_str().len() < kept.mount.as_os_str().len() => {
                *kept = drive;
            }
            Some(_) => {}
            None => out.push((device, drive)),
        }
    }
    let mut drives: Vec<Drive> = out.into_iter().map(|(_, d)| d).collect();
    drives.sort_by_key(|d| (d.mount.components().count(), d.mount.clone()));
    drives
}

fn kind(device: &str, kind: DiskKind, removable: bool) -> Kind {
    if removable {
        Kind::Removable
    } else if device.contains("nvme") {
        Kind::Nvme
    } else {
        match kind {
            DiskKind::SSD => Kind::Ssd,
            DiskKind::HDD => Kind::Hdd,
            DiskKind::Unknown(_) => Kind::Unknown,
        }
    }
}

fn label(device: &str, mount: &Path) -> String {
    // Windows and macOS name a volume ("Windows", "Macintosh HD"); Linux gives `/dev/...`.
    if !device.is_empty() && !device.starts_with('/') {
        return device.to_owned();
    }
    if mount.parent().is_none() {
        return "System".into();
    }
    mount
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| mount.display().to_string())
}

/// The drive `path` is on: the one with the longest mount that contains it.
pub fn of<'a>(drives: &'a [Drive], path: &Path) -> Option<&'a Drive> {
    drives
        .iter()
        .filter(|d| path.starts_with(&d.mount))
        .max_by_key(|d| d.mount.as_os_str().len())
}

/// Where a library goes on `drive` when nobody picks a folder: the app's data folder on its own
/// drive, else `lata` at the drive's top.
pub fn folder(drive: &Drive, drives: &[Drive], data: &Path) -> PathBuf {
    if of(drives, data).is_some_and(|d| d.mount == drive.mount) {
        data.to_owned()
    } else {
        drive.mount.join("lata")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drive(mount: &str) -> Drive {
        Drive {
            name: label("/dev/sda1", Path::new(mount)),
            mount: mount.into(),
            total: 0,
            free: 0,
            kind: Kind::Unknown,
        }
    }

    #[test]
    fn finds_the_innermost_drive_and_its_folder() {
        let drives = [drive("/"), drive("/mnt/datos")];
        let data = Path::new("/home/v/.local/share/lata");
        assert_eq!(of(&drives, data).unwrap().mount, Path::new("/"));
        let games = of(&drives, Path::new("/mnt/datos/games")).unwrap();
        assert_eq!(games.name, "datos");
        assert_eq!(drives[0].name, "System");
        assert_eq!(folder(&drives[0], &drives, data), data);
        assert_eq!(
            folder(&drives[1], &drives, data),
            Path::new("/mnt/datos/lata")
        );
    }

    #[test]
    fn tells_nvme_from_the_device_name() {
        assert_eq!(kind("/dev/nvme0n1p2", DiskKind::SSD, false), Kind::Nvme);
        assert_eq!(kind("/dev/sdb1", DiskKind::HDD, false), Kind::Hdd);
        assert_eq!(kind("/dev/sdc1", DiskKind::SSD, true), Kind::Removable);
    }

    /// Prints what this machine reports, to eyeball the filtering.
    #[test]
    #[ignore]
    fn prints_this_machines_drives() {
        for d in list() {
            println!("{d:?}");
        }
    }
}
