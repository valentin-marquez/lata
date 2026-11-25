//! Links from outside lata, like `lata://install-mod?id=<id>` and
//! `lata://install-collection?id=<id>`, which the browser extension opens from a Workshop page. Any web page can open one, so nothing here trusts what it says: it is
//! read strictly, the item is checked against Steam, and the user always confirms.

#[path = "browser.model.rs"]
pub mod browser;
#[path = "install.view.rs"]
pub mod install;
#[path = "protocol.model.rs"]
pub mod protocol;
#[path = "single.model.rs"]
pub mod single;

use crate::instance::Instance;
use crate::instance::mods::{self, Bundle, Listing};

const SCHEME: &str = "lata://";
/// The longest link read; a real one is about 40 characters.
const MAX_LEN: usize = 200;
const NOT_OURS: &str = "that isn't a lata link";

/// What reaches the running lata from outside: a link, or a second lata asking it to show itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The text of a link, not yet read (`parse` decides).
    Link(String),
    Show,
    /// From the tray's menu: quit lata, even with its window hidden.
    #[cfg_attr(
        not(any(windows, target_os = "linux")),
        expect(dead_code, reason = "only the Windows and Linux trays send it")
    )]
    Quit,
}

/// What a link asks for.
#[derive(Debug, PartialEq)]
pub enum Request {
    /// Install the Workshop item with this id.
    InstallMod(u64),
    /// Install the mods of the Workshop collection with this id.
    InstallCollection(u64),
}

/// What a link points at, as the Workshop lists it, for the dialog that asks the user.
#[derive(Debug, Clone)]
pub enum Found {
    Mod(Listing),
    Collection(Bundle),
}

impl Found {
    /// Its Workshop id.
    pub fn id(&self) -> u64 {
        match self {
            Found::Mod(found) => found.item.id,
            Found::Collection(found) => found.id,
        }
    }
}

/// The links among the arguments lata was started with. The system hands over the link it was
/// asked to open, maybe next to other arguments; this keeps only what claims to be one of ours
/// (`parse` still decides whether it is valid).
pub fn from_args(args: impl IntoIterator<Item = String>) -> Vec<String> {
    args.into_iter()
        .filter(|arg| {
            arg.get(..SCHEME.len() - 2)
                .is_some_and(|start| start.eq_ignore_ascii_case("lata:"))
        })
        .collect()
}

/// Reads a link. Only `lata://install-mod` and `lata://install-collection` with exactly one
/// `id=` made of digits are accepted; other parameters are ignored, and anything else is
/// refused with a reason.
pub fn parse(text: &str) -> Result<Request, &'static str> {
    let text = text.trim();
    if text.len() > MAX_LEN || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(NOT_OURS);
    }
    let rest = text
        .get(..SCHEME.len())
        .filter(|start| start.eq_ignore_ascii_case(SCHEME))
        .map(|_| &text[SCHEME.len()..])
        .ok_or(NOT_OURS)?;
    // Some systems add a slash at the end, and a fragment never matters.
    let rest = rest.split('#').next().unwrap_or("").trim_end_matches('/');
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    let action = action.trim_end_matches('/');
    let request: fn(u64) -> Request = if action.eq_ignore_ascii_case("install-mod") {
        Request::InstallMod
    } else if action.eq_ignore_ascii_case("install-collection") {
        Request::InstallCollection
    } else {
        return Err("lata doesn't know that link");
    };
    let mut ids = query.split('&').filter_map(|pair| pair.strip_prefix("id="));
    let (Some(id), None) = (ids.next(), ids.next()) else {
        return Err("the link needs exactly one Workshop id");
    };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("the Workshop id isn't a number");
    }
    id.parse()
        .ok()
        .filter(|id| *id != 0)
        .map(request)
        .ok_or("the Workshop id isn't valid")
}

/// What a link asks to install, as the Workshop lists it, for the dialog that asks the user.
/// The link's word for it must be true, whatever else it says: a mod link needs a Project
/// Zomboid mod (not a collection, a guide, an illustration or another game's item), and a
/// collection link needs a collection with Project Zomboid mods in it.
pub async fn resolve(text: &str) -> Result<Found, String> {
    match parse(text)? {
        Request::InstallMod(id) => {
            let listing = mods::lookup(id).await?;
            if !listing.item.is_mod() {
                return Err("that Workshop item isn't a mod (it may be a guide or art)".into());
            }
            Ok(Found::Mod(listing))
        }
        Request::InstallCollection(id) => {
            let bundle = mods::collection(id).await?;
            // Of either build line: which ones fit is decided once the instance is known.
            let (modern, legacy) = bundle.split(true);
            if modern.is_empty() && legacy.is_empty() {
                return Err("that collection has no Project Zomboid mods".into());
            }
            Ok(Found::Collection(bundle))
        }
    }
}

/// The instance to have selected in the install dialog: the one a mod went into last, if it
/// still exists; else the only instance there is; else none, so the first time the user chooses.
pub fn preselect(instances: &[Instance], remembered: Option<&str>) -> Option<usize> {
    remembered
        .and_then(|id| instances.iter().position(|i| i.id == id))
        .or((instances.len() == 1).then_some(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u64) -> Result<Request, &'static str> {
        Ok(Request::InstallMod(n))
    }

    #[test]
    fn reads_an_install_link() {
        assert_eq!(parse("lata://install-mod?id=3832274571"), id(3832274571));
        // A collection has its own word, read the same way.
        for link in [
            "lata://install-collection?id=3834236342",
            "LATA://Install-Collection/?id=3834236342#top",
        ] {
            assert_eq!(
                parse(link),
                Ok(Request::InstallCollection(3834236342)),
                "{link}"
            );
        }
        // How browsers and systems tend to hand it over.
        for link in [
            "  lata://install-mod?id=42\n",
            "LATA://Install-Mod?id=42",
            "lata://install-mod/?id=42",
            "lata://install-mod?id=42/",
            "lata://install-mod?id=42#top",
            "lata://install-mod?from=steam&id=42&v=2",
        ] {
            assert_eq!(parse(link), id(42), "{link}");
        }
    }

    #[test]
    fn keeps_only_the_links_among_the_arguments() {
        fn args(list: &[&str]) -> Vec<String> {
            list.iter().map(|a| a.to_string()).collect()
        }
        assert_eq!(
            from_args(args(&[
                "lata://install-mod?id=1",
                "--debug",
                "LATA:install-mod?id=2"
            ])),
            ["lata://install-mod?id=1", "LATA:install-mod?id=2"]
        );
        assert!(from_args(args(&[])).is_empty());
        // Not links, and not a character boundary where the scheme would end.
        assert!(from_args(args(&["--lata", "https://lata.example", "lul", "ñññ"])).is_empty());
    }

    #[test]
    fn refuses_what_is_not_a_clean_install_link() {
        for link in [
            "",
            "lata://",
            "lata://install-mod",
            "lata://install-mod?id=",
            "lata://install-mod?id=0",
            "lata://install-mod?id=-1",
            "lata://install-mod?id=12ab",
            "lata://install-mod?id=%31%32",
            "lata://install-mod?id=1.5",
            "lata://install-mod?id=18446744073709551616",
            "lata://install-mod?id=000000000000000000001",
            "lata://install-mod?id=1&id=2",
            "lata://install-mod?ID=1",
            "lata://delete-everything?id=1",
            "lata://install-collection",
            "lata://install-collection?id=abc",
            "lata://install-collections?id=1",
            "lata://install-mod/extra?id=1",
            "lata:/install-mod?id=1",
            "lata:install-mod?id=1",
            "xlata://install-mod?id=1",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=1",
            "lata://install-mod?id=1 2",
            "lata://install-mod?id=1\u{0}",
            "lata://install-mod?id=1\r\nlata://install-mod?id=2",
            "lata://install-mod?id=\u{661}",
        ] {
            assert!(parse(link).is_err(), "{link:?}");
        }
        let long = format!("lata://install-mod?id=1&pad={}", "x".repeat(300));
        assert!(parse(&long).is_err(), "a link that long is not ours");
    }

    fn instance(id: &str) -> Instance {
        Instance {
            id: id.into(),
            dir: std::path::PathBuf::new(),
            name: id.into(),
            branch: "public".into(),
            build: None,
            last_played: None,
            icon: None,
            memory_gb: None,
            jvm_args: String::new(),
            launch_args: String::new(),
        }
    }

    #[test]
    fn preselects_the_last_instance_when_it_still_exists() {
        let two = [instance("a"), instance("b")];
        assert_eq!(preselect(&two, Some("b")), Some(1));
        // First time, or the remembered one is gone: with several, the user chooses.
        assert_eq!(preselect(&two, None), None);
        assert_eq!(preselect(&two, Some("deleted")), None);
        // With only one there is nothing to choose.
        let one = [instance("a")];
        assert_eq!(preselect(&one, None), Some(0));
        assert_eq!(preselect(&one, Some("deleted")), Some(0));
        assert_eq!(preselect(&[], Some("a")), None);
    }

    /// Against Steam: a mod resolves; a guide of the same game, a collection and a made-up id
    /// don't. It downloads the mod's picture into lata's cache, so point `LATA_HOME` at a
    /// scratch folder: `LATA_HOME=/tmp/x cargo test -p lata -- --ignored link`.
    #[test]
    #[ignore = "needs network access to Steam"]
    fn resolves_only_a_mod_with_files() {
        let run = |link: String| {
            tokio::runtime::Runtime::new()
                .expect("a runtime")
                .block_on(async move { resolve(&link).await })
        };
        let Found::Mod(listing) = run("lata://install-mod?id=3832274571".into()).expect("a PZ mod")
        else {
            panic!("a mod link resolves to a mod");
        };
        assert_eq!(listing.item.id, 3832274571);
        assert!(!listing.item.title.is_empty());

        // A collection resolves only through its own link, and a mod isn't one.
        let Found::Collection(bundle) =
            run("lata://install-collection?id=3834236342".into()).expect("a PZ collection")
        else {
            panic!("a collection link resolves to a collection");
        };
        assert_eq!(bundle.id, 3834236342);
        assert!(!bundle.mods.is_empty());
        assert!(run("lata://install-collection?id=3832274571".into()).is_err());

        // The Workshop's "Modding Policy" guide: a Project Zomboid item, but not a mod.
        let guide = run("lata://install-mod?id=2872282653".into()).unwrap_err();
        assert!(guide.contains("isn't a mod"), "{guide}");
        // A collection of PZ mods, and an id nobody has.
        assert!(run("lata://install-mod?id=3834236342".into()).is_err());
        assert!(run("lata://install-mod?id=1".into()).is_err());
        // A link that isn't ours never gets to Steam.
        assert_eq!(
            run("lata://nope".into()).unwrap_err(),
            "lata doesn't know that link"
        );
    }
}
