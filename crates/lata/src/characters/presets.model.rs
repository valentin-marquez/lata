//! The game's character presets: `saved_outfits.txt` (looks) and `saved_builds.txt`
//! (profession and traits), in the instance's `zomboid/Lua/`.
//!
//! Both are `name:value` per line, written by `CharacterCreationMain` and
//! `CharacterCreationProfession` (`BCRC`) in the game's Lua. Outfits start with `VERSION=1` and
//! the game reads only the lines under that version. An outfit value is `key=value;...`
//! (`gender`, `skincolor`, `name`, `hair`, `chesthair`, `beard`, then one `Slot=Module.Item|tint`
//! per piece of clothing); B42 adds `stubble`, `beardstubble`, `voiceStyle` and `voicePitch`.
//! B41's loader skips keys it doesn't know (they fall through to its clothing-slot lookup and
//! match nothing), so B42 lines load in B41 as they are. A build value is
//! `profession;trait;trait;...`. Values are kept as written, so nothing is lost on the way.

use std::fmt;

/// Which of the two files.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Outfits,
    Builds,
}

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Outfits, Kind::Builds];

    pub fn file(self) -> &'static str {
        match self {
            Kind::Outfits => "saved_outfits.txt",
            Kind::Builds => "saved_builds.txt",
        }
    }
}

/// The outfit file version the game reads; both B41 and B42 write 1.
const VERSION: &str = "VERSION=1";

#[derive(Clone, Debug, PartialEq)]
enum Line {
    Preset(String, String),
    /// The version line, blank lines and anything the game wouldn't read, written back as is.
    Other(String),
}

/// A preset file, line by line in its own order.
#[derive(Clone, Debug, PartialEq)]
pub struct Presets {
    kind: Kind,
    lines: Vec<Line>,
}

impl Presets {
    pub fn parse(kind: Kind, text: &str) -> Self {
        // Like the game: outfits count only under `VERSION=1`, builds have no version.
        let mut read = kind == Kind::Builds;
        let lines = text
            .lines()
            .map(|line| {
                if kind == Kind::Outfits && line.starts_with("VERSION=") {
                    read = line == VERSION;
                    return Line::Other(line.to_owned());
                }
                match line.split_once(':') {
                    Some((name, value)) if read && !name.is_empty() => {
                        Line::Preset(name.to_owned(), value.to_owned())
                    }
                    _ => Line::Other(line.to_owned()),
                }
            })
            .collect();
        Self { kind, lines }
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Presets by name and value; a repeated name counts once, its last value (the game's
    /// table keeps the last one too).
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        let later = |i: usize, name: &str| {
            self.lines[i + 1..]
                .iter()
                .any(|l| matches!(l, Line::Preset(n, _) if n == name))
        };
        self.lines
            .iter()
            .enumerate()
            .filter_map(move |(i, line)| match line {
                Line::Preset(name, value) if !later(i, name) => {
                    Some((name.as_str(), value.as_str()))
                }
                _ => None,
            })
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.lines.iter().rev().find_map(|line| match line {
            Line::Preset(n, value) if n == name => Some(value.as_str()),
            _ => None,
        })
    }

    /// Sets a preset, in its place if the name is there, else at the end.
    pub fn set(&mut self, name: &str, value: &str) {
        let found = self.lines.iter_mut().rev().find_map(|line| match line {
            Line::Preset(n, v) if n == name => Some(v),
            _ => None,
        });
        match found {
            Some(v) => value.clone_into(v),
            None => {
                // The game reads only under `VERSION=1`: the end must be under it.
                let last = self.lines.iter().rev().find_map(|line| match line {
                    Line::Other(text) if text.starts_with("VERSION=") => Some(text),
                    _ => None,
                });
                match last {
                    _ if self.kind == Kind::Builds => {}
                    None => self.lines.insert(0, version()),
                    Some(text) if text != VERSION => self.lines.push(version()),
                    Some(_) => {}
                }
                self.lines
                    .push(Line::Preset(name.to_owned(), value.to_owned()));
            }
        }
    }

    /// Rewrites every preset's value through `f`, in place.
    pub fn map(&mut self, f: impl Fn(&str) -> String) {
        for line in &mut self.lines {
            if let Line::Preset(_, value) = line {
                *value = f(value);
            }
        }
    }

    /// Drops every preset named `name`. True if there was one.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.lines.len();
        self.lines
            .retain(|line| !matches!(line, Line::Preset(n, _) if n == name));
        self.lines.len() != before
    }

    /// Takes every preset of `other`, its value winning on a shared name; presets only here
    /// stay. True if anything changed.
    pub fn merge(&mut self, other: &Presets) -> bool {
        let mut changed = false;
        for (name, value) in other.iter() {
            if self.get(name) != Some(value) {
                self.set(name, value);
                changed = true;
            }
        }
        changed
    }
}

fn version() -> Line {
    Line::Other(VERSION.to_owned())
}

impl fmt::Display for Presets {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for line in &self.lines {
            match line {
                Line::Preset(name, value) => writeln!(f, "{name}:{value}")?,
                Line::Other(text) => writeln!(f, "{text}")?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines as `saveBuildStep2` builds them: B42 with stubble and voice, B41 without.
    const B42_OUTFIT: &str = "Kate Smith:gender=1;skincolor=0.88,0.73,0.62;name=Kate|Smith;\
        hair=Bob|0.32,0.2,0.1;stubble=2;voiceStyle=Female_1;voicePitch=0.15;\
        Tshirt=Base.Tshirt_DefaultTEXTURE_TINT|0.5,0.1,0.1;Pants=Base.Trousers_Denim|2;\
        Shoes=Base.Shoes_TrainerTINT|0.9,0.9,0.9;";
    const B41_OUTFIT: &str = "Bob:gender=2;skincolor=0.6,0.45,0.35;name=Bob|Jones;\
        hair=Short|0.1,0.1,0.1;chesthair=1;beard=Goatee;Tshirt=Base.Tshirt_WhiteTINT|1,1,1;";

    #[test]
    fn reads_and_writes_outfits_unchanged() {
        let text = format!("VERSION=1\n{B42_OUTFIT}\n{B41_OUTFIT}\n");
        let presets = Presets::parse(Kind::Outfits, &text);
        assert_eq!(presets.to_string(), text);
        let names: Vec<_> = presets.iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["Kate Smith", "Bob"]);
        assert_eq!(
            presets.get("Bob"),
            B41_OUTFIT.split_once(':').map(|(_, v)| v)
        );
    }

    #[test]
    fn reads_builds_and_keeps_lines_it_does_not_know() {
        let text = "Carpenter run:carpenter;Strong;Insomniac;\n\njunk without colon\n\
                    Nurse:nurse;FastLearner;\n";
        let presets = Presets::parse(Kind::Builds, text);
        assert_eq!(presets.to_string(), text);
        assert_eq!(presets.iter().count(), 2);
        assert_eq!(presets.get("Nurse"), Some("nurse;FastLearner;"));
    }

    #[test]
    fn outfits_count_only_under_version_one() {
        let presets = Presets::parse(Kind::Outfits, "Old:gender=1;\nVERSION=2\nNew:gender=2;\n");
        assert_eq!(presets.iter().count(), 0);
        let presets = Presets::parse(Kind::Outfits, "VERSION=1\nA:gender=1;\n");
        assert_eq!(presets.get("A"), Some("gender=1;"));
    }

    #[test]
    fn a_repeated_name_is_its_last_value() {
        let mut presets = Presets::parse(Kind::Builds, "A:one;\nA:two;\n");
        assert_eq!(presets.iter().collect::<Vec<_>>(), [("A", "two;")]);
        presets.set("A", "three;");
        assert_eq!(presets.to_string(), "A:one;\nA:three;\n");
    }

    #[test]
    fn merges_by_name_without_dropping_presets() {
        let mut here = Presets::parse(
            Kind::Outfits,
            "VERSION=1\nMine:gender=1;\nShared:gender=1;\n",
        );
        let store = Presets::parse(
            Kind::Outfits,
            "VERSION=1\nShared:gender=2;\nNew:gender=1;\n",
        );
        assert!(here.merge(&store));
        assert_eq!(
            here.to_string(),
            "VERSION=1\nMine:gender=1;\nShared:gender=2;\nNew:gender=1;\n"
        );
        assert!(!here.merge(&store), "nothing left to change");
    }

    #[test]
    fn a_new_outfit_file_gets_its_version_line() {
        let mut empty = Presets::parse(Kind::Outfits, "");
        empty.set("A", "gender=1;");
        assert_eq!(empty.to_string(), "VERSION=1\nA:gender=1;\n");
        let mut builds = Presets::parse(Kind::Builds, "");
        builds.set("A", "nurse;");
        assert_eq!(builds.to_string(), "A:nurse;\n");
    }

    #[test]
    fn a_new_outfit_lands_under_version_one() {
        let mut presets = Presets::parse(Kind::Outfits, "VERSION=1\nA:gender=1;\nVERSION=2\n");
        presets.set("B", "gender=2;");
        assert_eq!(presets.get("B"), Some("gender=2;"));
        let again = Presets::parse(Kind::Outfits, &presets.to_string());
        assert_eq!(again.get("B"), Some("gender=2;"), "the game reads it");
    }
}
