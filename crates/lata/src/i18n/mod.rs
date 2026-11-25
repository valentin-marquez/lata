//! The UI in the user's language. The English text in the code is both the key and the
//! fallback: each other language is a JSON file from that text to its own, and whatever it
//! lacks shows in English.
//!
//! - `t!("Play")` is the text in the current language.
//! - `t!("Played {when}", when = ago)` fills `{name}` placeholders.
//! - `tn!(n, "{n} mod", "{n} mods")` picks the plural form for `n`. A language file keys it by
//!   the English plural; Russian gives its three forms (one, few, many) split by `|`.
//! - `n!("Light")` only marks text for translation where it must stay a constant; `tr` turns it
//!   into the current language where it is shown.

use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

macro_rules! t {
    ($key:literal) => {
        $crate::i18n::tr($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill(
            $crate::i18n::tr($key),
            &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+],
        )
    };
}

macro_rules! tn {
    ($n:expr, $one:literal, $other:literal $(, $name:ident = $value:expr)* $(,)?) => {{
        let n = $n;
        $crate::i18n::fill(
            $crate::i18n::plural(n as u64, $one, $other),
            &[("n", &n as &dyn ::std::fmt::Display) $(, (stringify!($name), &$value as &dyn ::std::fmt::Display))*],
        )
    }};
}

#[allow(unused_macros)]
macro_rules! n {
    ($key:literal) => {
        $key
    };
}

const ZH_CN: &str = include_str!("zh-cn.locale.json");
const RU: &str = include_str!("ru.locale.json");

/// The language picked in Settings.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Language {
    /// The system's language when lata has it, English otherwise.
    #[default]
    #[serde(rename = "system")]
    System,
    #[serde(rename = "en")]
    English,
    #[serde(rename = "zh-CN")]
    Chinese,
    #[serde(rename = "ru")]
    Russian,
}

impl Language {
    pub const ALL: [Language; 4] = [
        Language::System,
        Language::English,
        Language::Chinese,
        Language::Russian,
    ];

    /// Each language by its own name, so anyone finds theirs whatever is showing now.
    pub fn label(self) -> &'static str {
        match self {
            Language::System => t!("System"),
            Language::English => "English",
            Language::Chinese => "简体中文",
            Language::Russian => "Русский",
        }
    }

    pub fn is_system(&self) -> bool {
        *self == Language::System
    }

    /// The language the text shows in.
    fn locale(self) -> Locale {
        match self {
            Language::System => sys_locale::get_locale()
                .map(|tag| Locale::from_tag(&tag))
                .unwrap_or(Locale::En),
            Language::English => Locale::En,
            Language::Chinese => Locale::ZhCn,
            Language::Russian => Locale::Ru,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Locale {
    En,
    ZhCn,
    Ru,
}

impl Locale {
    /// A system locale like `zh-CN`, `zh_CN.UTF-8` or `ru-RU`. Traditional Chinese stays
    /// English rather than showing the simplified script.
    fn from_tag(tag: &str) -> Locale {
        let tag = tag.replace('_', "-").to_ascii_lowercase();
        let lang = tag.split(['-', '.']).next().unwrap_or("");
        let traditional = ["hant", "tw", "hk", "mo"]
            .iter()
            .any(|part| tag.split(['-', '.']).any(|p| p == *part));
        match lang {
            "zh" if !traditional => Locale::ZhCn,
            "ru" => Locale::Ru,
            _ => Locale::En,
        }
    }

    /// The code gpui-component's own strings use.
    fn code(self) -> &'static str {
        match self {
            Locale::En => "en",
            Locale::ZhCn => "zh-CN",
            Locale::Ru => "ru",
        }
    }

    fn table(self) -> Option<&'static HashMap<String, String>> {
        static ZH: OnceLock<HashMap<String, String>> = OnceLock::new();
        static RU_: OnceLock<HashMap<String, String>> = OnceLock::new();
        let parse = |text| serde_json::from_str(text).expect("bundled locale file is valid");
        match self {
            Locale::En => None,
            Locale::ZhCn => Some(ZH.get_or_init(|| parse(ZH_CN))),
            Locale::Ru => Some(RU_.get_or_init(|| parse(RU))),
        }
    }
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

fn current() -> Locale {
    match CURRENT.load(Ordering::Relaxed) {
        1 => Locale::ZhCn,
        2 => Locale::Ru,
        _ => Locale::En,
    }
}

/// Shows text in `language` from now on. Views pick it up when they next render.
pub fn apply(language: Language) {
    let locale = language.locale();
    CURRENT.store(locale as u8, Ordering::Relaxed);
    gpui_kit::component::set_locale(locale.code());
}

/// The saved language, or `LATA_LANG` (`en`, `zh-CN`, `ru`) to try one without picking it.
pub fn init() {
    let pinned = std::env::var("LATA_LANG")
        .ok()
        .and_then(|code| serde_json::from_value(serde_json::Value::String(code)).ok());
    apply(pinned.unwrap_or_else(|| crate::settings::load().language));
}

fn lookup<'a>(locale: Locale, key: &'a str) -> &'a str {
    locale
        .table()
        .and_then(|table| table.get(key))
        .map_or(key, String::as_str)
}

/// `key` (English text) in the current language.
pub fn tr(key: &str) -> &str {
    lookup(current(), key)
}

/// Which of a plural's forms `n` takes: English and Chinese have one and other, Russian one,
/// few and many.
fn plural_in<'a>(locale: Locale, n: u64, one: &'a str, other: &'a str) -> &'a str {
    if locale == Locale::En {
        return if n == 1 { one } else { other };
    }
    let text = lookup(locale, other);
    if text == other {
        return if n == 1 { one } else { other };
    }
    let forms: Vec<&str> = text.split('|').collect();
    let pick = match (locale, forms.len()) {
        (Locale::Ru, 3) => match (n % 10, n % 100) {
            (1, m) if m != 11 => 0,
            (2..=4, m) if !(12..=14).contains(&m) => 1,
            _ => 2,
        },
        _ => 0,
    };
    forms[pick]
}

pub fn plural<'a>(n: u64, one: &'a str, other: &'a str) -> &'a str {
    plural_in(current(), n, one, other)
}

/// `text` with each `{name}` replaced by its value.
pub fn fill(text: &str, values: &[(&str, &dyn Display)]) -> String {
    let mut out = text.to_owned();
    for (name, value) in values {
        out = out.replace(&format!("{{{name}}}"), &value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::path::Path;

    use super::{Locale, RU, ZH_CN, lookup, plural_in};

    /// The text of a string literal starting right after its opening quote, and where it ends.
    fn literal(source: &str) -> Option<(String, usize)> {
        let mut out = String::new();
        let mut chars = source.char_indices();
        while let Some((at, c)) = chars.next() {
            match c {
                '"' => return Some((out, at + 1)),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    '\n' => {
                        // A line continuation skips the next line's indentation.
                        let rest = chars.as_str();
                        let trimmed = rest.trim_start();
                        let skip = rest.len() - trimmed.len();
                        for _ in 0..rest[..skip].chars().count() {
                            chars.next();
                        }
                    }
                    other => out.push(other),
                },
                c => out.push(c),
            }
        }
        None
    }

    /// Every English key the code uses: the literals in `t!`, `n!` and `tn!` (both forms).
    fn keys() -> BTreeSet<String> {
        let mut keys = BTreeSet::new();
        let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for (mac, count) in [("t!(", 1), ("n!(", 1), ("tn!(", 2)] {
                    for (at, _) in text.match_indices(mac) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let mut rest = &text[at + mac.len()..];
                        for _ in 0..count {
                            let Some(open) = rest.find('"') else { break };
                            // `t!(` must start with its literal; `tn!(` has the count first.
                            if mac != "tn!(" && !rest[..open].trim().is_empty() {
                                break;
                            }
                            let Some((key, end)) = literal(&rest[open + 1..]) else {
                                break;
                            };
                            keys.insert(key);
                            rest = &rest[open + 1 + end..];
                        }
                    }
                }
            }
        }
        keys
    }

    fn table(text: &str) -> HashMap<String, String> {
        serde_json::from_str(text).unwrap()
    }

    fn placeholders(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let mut rest = text;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            found.insert(rest[open..open + close + 1].to_owned());
            rest = &rest[open + close + 1..];
        }
        found
    }

    #[test]
    fn translations_match_the_english_in_the_code() {
        let keys = keys();
        assert!(keys.contains("System"), "the scanner finds keys");
        for (name, text) in [("zh-CN", ZH_CN), ("ru", RU)] {
            for (key, value) in table(text) {
                assert!(keys.contains(&key), "{name}: {key:?} isn't in the code");
                for glyph in ['·', '–', '—', '…', '・'] {
                    assert!(!value.contains(glyph), "{name}: {glyph} in {value:?}");
                }
                let forms: Vec<&str> = value.split('|').collect();
                if name == "ru" && forms.len() > 1 {
                    assert_eq!(forms.len(), 3, "ru: one, few and many in {value:?}");
                }
                for form in forms {
                    assert_eq!(
                        placeholders(form),
                        placeholders(&key),
                        "{name}: placeholders of {key:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn reads_system_locales() {
        assert_eq!(Locale::from_tag("zh-CN"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh_CN.UTF-8"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh-Hans-SG"), Locale::ZhCn);
        assert_eq!(Locale::from_tag("zh-TW"), Locale::En);
        assert_eq!(Locale::from_tag("ru-RU"), Locale::Ru);
        assert_eq!(Locale::from_tag("en-US"), Locale::En);
        assert_eq!(Locale::from_tag("es-CL"), Locale::En);
    }

    #[test]
    fn falls_back_to_english_and_picks_russian_plurals() {
        assert_eq!(lookup(Locale::Ru, "not a key"), "not a key");
        let other = "{n} mods";
        let one = "{n} mod";
        assert_eq!(plural_in(Locale::En, 1, one, other), one);
        assert_eq!(plural_in(Locale::En, 2, one, other), other);
        if let Some(forms) = table(RU).get(other) {
            let forms: Vec<&str> = forms.split('|').collect();
            for (n, form) in [(1, 0), (21, 0), (2, 1), (24, 1), (5, 2), (11, 2), (12, 2)] {
                assert_eq!(plural_in(Locale::Ru, n, one, other), forms[form], "{n}");
            }
        }
    }
}
