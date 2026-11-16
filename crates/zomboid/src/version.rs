//! The game version (42.22.2) of a build, read from its files. Steam only knows build ids.
//!
//! `zombie/core/Core.class` holds it: its static initializer stores
//! `new GameVersion(major, minor, suffix)` into the field `gameVersion`, and the field
//! `buildVersion` is a constant with the patch. Build 42 ships the class inside
//! `projectzomboid.jar`; build 41 ships it loose.

use std::fmt;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    /// Usually empty; the game prints it right after the minor.
    pub suffix: String,
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Version {
            major,
            minor,
            patch,
            suffix,
        } = self;
        write!(f, "{major}.{minor}{suffix}.{patch}")
    }
}

impl std::str::FromStr for Version {
    type Err = ();

    /// Reads "42.22.2" (what [`Version`] prints, without a suffix).
    fn from_str(text: &str) -> Result<Self, ()> {
        let mut parts = text.split('.').map(|p| p.parse::<u32>().map_err(drop));
        let version = Version {
            major: parts.next().ok_or(())??,
            minor: parts.next().ok_or(())??,
            patch: parts.next().ok_or(())??,
            suffix: String::new(),
        };
        match parts.next() {
            None => Ok(version),
            Some(_) => Err(()),
        }
    }
}

/// Whether a file of a build (a manifest path, `/`-separated) holds the version.
pub fn is_source(path: &str) -> bool {
    path.ends_with("projectzomboid.jar") || path.ends_with("zombie/core/Core.class")
}

/// The version in `path`: `projectzomboid.jar` or a loose `Core.class`.
pub fn read(path: &Path) -> Option<Version> {
    let bytes = std::fs::read(path).ok()?;
    if path.extension().is_some_and(|e| e == "jar") {
        from_jar(&bytes)
    } else {
        from_class(&bytes)
    }
}

/// Caps what is read out of the jar, in case the entry is not what it says.
const CLASS_CAP: u64 = 4 << 20;

pub fn from_jar(bytes: &[u8]) -> Option<Version> {
    let mut jar = zip::ZipArchive::new(std::io::Cursor::new(bytes)).ok()?;
    let entry = jar.by_name("zombie/core/Core.class").ok()?;
    let mut class = Vec::new();
    entry.take(CLASS_CAP).read_to_end(&mut class).ok()?;
    from_class(&class)
}

/// One constant pool entry, as far as this needs.
enum Constant {
    Utf8(String),
    Integer(i32),
    /// Class (name), String (text).
    Ref(u16),
    /// Fieldref and Methodref: class, name-and-type.
    Member(u16, u16),
    NameAndType(u16, u16),
    Other,
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let out = self.bytes.get(self.at..self.at.checked_add(n)?)?;
        self.at += n;
        Some(out)
    }
    fn u8(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }
}

struct Pool(Vec<Constant>);

impl Pool {
    fn get(&self, ix: u16) -> Option<&Constant> {
        self.0.get(usize::from(ix))
    }
    fn utf8(&self, ix: u16) -> Option<&str> {
        match self.get(ix)? {
            Constant::Utf8(s) => Some(s),
            _ => None,
        }
    }
    fn text(&self, ix: u16) -> Option<&str> {
        match self.get(ix)? {
            Constant::Ref(s) => self.utf8(*s),
            _ => None,
        }
    }
    /// (class, name, descriptor) of a Fieldref or Methodref.
    fn member(&self, ix: u16) -> Option<(&str, &str, &str)> {
        let Constant::Member(class, nat) = self.get(ix)? else {
            return None;
        };
        let Constant::NameAndType(name, desc) = self.get(*nat)? else {
            return None;
        };
        Some((self.text(*class)?, self.utf8(*name)?, self.utf8(*desc)?))
    }
}

/// The version in the bytes of `Core.class`, or `None` when the class doesn't have it the way
/// the game has written it so far (a refactor would break this; callers fall back to the build).
pub fn from_class(bytes: &[u8]) -> Option<Version> {
    let mut r = Reader { bytes, at: 0 };
    if r.u32()? != 0xCAFE_BABE {
        return None;
    }
    r.take(4)?; // minor, major
    let count = r.u16()?;
    let mut pool = vec![Constant::Other];
    while pool.len() < usize::from(count) {
        let tag = r.u8()?;
        let constant = match tag {
            1 => {
                let len = r.u16()?;
                // Modified UTF-8; the names we look for are ASCII.
                Constant::Utf8(String::from_utf8_lossy(r.take(len.into())?).into_owned())
            }
            3 => Constant::Integer(r.u32()? as i32),
            4 => {
                r.u32()?;
                Constant::Other
            }
            5 | 6 => {
                // Longs and doubles take two slots.
                r.take(8)?;
                pool.push(Constant::Other);
                Constant::Other
            }
            7 | 8 => Constant::Ref(r.u16()?),
            9..=11 => Constant::Member(r.u16()?, r.u16()?),
            12 => Constant::NameAndType(r.u16()?, r.u16()?),
            15 => {
                r.take(3)?;
                Constant::Other
            }
            16 | 19 | 20 => {
                r.u16()?;
                Constant::Other
            }
            17 | 18 => {
                r.u32()?;
                Constant::Other
            }
            _ => return None,
        };
        pool.push(constant);
    }
    let pool = Pool(pool);
    r.take(6)?; // access, this, super
    let interfaces = r.u16()?;
    r.take(usize::from(interfaces) * 2)?;

    // Fields: `buildVersion`'s ConstantValue is the patch.
    let mut patch = None;
    for _ in 0..r.u16()? {
        r.u16()?;
        let name = r.u16()?;
        r.u16()?;
        for _ in 0..r.u16()? {
            let (attr, len) = (r.u16()?, r.u32()?);
            let data = r.take(len as usize)?;
            if pool.utf8(name) == Some("buildVersion")
                && pool.utf8(attr) == Some("ConstantValue")
                && let Some(Constant::Integer(n)) =
                    pool.get(u16::from_be_bytes(data.get(..2)?.try_into().ok()?))
            {
                patch = u32::try_from(*n).ok();
            }
        }
    }

    // Methods: the static initializer's code holds the GameVersion constructor call.
    let mut clinit = None;
    for _ in 0..r.u16()? {
        r.u16()?;
        let name = r.u16()?;
        r.u16()?;
        for _ in 0..r.u16()? {
            let (attr, len) = (r.u16()?, r.u32()?);
            let data = r.take(len as usize)?;
            if pool.utf8(name) == Some("<clinit>") && pool.utf8(attr) == Some("Code") {
                let len = u32::from_be_bytes(data.get(4..8)?.try_into().ok()?) as usize;
                clinit = Some(data.get(8..8 + len)?);
            }
        }
    }
    let (major, minor, suffix) = game_version(clinit?, &pool)?;
    Some(Version {
        major,
        minor,
        patch: patch?,
        suffix,
    })
}

/// Finds `push major; push minor; ldc suffix; invokespecial GameVersion.<init>(IILString;)V;
/// putstatic gameVersion` in `code`. Matched at every offset rather than decoded instruction by
/// instruction: the constant pool checks make a false match practically impossible.
fn game_version(code: &[u8], pool: &Pool) -> Option<(u32, u32, String)> {
    (0..code.len()).find_map(|at| {
        let (major, at) = push(code, at)?;
        let (minor, at) = push(code, at)?;
        let (suffix, at) = match *code.get(at)? {
            0x12 => (u16::from(*code.get(at + 1)?), at + 2), // ldc
            0x13 => (index(code, at + 1)?, at + 3),          // ldc_w
            _ => return None,
        };
        if *code.get(at)? != 0xb7 || *code.get(at + 3)? != 0xb3 {
            return None; // invokespecial, putstatic
        }
        let init = pool.member(index(code, at + 1)?)?;
        let field = pool.member(index(code, at + 4)?)?;
        (init
            == (
                "zombie/core/GameVersion",
                "<init>",
                "(IILjava/lang/String;)V",
            )
            && field.1 == "gameVersion")
            .then(|| Some((major, minor, pool.text(suffix)?.to_owned())))?
    })
}

fn index(code: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([*code.get(at)?, *code.get(at + 1)?]))
}

/// A non-negative int pushed at `at` (iconst, bipush, sipush) and where the next one starts.
fn push(code: &[u8], at: usize) -> Option<(u32, usize)> {
    match *code.get(at)? {
        op @ 0x03..=0x08 => Some((u32::from(op - 0x03), at + 1)),
        0x10 => Some((u32::try_from(*code.get(at + 1)? as i8).ok()?, at + 2)),
        0x11 => Some((u32::try_from(index(code, at + 1)? as i16).ok()?, at + 3)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Core.class of the dedicated server's `public` and `legacy41` branches on 2026-11-16, the
    // same releases as the game's (news: "42.22.2 STABLE", branch: "Build 41.78.21").
    const B42: &[u8] = include_bytes!("testdata/core-42.22.2.class");
    const B41: &[u8] = include_bytes!("testdata/core-41.78.21.class");

    #[test]
    fn reads_the_version_of_both_build_lines() {
        assert_eq!(from_class(B42).unwrap().to_string(), "42.22.2");
        assert_eq!(from_class(B41).unwrap().to_string(), "41.78.21");
    }

    #[test]
    fn reads_it_out_of_the_jar() {
        use std::io::Write;
        let mut jar = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        jar.start_file::<_, ()>("zombie/core/Core.class", Default::default())
            .unwrap();
        jar.write_all(B42).unwrap();
        let bytes = jar.finish().unwrap().into_inner();
        assert_eq!(from_jar(&bytes).unwrap().to_string(), "42.22.2");
    }

    #[test]
    fn refuses_what_is_not_the_class() {
        assert_eq!(from_class(b""), None);
        assert_eq!(from_class(&B42[..B42.len() / 2]), None);
        assert_eq!(from_jar(B42), None);
    }

    #[test]
    fn parses_and_orders_versions() {
        let v: Version = "42.22.2".parse().unwrap();
        assert_eq!(v.to_string(), "42.22.2");
        assert!(v < "42.23.0".parse().unwrap());
        assert!("41.78.21".parse::<Version>().unwrap() < v);
        assert!("42.22".parse::<Version>().is_err());
        assert!(is_source("java/projectzomboid.jar"));
        assert!(is_source("zombie/core/Core.class"));
        assert!(!is_source("zombie/core/Core$1.class"));
    }
}
