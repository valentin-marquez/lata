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

/// A range of a jar that isn't held yet and has to be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Missing {
    pub start: u64,
    pub len: u64,
}

impl fmt::Display for Missing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "bytes {}..{} aren't held",
            self.start,
            self.start + self.len
        )
    }
}

impl std::error::Error for Missing {}

/// A jar of which only some ranges are held. The zip reader asks for what it needs (the end of
/// the file, the directory, the one class); a range that isn't held fails with [`Missing`], the
/// caller fetches it and tries again. A build's version is read this way without downloading
/// its jar whole.
pub struct Sparse {
    size: u64,
    at: u64,
    /// Held ranges by where they start; they never overlap.
    held: std::collections::BTreeMap<u64, Vec<u8>>,
}

impl Sparse {
    /// A jar of `size` bytes with nothing held.
    pub fn new(size: u64) -> Self {
        Sparse {
            size,
            at: 0,
            held: Default::default(),
        }
    }

    /// Keeps `bytes`, which start at `start` in the jar.
    pub fn hold(&mut self, start: u64, bytes: Vec<u8>) {
        self.held.insert(start, bytes);
    }

    /// How many bytes are held.
    pub fn held(&self) -> u64 {
        self.held.values().map(|b| b.len() as u64).sum()
    }
}

impl Read for Sparse {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() || self.at >= self.size {
            return Ok(0);
        }
        match self.held.range(..=self.at).next_back() {
            Some((start, bytes)) if self.at < start + bytes.len() as u64 => {
                let from = (self.at - start) as usize;
                let n = buf.len().min(bytes.len() - from);
                buf[..n].copy_from_slice(&bytes[from..from + n]);
                self.at += n as u64;
                Ok(n)
            }
            _ => Err(std::io::Error::other(Missing {
                start: self.at,
                len: (buf.len() as u64).min(self.size - self.at),
            })),
        }
    }
}

impl std::io::Seek for Sparse {
    fn seek(&mut self, to: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom::*;
        let next = match to {
            Start(n) => Some(n),
            End(n) => self.size.checked_add_signed(n),
            Current(n) => self.at.checked_add_signed(n),
        };
        self.at = next.ok_or_else(|| std::io::Error::other("seek before the start of the jar"))?;
        Ok(self.at)
    }
}

/// What is missing, if `error` is the reader saying so.
fn missing(error: &std::io::Error) -> Option<Missing> {
    error.get_ref()?.downcast_ref::<Missing>().copied()
}

/// The version in a jar read through [`Sparse`]: `Ok(None)` when the jar has no readable
/// `Core.class`, `Err` with the range to fetch when the reader needs bytes that aren't held.
/// Call again after holding them; each try reads only what it needs.
pub fn from_sparse(jar: &mut Sparse) -> Result<Option<Version>, Missing> {
    use zip::result::ZipError;
    jar.at = 0;
    let mut archive = match zip::ZipArchive::new(&mut *jar) {
        Ok(archive) => archive,
        Err(ZipError::Io(e)) => return missing(&e).map_or(Ok(None), Err),
        Err(_) => return Ok(None),
    };
    let entry = match archive.by_name("zombie/core/Core.class") {
        Ok(entry) => entry,
        Err(ZipError::Io(e)) => return missing(&e).map_or(Ok(None), Err),
        Err(_) => return Ok(None),
    };
    let mut class = Vec::new();
    match entry.take(CLASS_CAP).read_to_end(&mut class) {
        Ok(_) => Ok(from_class(&class)),
        Err(e) => missing(&e).map_or(Ok(None), Err),
    }
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

    /// A jar of about 8 MB with the class in the middle, between entries it doesn't care about.
    fn big_jar() -> Vec<u8> {
        use std::io::Write;
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut jar = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let mut state = 0x2545_f491_4f6c_dd1d_u64;
        let mut filler = |jar: &mut zip::ZipWriter<_>, name: &str, len: usize| {
            let bytes: Vec<u8> = (0..len)
                .map(|_| {
                    state ^= state << 13;
                    state ^= state >> 7;
                    state ^= state << 17;
                    state as u8
                })
                .collect();
            jar.start_file(name, stored).unwrap();
            jar.write_all(&bytes).unwrap();
        };
        filler(&mut jar, "a/First.class", 3 << 20);
        for n in 0..300 {
            filler(&mut jar, &format!("b/Small{n}.class"), 64);
        }
        jar.start_file::<_, ()>("zombie/core/Core.class", Default::default())
            .unwrap();
        jar.write_all(B42).unwrap();
        filler(&mut jar, "c/Last.class", 5 << 20);
        jar.finish().unwrap().into_inner()
    }

    /// Reads the version of `bytes` as a build is read from Steam: `fetch` hands out whole
    /// blocks of `block` bytes, which is how a chunk covers a range. Returns it with how many
    /// bytes were held and how many tries it took.
    fn read_in_blocks(bytes: &[u8], block: u64) -> (Option<Version>, u64, u32) {
        let size = bytes.len() as u64;
        let mut sparse = Sparse::new(size);
        for tries in 1..60 {
            match from_sparse(&mut sparse) {
                Ok(found) => return (found, sparse.held(), tries),
                Err(need) => {
                    let last = (need.start + need.len - 1) / block;
                    for b in need.start / block..=last {
                        let (from, to) = (b * block, ((b + 1) * block).min(size));
                        sparse.hold(from, bytes[from as usize..to as usize].to_vec());
                    }
                }
            }
        }
        panic!("it kept asking for more");
    }

    #[test]
    fn reads_the_version_holding_a_small_part_of_the_jar() {
        let jar = big_jar();
        let (found, held, tries) = read_in_blocks(&jar, 256 << 10);
        assert_eq!(found.unwrap().to_string(), "42.22.2");
        // The end with the directory and the class, not the 8 MB around them.
        assert!(held < jar.len() as u64 / 3, "held {held} of {}", jar.len());
        assert!(tries < 20, "{tries} tries");
        // Whatever the block size, the answer is the same.
        for block in [4 << 10, 1 << 20] {
            let (found, ..) = read_in_blocks(&jar, block);
            assert_eq!(found.unwrap().to_string(), "42.22.2", "block {block}");
        }
    }

    #[test]
    fn a_jar_without_the_class_answers_none_not_missing() {
        use std::io::Write;
        let mut jar = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        jar.start_file::<_, ()>("zombie/Other.class", Default::default())
            .unwrap();
        jar.write_all(b"nothing").unwrap();
        let bytes = jar.finish().unwrap().into_inner();
        assert_eq!(read_in_blocks(&bytes, 64).0, None);
        // Not a zip at all.
        assert_eq!(read_in_blocks(B42, 4096).0, None);
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
