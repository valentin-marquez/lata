//! Links from outside lulo, like `lulo://install-mod?id=<id>`, which the browser extension opens
//! from a Workshop page. Any web page can open one, so nothing here trusts what it says: it is
//! read strictly, the item is checked against Steam, and the user always confirms.

#[path = "protocol.model.rs"]
pub mod protocol;
#[path = "single.model.rs"]
pub mod single;

use crate::instance::Instance;
use crate::instance::mods::{self, Listing};

const SCHEME: &str = "lulo://";
/// The longest link read; a real one is about 40 characters.
const MAX_LEN: usize = 200;
const NOT_OURS: &str = "that isn't a lulo link";

/// What reaches the running lulo from outside: a link, or a second lulo asking it to show itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The text of a link, not yet read (`parse` decides).
    Link(String),
    Show,
}

/// What a link asks for.
#[derive(Debug, PartialEq)]
pub enum Request {
    /// Install the Workshop item with this id.
    InstallMod(u64),
}

/// The links among the arguments lulo was started with. The system hands over the link it was
/// asked to open, maybe next to other arguments; this keeps only what claims to be one of ours
/// (`parse` still decides whether it is valid).
pub fn from_args(args: impl IntoIterator<Item = String>) -> Vec<String> {
    args.into_iter()
        .filter(|arg| {
            arg.get(..SCHEME.len() - 2)
                .is_some_and(|start| start.eq_ignore_ascii_case("lulo:"))
        })
        .collect()
}

/// Reads a link. Only `lulo://install-mod` with exactly one `id=` made of digits is accepted;
/// other parameters are ignored, and anything else is refused with a reason.
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
    if !action
        .trim_end_matches('/')
        .eq_ignore_ascii_case("install-mod")
    {
        return Err("lulo doesn't know that link");
    }
    let mut ids = query.split('&').filter_map(|pair| pair.strip_prefix("id="));
    let (Some(id), None) = (ids.next(), ids.next()) else {
        return Err("the link needs exactly one mod id");
    };
    if id.is_empty() || id.len() > 20 || !id.bytes().all(|b| b.is_ascii_digit()) {
        return Err("the mod id isn't a number");
    }
    id.parse()
        .ok()
        .filter(|id| *id != 0)
        .map(Request::InstallMod)
        .ok_or("the mod id isn't valid")
}

/// The mod a link asks to install, as the Workshop lists it, for the dialog that asks the user.
/// It must be a Project Zomboid mod: a collection, a guide, an illustration or another game's
/// item is refused, whatever the link says.
pub async fn resolve(text: &str) -> Result<Listing, String> {
    let Request::InstallMod(id) = parse(text)?;
    let listing = mods::lookup(id).await?;
    if !listing.item.is_mod() {
        return Err("that Workshop item isn't a mod (it may be a guide or art)".into());
    }
    Ok(listing)
}

/// The instance to have selected in the install dialog: the one a mod went into last, if it
/// still exists; else the only instance there is; else none, so the first time the user chooses.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the link handler (#37) will call it")
)]
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
        assert_eq!(parse("lulo://install-mod?id=3832274571"), id(3832274571));
        // How browsers and systems tend to hand it over.
        for link in [
            "  lulo://install-mod?id=42\n",
            "LULO://Install-Mod?id=42",
            "lulo://install-mod/?id=42",
            "lulo://install-mod?id=42/",
            "lulo://install-mod?id=42#top",
            "lulo://install-mod?from=steam&id=42&v=2",
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
                "lulo://install-mod?id=1",
                "--debug",
                "LULO:install-mod?id=2"
            ])),
            ["lulo://install-mod?id=1", "LULO:install-mod?id=2"]
        );
        assert!(from_args(args(&[])).is_empty());
        // Not links, and not a character boundary where the scheme would end.
        assert!(from_args(args(&["--lulo", "https://lulo.example", "lul", "ñññ"])).is_empty());
    }

    #[test]
    fn refuses_what_is_not_a_clean_install_link() {
        for link in [
            "",
            "lulo://",
            "lulo://install-mod",
            "lulo://install-mod?id=",
            "lulo://install-mod?id=0",
            "lulo://install-mod?id=-1",
            "lulo://install-mod?id=12ab",
            "lulo://install-mod?id=%31%32",
            "lulo://install-mod?id=1.5",
            "lulo://install-mod?id=18446744073709551616",
            "lulo://install-mod?id=000000000000000000001",
            "lulo://install-mod?id=1&id=2",
            "lulo://install-mod?ID=1",
            "lulo://delete-everything?id=1",
            "lulo://install-mod/extra?id=1",
            "lulo:/install-mod?id=1",
            "lulo:install-mod?id=1",
            "xlulo://install-mod?id=1",
            "https://steamcommunity.com/sharedfiles/filedetails/?id=1",
            "lulo://install-mod?id=1 2",
            "lulo://install-mod?id=1\u{0}",
            "lulo://install-mod?id=1\r\nlulo://install-mod?id=2",
            "lulo://install-mod?id=\u{661}",
        ] {
            assert!(parse(link).is_err(), "{link:?}");
        }
        let long = format!("lulo://install-mod?id=1&pad={}", "x".repeat(300));
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
    /// don't. It downloads the mod's picture into lulo's cache, so point `LULO_HOME` at a
    /// scratch folder: `LULO_HOME=/tmp/x cargo test -p lulo -- --ignored link`.
    #[test]
    #[ignore = "needs network access to Steam"]
    fn resolves_only_a_mod_with_files() {
        let run = |link: String| {
            tokio::runtime::Runtime::new()
                .expect("a runtime")
                .block_on(async move { resolve(&link).await })
        };
        let listing = run("lulo://install-mod?id=3832274571".into()).expect("a PZ mod");
        assert_eq!(listing.item.id, 3832274571);
        assert!(!listing.item.title.is_empty());

        // The Workshop's "Modding Policy" guide: a Project Zomboid item, but not a mod.
        let guide = run("lulo://install-mod?id=2872282653".into()).unwrap_err();
        assert!(guide.contains("isn't a mod"), "{guide}");
        // A collection of PZ mods, and an id nobody has.
        assert!(run("lulo://install-mod?id=3834236342".into()).is_err());
        assert!(run("lulo://install-mod?id=1".into()).is_err());
        // A link that isn't ours never gets to Steam.
        assert_eq!(
            run("lulo://nope".into()).unwrap_err(),
            "lulo doesn't know that link"
        );
    }
}
