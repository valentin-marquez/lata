//! What a character can be in one game build, read from its files so Build 41 and Build 42
//! each get their own lists: professions and traits (B41 defines them in
//! `lua/shared/NPCs/MainCreationMethods.lua`, B42 in
//! `scripts/generated/characters/character_*.txt`), hair and beard styles
//! (`hairStyles/*.xml`), the skin and hair colors the character screen offers, and the English
//! names of all of these and of clothing items (`lua/shared/Translate/EN/`).
//!
//! Only the base game: mods add professions, traits and clothes in their own files, which
//! aren't read (a modded id shows as it is written).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::character::Rgb;

/// One option in a list: the id the game saves, its English name, and its cost in points.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Choice {
    pub id: String,
    pub name: String,
    pub cost: i32,
    /// Traits that can't be taken with this one; for a profession, those its own traits rule
    /// out (a veteran is desensitized, so can't be brave or cowardly).
    pub exclusive: Vec<String>,
    /// The traits a profession comes with.
    pub granted: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Catalog {
    /// Unemployed first, then by name.
    pub professions: Vec<Choice>,
    /// Traits a player picks (not the ones a profession grants), by name.
    pub traits: Vec<Choice>,
    pub female_hair: Vec<Choice>,
    pub male_hair: Vec<Choice>,
    /// "None" (an empty id) first.
    pub beards: Vec<Choice>,
    pub skins: Vec<Rgb>,
    pub hair_colors: Vec<Rgb>,
    /// The build's `media/` folder, for the preview's models and textures.
    pub media: PathBuf,
    names: HashMap<String, String>,
}

impl Catalog {
    /// Reads the build installed at `game`; `None` when its files aren't there.
    pub fn load(game: &Path) -> Option<Self> {
        let media = media(game)?;
        let read = |path: &str| std::fs::read(media.join(path)).ok();
        let text = |path: &str| read(path).map(|b| String::from_utf8_lossy(&b).into_owned());
        let mut names = HashMap::new();
        for file in [
            "UI_EN.txt",
            "IG_UI_EN.txt",
            "ItemName_EN.txt",
            "UI.json",
            "IG_UI.json",
            "ItemName.json",
        ] {
            if let Some(found) = text(&format!("lua/shared/Translate/EN/{file}")) {
                names.extend(translations(&found));
            }
        }
        let name = |key: &str, fallback: &str| {
            names
                .get(key)
                .cloned()
                .unwrap_or_else(|| fallback.to_owned())
        };
        let creation = text("lua/shared/NPCs/MainCreationMethods.lua")?;
        let (mut professions, mut traits) = match (
            text("scripts/generated/characters/character_professions.txt"),
            text("scripts/generated/characters/character_traits.txt"),
        ) {
            (Some(p), Some(t)) => (
                script(&p, "character_profession_definition"),
                script(&t, "character_trait_definition"),
            ),
            _ => (lua_professions(&creation), lua_traits(&creation)),
        };
        let named = |list: &mut Vec<(Choice, String, bool)>| {
            list.iter_mut()
                .map(|(choice, key, granted)| {
                    let choice = Choice {
                        name: name(key, &choice.id),
                        ..std::mem::take(choice)
                    };
                    (choice, *granted)
                })
                .collect::<Vec<_>>()
        };
        let mut traits = named(&mut traits);
        for (a, b) in exclusions(&creation) {
            for (one, other) in [(&a, &b), (&b, &a)] {
                let found = traits
                    .iter_mut()
                    .find(|(t, _)| t.id.eq_ignore_ascii_case(one));
                if let Some((t, _)) = found {
                    t.exclusive.push(other.clone());
                }
            }
        }
        let mut professions: Vec<Choice> = named(&mut professions)
            .into_iter()
            .map(|(mut profession, _)| {
                for own in &profession.granted {
                    let found = traits.iter().find(|(t, _)| t.id.eq_ignore_ascii_case(own));
                    if let Some((t, _)) = found {
                        profession.exclusive.extend(t.exclusive.iter().cloned());
                    }
                }
                profession
            })
            .collect();
        professions.sort_by_key(|p| (p.id != "unemployed", p.name.clone()));
        // Only the ones a player picks; a profession's own come with it.
        let mut traits: Vec<Choice> = traits
            .into_iter()
            .filter(|(_, granted)| !granted)
            .map(|(t, _)| t)
            .collect();
        traits.sort_by(|a, b| a.name.cmp(&b.name));
        let styles = |list: Vec<String>, prefix: &str, none: &str| -> Vec<Choice> {
            list.into_iter()
                .map(|id| Choice {
                    name: if id.is_empty() {
                        name(none, t!("None"))
                    } else {
                        name(&format!("{prefix}{id}"), &id)
                    },
                    id,
                    ..Choice::default()
                })
                .collect()
        };
        let hair = text("hairStyles/hairStyles.xml").unwrap_or_default();
        let mut beards = vec![String::new()];
        beards.extend(xml_styles(
            &text("hairStyles/beardStyles.xml").unwrap_or_default(),
            "style",
        ));
        let screen = text("lua/client/OptionScreens/CharacterCreationMain.lua").unwrap_or_default();
        Some(Self {
            professions,
            traits,
            female_hair: styles(xml_styles(&hair, "female"), "IGUI_Hair_", "IGUI_Hair_Bald"),
            male_hair: styles(xml_styles(&hair, "male"), "IGUI_Hair_", "IGUI_Hair_Bald"),
            beards: styles(beards, "IGUI_Beard_", "IGUI_Beard_None"),
            skins: skin_colors(&screen),
            hair_colors: hair_colors(&creation),
            media,
            names,
        })
    }

    pub fn hair(&self, female: bool) -> &[Choice] {
        if female {
            &self.female_hair
        } else {
            &self.male_hair
        }
    }

    /// A profession by the id a build saved. B41 and B42 spell some ids differently in case
    /// (`Brave`, `brave`), so case doesn't count.
    pub fn profession(&self, id: &str) -> Option<&Choice> {
        self.professions
            .iter()
            .find(|p| p.id.eq_ignore_ascii_case(id))
    }

    pub fn trait_(&self, id: &str) -> Option<&Choice> {
        self.traits.iter().find(|t| t.id.eq_ignore_ascii_case(id))
    }

    /// A clothing item's English name (`Base.Shirt_PoliceBlue`), else its id without the
    /// module.
    pub fn item(&self, id: &str) -> String {
        self.names
            .get(id)
            .or_else(|| self.names.get(&format!("ItemName_{id}")))
            .cloned()
            .unwrap_or_else(|| id.rsplit('.').next().unwrap_or(id).replace('_', " "))
    }
}

/// The game's `media/` folder: in the root on Windows, under `projectzomboid/` on Linux,
/// inside the app bundle on macOS.
fn media(game: &Path) -> Option<PathBuf> {
    [
        "media",
        "projectzomboid/media",
        "Project Zomboid.app/Contents/Java/media",
    ]
    .into_iter()
    .map(|dir| game.join(dir))
    .find(|dir| dir.is_dir())
}

/// A translation file: B41's `KEY = "text",` lines or B42's JSON `"KEY": "text",`.
fn translations(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|line| {
            let line = line.trim().trim_end_matches(',');
            let at = line.find([':', '='])?;
            let key = line[..at].trim().trim_matches('"');
            let value = line[at + 1..].trim();
            let value = value.strip_prefix('"')?.strip_suffix('"')?;
            (!key.is_empty()).then(|| (key.to_owned(), value.replace("\\\"", "\"")))
        })
        .collect()
}

/// Definitions of `kind` in a B42 script: each id (without its `base:`), its name key, cost,
/// exclusive traits, and whether a profession grants it.
fn script(text: &str, kind: &str) -> Vec<(Choice, String, bool)> {
    let mut found = Vec::new();
    let mut current: Option<(Choice, String, bool)> = None;
    for line in text.lines().map(str::trim) {
        if let Some(id) = line.strip_prefix(kind) {
            let id = id.trim();
            let id = id.split_once(':').map_or(id, |(_, id)| id);
            current = Some((
                Choice {
                    id: id.to_owned(),
                    ..Choice::default()
                },
                String::new(),
                false,
            ));
            continue;
        }
        let Some((choice, key, granted)) = current.as_mut() else {
            continue;
        };
        if line == "}" {
            found.extend(current.take());
            continue;
        }
        let Some((field, value)) = line.trim_end_matches(',').split_once('=') else {
            continue;
        };
        let value = value.trim();
        match field.trim() {
            "Cost" => choice.cost = value.parse().unwrap_or(0),
            "UIName" => *key = value.to_owned(),
            "IsProfessionTrait" => *granted = value == "true",
            "MutuallyExclusiveTraits" => choice.exclusive = ids(value),
            "GrantedTraits" => choice.granted = ids(value),
            _ => {}
        }
    }
    found
}

/// `base:a;base:b` to `a`, `b`.
fn ids(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(|t| t.split_once(':').map_or(t, |(_, id)| id).to_owned())
        .collect()
}

/// The arguments of each uncommented `call(...)` in B41's Lua, split at commas (none of the
/// calls read here has a comma inside an argument).
fn calls<'a>(text: &'a str, call: &str) -> Vec<Vec<&'a str>> {
    calls_in(text, call)
        .into_iter()
        .map(|(_, args)| args)
        .collect()
}

/// [`calls`], each with what comes before the call on its line (`local veteran =`).
fn calls_in<'a>(text: &'a str, call: &str) -> Vec<(&'a str, Vec<&'a str>)> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .filter_map(|line| {
            let at = line.find(call)?;
            let args = line[at + call.len()..].strip_prefix('(')?;
            let args = &args[..args.rfind(')')?];
            Some((&line[..at], args.split(',').map(str::trim).collect()))
        })
        .collect()
}

fn quoted(arg: &str) -> String {
    arg.trim_matches('"').to_owned()
}

/// `getText("UI_prof_x")` to `UI_prof_x`.
fn text_key(arg: &str) -> String {
    let arg = arg.strip_prefix("getText(").unwrap_or(arg);
    quoted(arg.trim_end_matches(')'))
}

/// `local veteran = ProfessionFactory.addProfession("id", getText("name"), "icon", cost)`,
/// with the traits it grants (`veteran:addFreeTrait("Desensitized")`).
fn lua_professions(text: &str) -> Vec<(Choice, String, bool)> {
    let free = calls_in(text, ":addFreeTrait");
    calls_in(text, "ProfessionFactory.addProfession")
        .into_iter()
        .filter(|(_, args)| args.len() >= 4)
        .map(|(before, args)| {
            let var = before.trim().trim_end_matches('=').trim();
            let var = var.strip_prefix("local ").unwrap_or(var).trim();
            let granted = free
                .iter()
                .filter(|(who, args)| who.trim() == var && !args.is_empty())
                .map(|(_, args)| quoted(args[0]))
                .collect();
            let choice = Choice {
                id: quoted(args[0]),
                cost: args[3].parse().unwrap_or(0),
                granted,
                ..Choice::default()
            };
            (choice, text_key(args[1]), false)
        })
        .collect()
}

/// `TraitFactory.addTrait("Id", getText("name"), cost, getText("desc"), profession, ...)`.
fn lua_traits(text: &str) -> Vec<(Choice, String, bool)> {
    calls(text, "TraitFactory.addTrait")
        .into_iter()
        .filter(|args| args.len() >= 5)
        .map(|args| {
            let choice = Choice {
                id: quoted(args[0]),
                cost: args[2].parse().unwrap_or(0),
                ..Choice::default()
            };
            (choice, text_key(args[1]), args[4] == "true")
        })
        .collect()
}

/// `TraitFactory.setMutualExclusive("A", "B")`; B42 lists them in the traits themselves.
fn exclusions(text: &str) -> Vec<(String, String)> {
    calls(text, "TraitFactory.setMutualExclusive")
        .into_iter()
        .filter(|args| args.len() == 2)
        .map(|args| (quoted(args[0]), quoted(args[1])))
        .collect()
}

/// `SurvivorDesc.addHairColor(ColorInfo.new(212/255,171/255,69/255, 1))`.
fn hair_colors(text: &str) -> Vec<Rgb> {
    calls(text, "SurvivorDesc.addHairColor")
        .into_iter()
        .filter_map(|args| {
            let channel = |arg: &str| -> Option<f32> {
                let arg = arg.trim_start_matches("ColorInfo.new(");
                let (n, d) = arg.split_once('/').unwrap_or((arg, "1"));
                Some(n.trim().parse::<f32>().ok()? / d.trim().parse::<f32>().ok()?)
            };
            Some([
                channel(args.first()?)?,
                channel(args.get(1)?)?,
                channel(args.get(2)?)?,
            ])
        })
        .collect()
}

/// `self.skinColors = { {r=1,g=0.91,b=0.72}, ... }` in the character screen.
fn skin_colors(text: &str) -> Vec<Rgb> {
    let Some(start) = text.find("self.skinColors = {") else {
        return Vec::new();
    };
    let rest = &text[start..];
    // The list ends at the first `}` that doesn't close a color.
    let end = rest
        .find("} }")
        .or_else(|| rest.find("}}"))
        .unwrap_or(rest.len());
    rest[..end]
        .split("{r=")
        .skip(1)
        .filter_map(|color| {
            let mut parts = color.split(',').map(|part| {
                let value = part.split_once('=').map_or(part, |(_, v)| v);
                value.trim().trim_end_matches('}').parse::<f32>().ok()
            });
            Some([parts.next()??, parts.next()??, parts.next()??])
        })
        .collect()
}

/// The model and texture of the `<tag>` entry called `name` in a styles file
/// (`skinned/hair/m_hair_picard`, `F_Hair_White`); the model is empty for bald.
pub fn style(xml: &str, tag: &str, name: &str) -> Option<(String, String)> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    let field = |entry: &str, field: &str| {
        let value = entry.split(&format!("<{field}>")).nth(1)?;
        Some(
            value
                .split(&format!("</{field}>"))
                .next()?
                .trim()
                .to_owned(),
        )
    };
    xml.split(&open).skip(1).find_map(|entry| {
        let entry = entry.split(&close).next()?;
        field(entry, "name")
            .filter(|n| n.eq_ignore_ascii_case(name))
            .map(|_| {
                (
                    field(entry, "model").unwrap_or_default(),
                    field(entry, "texture").unwrap_or_default(),
                )
            })
    })
}

/// The names of `<tag>` entries in a styles file, leaving out the ones the game doesn't
/// offer on its character screen (`noChoose`).
fn xml_styles(xml: &str, tag: &str) -> Vec<String> {
    let (open, close) = (format!("<{tag}>"), format!("</{tag}>"));
    xml.split(&open)
        .skip(1)
        .filter_map(|entry| {
            let entry = entry.split(&close).next()?;
            if entry.contains("<noChoose>true</noChoose>") {
                return None;
            }
            let name = entry.split("<name>").nth(1)?.split("</name>").next()?;
            Some(name.trim().to_owned())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lines from Build 41.78's `MainCreationMethods.lua`.
    const B41_LUA: &str = r#"
	local unemployed = ProfessionFactory.addProfession("unemployed", getText("UI_prof_unemployed"), "", 8);
    local policeofficer = ProfessionFactory.addProfession("policeofficer", getText("UI_prof_policeoff"), "profession_policeofficer2", -4);
	local veteran = ProfessionFactory.addProfession("veteran", getText("UI_prof_Veteran"), "profession_veteran2", -8);
	veteran:addFreeTrait("Desensitized");
    TraitFactory.addTrait("Axeman", getText("UI_trait_axeman"), 0, getText("UI_trait_axemandesc"), true);
    local handy = TraitFactory.addTrait("Handy", getText("UI_trait_handy"), 8, getText("UI_trait_handydesc"), false);
  --  TraitFactory.addTrait("Brooding", getText("UI_trait_brooding"), -2, getText("UI_trait_broodingdesc"), false);
    TraitFactory.addTrait("SpeedDemon", getText("UI_trait_SpeedDemon"), 1, getText("UI_trait_SpeedDemonDesc"), false);
	TraitFactory.addTrait("SundayDriver", getText("UI_trait_SundayDriver"), -1, getText("UI_trait_SundayDriverDesc"), false);
	TraitFactory.addTrait("NeedsLessSleep", getText("UI_trait_LessSleep"), 2, getText("UI_trait_LessSleepDesc"), false, not sleepOK);
	TraitFactory.setMutualExclusive("SpeedDemon", "SundayDriver");
--    TraitFactory.setMutualExclusive("Metalworker", "Metalworker2");
	SurvivorDesc.addHairColor(ColorInfo.new(212/255,171/255,69/255, 1)) -- blonde
	SurvivorDesc.addHairColor(ColorInfo.new(169/255,133/255,81/255,1));
--	SurvivorDesc.addHairColor(ColorInfo.new(0.8313725590705872,0.6705882549285889,0.2705882489681244, 1)) -- blonde
"#;

    /// From Build 42.22's `character_professions.txt` and `character_traits.txt`.
    const B42_SCRIPT: &str = "module Base
{
    character_profession_definition base:burglar
    {
        CharacterProfession = base:burglar,
        Cost = -6,
        UIName = UI_prof_Burglar,
        IconPathName = profession_burglar2,
        GrantedTraits = base:burglar,
        XPBoosts = Nimble=2;Sneak=2;Lightfoot=2,
    }

    character_trait_definition base:adrenalinejunkie
    {
        IsProfessionTrait = false,
        DisabledInMultiplayer = false,
        CharacterTrait = base:adrenalinejunkie,
        Cost = 4,
        UIName = UI_trait_AdrenalineJunkie,
        UIDescription = UI_trait_AdrenalineJunkieDesc,
        MutuallyExclusiveTraits = base:agoraphobic;base:claustrophobic,
    }

    character_trait_definition base:burglar
    {
        IsProfessionTrait = true,
        CharacterTrait = base:burglar,
        Cost = 0,
        UIName = UI_prof_Burglar,
    }
}
";

    #[test]
    fn reads_build_41_professions_and_traits() {
        let professions = lua_professions(B41_LUA);
        let ids: Vec<_> = professions.iter().map(|(p, _, _)| p.id.as_str()).collect();
        assert_eq!(ids, ["unemployed", "policeofficer", "veteran"]);
        assert_eq!(professions[1].0.cost, -4);
        assert_eq!(professions[2].0.granted, ["Desensitized"]);
        assert_eq!(professions[1].1, "UI_prof_policeoff");
        let traits = lua_traits(B41_LUA);
        let ids: Vec<_> = traits.iter().map(|(t, _, _)| t.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "Axeman",
                "Handy",
                "SpeedDemon",
                "SundayDriver",
                "NeedsLessSleep"
            ]
        );
        assert!(traits[0].2, "a profession's own");
        assert_eq!(traits[1].0.cost, 8);
        assert_eq!(
            exclusions(B41_LUA),
            [("SpeedDemon".to_owned(), "SundayDriver".to_owned())]
        );
        let colors = hair_colors(B41_LUA);
        assert_eq!(colors.len(), 2);
        assert!((colors[0][0] - 212. / 255.).abs() < 1e-6);
    }

    #[test]
    fn reads_build_42_definitions() {
        let professions = script(B42_SCRIPT, "character_profession_definition");
        assert_eq!(professions.len(), 1);
        assert_eq!(professions[0].0.id, "burglar");
        assert_eq!(professions[0].0.cost, -6);
        assert_eq!(professions[0].1, "UI_prof_Burglar");
        assert_eq!(professions[0].0.granted, ["burglar"]);
        let traits = script(B42_SCRIPT, "character_trait_definition");
        assert_eq!(traits.len(), 2);
        assert_eq!(traits[0].0.id, "adrenalinejunkie");
        assert_eq!(traits[0].0.exclusive, ["agoraphobic", "claustrophobic"]);
        assert!(
            !traits[0].2 && traits[1].2,
            "the burglar's own trait is granted"
        );
    }

    #[test]
    fn reads_both_translation_formats() {
        let b41 = translations(
            "UI_EN = {\n    UI_prof_policeoff = \"Police Officer\",\n    ItemName_Base.Shirt_PoliceBlue = \"Police Trooper Shirt\",\n}",
        );
        assert_eq!(b41["UI_prof_policeoff"], "Police Officer");
        let b42 = translations(
            "{\n    \"UI_trait_axeman\": \"Ax-pert\",\n    \"Base.Shirt_PoliceBlue\": \"Police Shirt\"\n}",
        );
        assert_eq!(b42["UI_trait_axeman"], "Ax-pert");
        let catalog = Catalog {
            names: b41,
            ..Catalog::default()
        };
        assert_eq!(
            catalog.item("Base.Shirt_PoliceBlue"),
            "Police Trooper Shirt"
        );
        assert_eq!(catalog.item("Base.Belt_Leather"), "Belt Leather");
    }

    #[test]
    fn reads_styles_and_skin_colors() {
        let xml = "<hairStyles>\n\t<male>\n\t\t<name>Bald</name>\n\t\t<level>0</level>\n\t</male>\n\
            \t<male>\n\t\t<name>Hat</name>\n\t\t<noChoose>true</noChoose>\n\t</male>\n\
            \t<female>\n\t\t<name>Bob</name>\n\t</female>\n</hairStyles>";
        assert_eq!(xml_styles(xml, "male"), ["Bald"]);
        assert_eq!(xml_styles(xml, "female"), ["Bob"]);
        let picard = "<male>\n\t<name>Picard</name>\n\t<model>skinned/hair/m_hair_picard</model>\n\
            \t<texture>F_Hair_White</texture>\n</male>";
        assert_eq!(
            style(picard, "male", "picard"),
            Some(("skinned/hair/m_hair_picard".into(), "F_Hair_White".into()))
        );
        assert_eq!(
            style(xml, "male", "Bald"),
            Some((String::new(), String::new()))
        );
        assert_eq!(style(xml, "female", "Picard"), None);
        let lua = "\tself.skinColors = { {r=1,g=0.91,b=0.72},\n\t\t{r=0.98,g=0.79,b=0.49},\n\t\t{r=0.36,g=0.25,b=0.14} }\n\tlocal skinColorBtn = ISButton:new()";
        assert_eq!(
            skin_colors(lua),
            [[1., 0.91, 0.72], [0.98, 0.79, 0.49], [0.36, 0.25, 0.14]]
        );
    }

    /// Against the real builds, when they are installed here.
    #[test]
    #[ignore]
    fn reads_the_installed_builds() {
        for (build, traits) in [("24928713", "Handy"), ("26486631", "handy")] {
            let game = PathBuf::from("/mnt/datos/lata/builds").join(build);
            let catalog = Catalog::load(&game).expect("installed");
            assert!(catalog.professions.len() > 15, "{build}");
            assert_eq!(catalog.professions[0].id, "unemployed");
            assert!(catalog.trait_(traits).is_some_and(|t| t.name == "Handy"));
            assert!(catalog.male_hair.len() > 20 && catalog.female_hair.len() > 20);
            assert!(catalog.beards.len() > 5 && catalog.beards[0].id.is_empty());
            let veteran = catalog.profession("veteran").expect("veteran");
            assert!(
                veteran
                    .exclusive
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case("brave"))
            );
            assert!(
                catalog.trait_("desensitized").is_none(),
                "granted, not picked"
            );
            assert_eq!(catalog.skins.len(), 5);
            assert!(catalog.hair_colors.len() > 15);
        }
    }
}
