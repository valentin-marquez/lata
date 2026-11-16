//! Which release is newer than this lata, read from GitHub's list of releases, and the
//! checksum it publishes for a download.

use std::cmp::Ordering;
use std::fmt;

use serde::Deserialize;

const REPO: &str = "valentin-marquez/lata";
/// GitHub's API refuses requests without one.
pub const AGENT: &str = concat!("lata/", env!("CARGO_PKG_VERSION"));
/// The most GitHub's list of releases may weigh; a hundred of them are well under 1 MB.
const LIST_BYTES: u64 = 8 * 1024 * 1024;
/// The file in every release that lists the SHA-256 of the others.
pub const SUMS: &str = "SHA256SUMS";

/// How this system's package ends, after `lata-launcher-<tag>-`; `None` where no package is
/// built, and a newer version is only announced.
pub const PACKAGE: Option<&str> = if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x86_64.tar.gz")
} else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
    Some("windows-x86_64.zip")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    // The .dmg is for people; a .zip of the same Lata.app opens without mounting anything.
    Some("macos-arm64.zip")
} else {
    None
};

/// A semantic version: `0.7.0`, `v0.7.0`, `1.0.0-beta.2`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    core: [u64; 3],
    /// What follows a dash, by its dots; empty for a final version.
    pre: Vec<Part>,
}

/// Numbers sort before words, as semver has it.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Number(u64),
    Word(String),
}

impl Version {
    /// Strict, as semver.org spells it (plus an optional leading `v`): a tag is also put in
    /// a download address, so nothing but its letters, digits, dots and dashes gets through.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('v').unwrap_or(text);
        let word = |part: &str| {
            !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        };
        // No sign, no leading zero.
        let number = |part: &str| {
            let digits = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
            (digits && (part == "0" || !part.starts_with('0')))
                .then(|| part.parse::<u64>().ok())
                .flatten()
        };
        // Build metadata (after a plus) never orders versions.
        let text = match text.split_once('+') {
            Some((text, build)) if build.split('.').all(word) => text,
            Some(_) => return None,
            None => text,
        };
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let mut numbers = core.split('.').map(number);
        let core = [numbers.next()??, numbers.next()??, numbers.next()??];
        if numbers.next().is_some() {
            return None;
        }
        let part = |part: &str| {
            if part.bytes().all(|b| b.is_ascii_digit()) {
                number(part).map(Part::Number)
            } else {
                word(part).then(|| Part::Word(part.to_owned()))
            }
        };
        let pre = match pre {
            Some(pre) => pre.split('.').map(part).collect::<Option<Vec<_>>>()?,
            None => Vec::new(),
        };
        Some(Self { core, pre })
    }

    pub fn is_pre(&self) -> bool {
        !self.pre.is_empty()
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // 1.0.0-beta comes before 1.0.0: having nothing after the dash is the greater one.
        (self.core, !self.is_pre(), &self.pre).cmp(&(other.core, !other.is_pre(), &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let [major, minor, patch] = self.core;
        write!(f, "{major}.{minor}.{patch}")?;
        for (ix, part) in self.pre.iter().enumerate() {
            f.write_str(if ix == 0 { "-" } else { "." })?;
            match part {
                Part::Number(n) => write!(f, "{n}")?,
                Part::Word(word) => f.write_str(word)?,
            }
        }
        Ok(())
    }
}

/// A release newer than the lata that asked.
#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    tag: String,
    /// This system's package and its size in bytes, when the release has one and lists
    /// checksums; without either there is nothing lata could verify, so it only links the page.
    pub package: Option<(String, u64)>,
}

impl Release {
    /// The release's page, for people.
    pub fn page(&self) -> String {
        format!("https://github.com/{REPO}/releases/tag/{}", self.tag)
    }

    /// Where a file of the release downloads from. Built here instead of taken from the API's
    /// answer, so nothing in that answer can point the download somewhere else.
    pub fn file(&self, name: &str) -> String {
        format!(
            "https://github.com/{REPO}/releases/download/{}/{name}",
            self.tag
        )
    }
}

#[derive(Deserialize)]
struct Listed {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct Asset {
    name: String,
    #[serde(default)]
    size: u64,
}

/// The newest release after `current` in GitHub's answer, if there is one. `package` is how
/// this system's download ends ([`PACKAGE`]).
///
/// Pre-releases are for whoever already runs one: a `current` with a dash in it, or one whose
/// own release is listed and marked pre-release. Anyone else only hears about final releases,
/// and that includes a version GitHub doesn't list (a build of your own, a release that was
/// deleted or fell off the page): not knowing must never move someone onto betas. The browser
/// extension's releases (`extension-v*`) live in the same list and are skipped.
pub fn newest(
    json: &[u8],
    current: &Version,
    package: Option<&str>,
) -> Result<Option<Release>, String> {
    let listed: Vec<Listed> = serde_json::from_slice(json).map_err(|err| err.to_string())?;
    let ours: Vec<(Version, Listed)> = listed
        .into_iter()
        .filter(|release| !release.draft && release.tag_name.starts_with('v'))
        .filter_map(|release| Some((Version::parse(&release.tag_name)?, release)))
        .collect();
    let pre = current.is_pre()
        || ours
            .iter()
            .find(|(version, _)| version == current)
            .is_some_and(|(_, release)| release.prerelease);
    let newest = ours
        .into_iter()
        .filter(|(version, release)| {
            version > current && (pre || !(release.prerelease || version.is_pre()))
        })
        .max_by(|a, b| a.0.cmp(&b.0));
    Ok(newest.map(|(version, release)| {
        let sums = release.assets.iter().any(|asset| asset.name == SUMS);
        let wanted = package.map(|end| format!("lata-launcher-{}-{end}", release.tag_name));
        let package = release
            .assets
            .iter()
            .find(|asset| sums && Some(&asset.name) == wanted.as_ref())
            .map(|asset| (asset.name.clone(), asset.size));
        Release {
            version,
            tag: release.tag_name,
            package,
        }
    }))
}

/// Asks GitHub for a release newer than `current`. `/releases/latest` would be one request
/// for one release, but it never answers with a pre-release, which is all there is so far.
pub async fn check(current: &Version) -> Result<Option<Release>, String> {
    let reply = steam::http()
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases?per_page=100"
        ))
        .header("User-Agent", AGENT)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .and_then(|reply| reply.error_for_status())
        .map_err(|err| err.to_string())?;
    let json = super::body(reply, LIST_BYTES, |_| ()).await?;
    newest(&json, current, PACKAGE)
}

/// The SHA-256 a `SHA256SUMS` file (as `sha256sum` writes it) lists for `name`, in lowercase.
pub fn checksum(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, file) = line.split_once(char::is_whitespace)?;
        // `sha256sum` marks a file read as binary with a star before its name.
        let file = file.trim();
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        (valid && file.strip_prefix('*').unwrap_or(file) == name).then(|| hash.to_ascii_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's real answer on 2026-11-15 (one release, v0.6.0), cut down to the fields read
    /// and a few around them.
    const REAL: &str = include_str!("fixtures/releases.json");

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    /// The real answer plus releases made from its first one: `(tag, prerelease, draft)`,
    /// each with every package and a checksum file.
    fn with(more: &[(&str, bool, bool)]) -> Vec<u8> {
        let mut all: Vec<serde_json::Value> = serde_json::from_str(REAL).unwrap();
        for (tag, pre, draft) in more {
            let mut release = all[0].clone();
            release["tag_name"] = (*tag).into();
            release["prerelease"] = (*pre).into();
            release["draft"] = (*draft).into();
            release["assets"] = [
                "linux-x86_64.tar.gz",
                "windows-x86_64.zip",
                "macos-arm64.zip",
            ]
            .iter()
            .map(|end| serde_json::json!({"name": format!("lata-launcher-{tag}-{end}"), "size": 7}))
            .chain([serde_json::json!({"name": SUMS, "size": 300})])
            .collect();
            all.push(release);
        }
        serde_json::to_vec(&all).unwrap()
    }

    #[test]
    fn versions_order_like_semver() {
        let ordered = [
            "0.6.0",
            "0.6.1",
            "0.10.0",
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            assert!(v(pair[0]) < v(pair[1]), "{} < {}", pair[0], pair[1]);
        }
        assert_eq!(v("v0.6.0"), v("0.6.0+linux"));
        assert_eq!(v("v1.0.0-beta.2").to_string(), "1.0.0-beta.2");
        for bad in [
            "",
            "0.6",
            "0.6.0.1",
            "extension-v1.2.0",
            "a.b.c",
            "1.-2.3",
            "+1.2.3",
            "01.2.3",
            "1.0.0-",
            "1.0.0-a..b",
            "1.0.0-01",
            "1.0.0-a b",
            "1.0.0+",
            "1.0.0/../x",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn the_real_answer_holds_the_release_it_was_taken_from() {
        // 0.6.0 is marked pre-release, so only someone on a pre-release hears about it.
        let found = newest(
            REAL.as_bytes(),
            &v("0.5.0-dev"),
            Some("linux-x86_64.tar.gz"),
        )
        .unwrap()
        .expect("0.6.0 is newer than 0.5.0-dev");
        assert!(
            newest(REAL.as_bytes(), &v("0.5.0"), PACKAGE)
                .unwrap()
                .is_none()
        );
        assert_eq!(found.version, v("0.6.0"));
        assert_eq!(
            found.page(),
            "https://github.com/valentin-marquez/lata/releases/tag/v0.6.0"
        );
        // v0.6.0 was published before checksums were: it is announced, never downloaded.
        assert!(found.package.is_none());
        // And whoever runs it has nothing newer.
        assert!(
            newest(REAL.as_bytes(), &v("0.6.0"), PACKAGE)
                .unwrap()
                .is_none()
        );
        assert!(newest(b"{\"message\":\"rate limited\"}", &v("0.6.0"), PACKAGE).is_err());
    }

    #[test]
    fn picks_the_newest_release_and_this_systems_package() {
        let json = with(&[
            ("v0.7.0", true, false),
            ("v0.10.0", true, false),
            ("v0.9.0", true, false),
            // Not published, the extension's, and not a version at all.
            ("v0.11.0", true, true),
            ("extension-v9.0.0", false, false),
            ("vnext", false, false),
        ]);
        let found = newest(&json, &v("0.6.0"), Some("windows-x86_64.zip"))
            .unwrap()
            .unwrap();
        assert_eq!(found.version, v("0.10.0"));
        let (name, size) = found.package.clone().unwrap();
        assert_eq!(name, "lata-launcher-v0.10.0-windows-x86_64.zip");
        assert_eq!(size, 7);
        assert_eq!(
            found.file(&name),
            "https://github.com/valentin-marquez/lata/releases/download/v0.10.0/lata-launcher-v0.10.0-windows-x86_64.zip"
        );
        // A system no package is built for still hears about it.
        let other = newest(&json, &v("0.6.0"), None).unwrap().unwrap();
        assert!(other.package.is_none());
    }

    #[test]
    fn pre_releases_are_for_whoever_runs_one() {
        let json = with(&[
            ("v1.0.0", false, false),
            ("v1.1.0-beta.1", true, false),
            ("v1.2.0", true, false),
        ]);
        let next = |current: &str| {
            newest(&json, &v(current), PACKAGE)
                .unwrap()
                .map(|release| release.version.to_string())
        };
        // 0.6.0 is itself marked pre-release, so it gets everything.
        assert_eq!(next("0.6.0").as_deref(), Some("1.2.0"));
        // On the final 1.0.0 there is nothing final after it.
        assert_eq!(next("1.0.0"), None);
        assert_eq!(next("1.1.0-beta.1").as_deref(), Some("1.2.0"));
        // A version GitHub doesn't list stays on final releases.
        assert_eq!(next("0.9.9").as_deref(), Some("1.0.0"));

        let stable = with(&[("v1.0.0", false, false), ("v1.0.1", false, false)]);
        let found = newest(&stable, &v("1.0.0"), PACKAGE).unwrap().unwrap();
        assert_eq!(found.version, v("1.0.1"));
    }

    #[test]
    fn reads_a_checksum_file() {
        let a = "e4a153ec9f4a060b58082288191ade6e67d3794f980abe2ce9bad11f92e3c1df";
        let b = "5B5D24372192F42A30576BEDE2207E3ACF456F209F6566BA39A5828C7790756E";
        let sums = format!("{a}  lata.tar.gz\n{b} *lata.zip\nnot-a-hash  lata.dmg\n\n");
        assert_eq!(checksum(&sums, "lata.tar.gz").as_deref(), Some(a));
        assert_eq!(checksum(&sums, "lata.zip"), Some(b.to_lowercase()));
        assert_eq!(checksum(&sums, "lata.dmg"), None);
        assert_eq!(checksum(&sums, "lata"), None, "a name must match whole");
    }

    /// `cargo test -p lata -- --ignored asks_github`
    #[test]
    #[ignore = "talks to GitHub"]
    fn asks_github() {
        let asked = crate::runtime::spawn(async { check(&v("0.0.1-dev")).await });
        let found = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(asked)
            .unwrap()
            .unwrap()
            .expect("a release exists");
        assert!(found.version >= v("0.6.0"));
    }
}
