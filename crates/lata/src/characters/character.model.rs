//! A character as the Characters page shows it: the look and the build the game saved under
//! one name. The game keeps them apart (its look screen and its profession screen each save
//! their own presets), so a name may have only one of the two.

use super::presets::Presets;

/// Red, green and blue from 0 to 1, as the game writes them.
pub type Rgb = [f32; 3];

#[derive(Clone, Debug, PartialEq)]
pub struct Character {
    pub name: String,
    pub look: Option<Look>,
    pub build: Option<Build>,
}

/// Every name in either file, looks first in their order, then builds that have no look.
pub fn list(outfits: &Presets, builds: &Presets) -> Vec<Character> {
    let mut names: Vec<&str> = outfits.iter().map(|(name, _)| name).collect();
    for (name, _) in builds.iter() {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
        .into_iter()
        .map(|name| Character {
            name: name.to_owned(),
            look: outfits.get(name).map(Look::parse),
            build: builds.get(name).map(Build::parse),
        })
        .collect()
}

impl Character {
    /// Writes this character into both files, in place of `old` (its name before a rename).
    pub fn save(&self, old: Option<&str>, outfits: &mut Presets, builds: &mut Presets) {
        if let Some(old) = old.filter(|old| *old != self.name) {
            outfits.remove(old);
            builds.remove(old);
        }
        match &self.look {
            Some(look) => outfits.set(&self.name, &look.to_string()),
            None => {
                outfits.remove(&self.name);
            }
        }
        match &self.build {
            Some(build) => builds.set(&self.name, &build.to_string()),
            None => {
                builds.remove(&self.name);
            }
        }
    }
}

/// What a preset name may hold: the game splits its lines at the first colon.
pub fn clean(name: &str) -> String {
    name.replace([':', '\n', '\r'], "").trim().to_owned()
}

/// `name` if no character has it yet, else `name 2`, `name 3`...
pub fn unique(name: &str, taken: &[Character]) -> String {
    let free = |candidate: &str| !taken.iter().any(|c| c.name == candidate);
    if free(name) {
        return name.to_owned();
    }
    (2..)
        .map(|n| format!("{name} {n}"))
        .find(|candidate| free(candidate))
        .unwrap_or_default()
}

/// A look: `key=value;` entries in the game's order. Keys lata doesn't edit (clothing, voice,
/// stubble) are kept as written.
#[derive(Clone, Debug, PartialEq)]
pub struct Look(Vec<String>);

/// Keys that aren't a piece of clothing.
const BODY: [&str; 10] = [
    "gender",
    "skincolor",
    "name",
    "hair",
    "stubble",
    "chesthair",
    "beard",
    "beardstubble",
    "voiceStyle",
    "voicePitch",
];

/// A piece of clothing: the body location it is worn on, the item and its tint.
#[derive(Clone, Debug, PartialEq)]
pub struct Piece {
    pub slot: String,
    pub item: String,
    pub tint: Option<Rgb>,
}

impl Look {
    pub fn parse(value: &str) -> Self {
        Self(
            value
                .split(';')
                .filter(|entry| !entry.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    }

    /// A first look: the game's first skin color, its default T-shirt, trousers and trainers
    /// (in every build's default selection), so it doesn't load naked.
    pub fn new(female: bool, skin: Rgb, hair: &str, color: Rgb) -> Self {
        let mut look = Self(Vec::new());
        look.set_female(female);
        look.set_skin(skin);
        look.set_hair(hair, color);
        for entry in [
            "Tshirt=Base.Tshirt_DefaultTEXTURE_TINT|1,1,1",
            "Pants=Base.Trousers_DefaultTEXTURE_TINT|0.2,0.25,0.35",
            "Socks=Base.Socks_Ankle",
            "Shoes=Base.Shoes_TrainerTINT|0.9,0.9,0.9",
        ] {
            look.0.push(entry.to_owned());
        }
        look
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.0.iter().find_map(|entry| match entry.split_once('=') {
            Some((k, value)) if k == key => Some(value),
            _ => None,
        })
    }

    /// Sets `key`, in its place if it is there, else after the body keys before it in the
    /// game's order (the game reads `gender` first: it clears the clothes).
    fn set(&mut self, key: &str, value: &str) {
        let entry = format!("{key}={value}");
        if let Some(found) = self
            .0
            .iter_mut()
            .find(|e| e.split_once('=').is_some_and(|(k, _)| k == key))
        {
            *found = entry;
            return;
        }
        let rank = |k: &str| BODY.iter().position(|b| *b == k);
        let mine = rank(key).unwrap_or(BODY.len());
        let at = self
            .0
            .iter()
            .position(|e| {
                let k = e.split_once('=').map_or(e.as_str(), |(k, _)| k);
                rank(k).is_none_or(|r| r > mine)
            })
            .unwrap_or(self.0.len());
        self.0.insert(at, entry);
    }

    fn remove(&mut self, key: &str) {
        self.0
            .retain(|e| e.split_once('=').is_none_or(|(k, _)| k != key));
    }

    /// The game writes 1 for female and 2 for male (its gender list's index).
    pub fn female(&self) -> bool {
        self.get("gender") == Some("1")
    }

    /// A woman's look loses the beard and chest hair the game only reads for men.
    pub fn set_female(&mut self, female: bool) {
        self.set("gender", if female { "1" } else { "2" });
        if female {
            for key in ["chesthair", "beard", "beardstubble"] {
                self.remove(key);
            }
        }
    }

    pub fn skin(&self) -> Option<Rgb> {
        rgb(self.get("skincolor")?)
    }

    pub fn set_skin(&mut self, color: Rgb) {
        self.set("skincolor", &text(color));
    }

    /// Forename and surname.
    pub fn names(&self) -> (String, String) {
        let value = self.get("name").unwrap_or_default();
        let (first, last) = value.split_once('|').unwrap_or((value, ""));
        (first.to_owned(), last.to_owned())
    }

    pub fn set_names(&mut self, first: &str, last: &str) {
        let tidy = |s: &str| {
            s.replace(['|', ';', '=', ':', '\n', '\r'], "")
                .trim()
                .to_owned()
        };
        self.set("name", &format!("{}|{}", tidy(first), tidy(last)));
    }

    /// The hair style and its color.
    pub fn hair(&self) -> (String, Option<Rgb>) {
        let value = self.get("hair").unwrap_or_default();
        let (style, color) = value.split_once('|').unwrap_or((value, ""));
        (style.to_owned(), rgb(color))
    }

    pub fn set_hair(&mut self, style: &str, color: Rgb) {
        self.set("hair", &format!("{style}|{}", text(color)));
    }

    /// The beard style; empty for none.
    pub fn beard(&self) -> &str {
        self.get("beard").unwrap_or_default()
    }

    pub fn set_beard(&mut self, style: &str) {
        self.set("beard", style);
    }

    /// The clothes, in the order they were saved.
    pub fn pieces(&self) -> Vec<Piece> {
        self.0
            .iter()
            .filter_map(|entry| {
                let (slot, value) = entry.split_once('=')?;
                if BODY.contains(&slot) {
                    return None;
                }
                let mut parts = value.split('|');
                let item = parts.next()?.to_owned();
                // A tint is three numbers; a lone number after the item is a texture choice.
                let tint = parts.find_map(rgb);
                Some(Piece {
                    slot: slot.to_owned(),
                    item,
                    tint,
                })
            })
            .collect()
    }
}

impl std::fmt::Display for Look {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.iter().try_for_each(|entry| write!(f, "{entry};"))
    }
}

fn rgb(text: &str) -> Option<Rgb> {
    let mut parts = text.split(',').map(|n| n.trim().parse::<f32>().ok());
    let color = [parts.next()??, parts.next()??, parts.next()??];
    parts.next().is_none().then_some(color)
}

/// Short, as people would type them: the game reads any float.
fn text(color: Rgb) -> String {
    let n = |v: f32| {
        let s = format!("{:.3}", v.clamp(0., 1.));
        s.trim_end_matches('0').trim_end_matches('.').to_owned()
    };
    format!("{},{},{}", n(color[0]), n(color[1]), n(color[2]))
}

/// A build: `profession;trait;trait;`.
#[derive(Clone, Debug, PartialEq)]
pub struct Build {
    pub profession: String,
    pub traits: Vec<String>,
}

impl Build {
    pub fn parse(value: &str) -> Self {
        let mut parts = value
            .split(';')
            .filter(|p| !p.is_empty())
            .map(str::to_owned);
        Self {
            profession: parts.next().unwrap_or_default(),
            traits: parts.collect(),
        }
    }
}

impl std::fmt::Display for Build {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{};", self.profession)?;
        self.traits.iter().try_for_each(|t| write!(f, "{t};"))
    }
}

#[cfg(test)]
mod tests {
    use super::super::presets::Kind;
    use super::*;

    /// Saved by Build 42.22 (`saved_outfits.txt` and `saved_builds.txt`), shortened.
    const OUTFIT: &str = "gender=2;skincolor=0.8,0.65,0.45;name=Comandante|Martinez;\
        hair=GreasedBack|0.10588235408067703,0.09019608050584793,0.08627451211214066;stubble=1;\
        chesthair=2;beard=;beardstubble=1;voiceStyle=Bob;voicePitch=-100;Belt=Base.Belt2;\
        Shirt=Base.Shirt_PoliceBlue;\
        Tshirt=Base.Tshirt_DefaultTEXTURE_TINT|0.33725491166114807,0.24705882370471954,0.1882352977991104;\
        Pants=Base.Trousers_Police;Socks=Base.Socks_Long|0.6078431606292725,0.7686274647712708,0.7568627595901489;";
    const BUILD: &str = "policeofficer;heartyappetite;pronetoillness;brave;";

    #[test]
    fn reads_a_saved_character() {
        let outfits = Presets::parse(Kind::Outfits, &format!("VERSION=1\nComandante:{OUTFIT}\n"));
        let builds = Presets::parse(Kind::Builds, &format!("Comandante:{BUILD}\nNurse:nurse;\n"));
        let all = list(&outfits, &builds);
        assert_eq!(all.len(), 2);
        let look = all[0].look.as_ref().unwrap();
        assert!(!look.female());
        assert_eq!(look.skin(), Some([0.8, 0.65, 0.45]));
        assert_eq!(look.names(), ("Comandante".into(), "Martinez".into()));
        assert_eq!(look.hair().0, "GreasedBack");
        assert_eq!(look.beard(), "");
        let pieces = look.pieces();
        let slots: Vec<_> = pieces.iter().map(|p| p.slot.as_str()).collect();
        assert_eq!(slots, ["Belt", "Shirt", "Tshirt", "Pants", "Socks"]);
        assert_eq!(pieces[1].item, "Base.Shirt_PoliceBlue");
        assert_eq!(pieces[1].tint, None);
        assert!(pieces[2].tint.is_some());
        assert_eq!(look.to_string(), OUTFIT, "written back as read");
        let build = all[0].build.as_ref().unwrap();
        assert_eq!(build.profession, "policeofficer");
        assert_eq!(build.traits, ["heartyappetite", "pronetoillness", "brave"]);
        assert_eq!(build.to_string(), BUILD);
        assert_eq!(all[1].name, "Nurse");
        assert!(all[1].look.is_none());
    }

    #[test]
    fn a_texture_choice_is_not_a_tint() {
        let look = Look::parse("gender=1;Pants=Base.Trousers_Denim|2;");
        assert_eq!(look.pieces()[0].tint, None);
    }

    #[test]
    fn edits_keep_the_order_the_game_reads() {
        let mut look = Look::parse(OUTFIT);
        look.set_female(true);
        assert!(look.female());
        assert_eq!(look.get("beard"), None, "no beard on a woman");
        look.set_hair("Bob", [0.5, 0.25, 0.125]);
        assert_eq!(look.hair(), ("Bob".into(), Some([0.5, 0.25, 0.125])));
        look.set_female(false);
        look.set_beard("Goatee");
        let text = look.to_string();
        let at = |key: &str| text.find(key).unwrap();
        assert!(at("beard=Goatee") < at("voiceStyle"), "{text}");
        assert!(at("gender=") == 0);

        let fresh = Look::new(true, [1., 0.91, 0.72], "Bob", [0.2, 0.1, 0.05]);
        assert!(
            fresh
                .to_string()
                .starts_with("gender=1;skincolor=1,0.91,0.72;hair=Bob|0.2,0.1,0.05;")
        );
        assert_eq!(fresh.pieces().len(), 4);
    }

    #[test]
    fn saves_renames_and_drops_both_halves() {
        let mut outfits = Presets::parse(Kind::Outfits, &format!("VERSION=1\nOld:{OUTFIT}\n"));
        let mut builds = Presets::parse(Kind::Builds, &format!("Old:{BUILD}\n"));
        let mut character = list(&outfits, &builds).remove(0);
        character.name = "New".into();
        character.build = None;
        character.save(Some("Old"), &mut outfits, &mut builds);
        assert_eq!(outfits.get("Old"), None);
        assert_eq!(outfits.get("New"), Some(OUTFIT));
        assert_eq!(builds.iter().count(), 0);
    }

    #[test]
    fn names_stay_readable_and_unique() {
        assert_eq!(clean(" Kate: the nurse\n"), "Kate the nurse");
        // The game splits a whole line at its colons, so none may hide in the value.
        let mut look = Look::parse("gender=1;");
        look.set_names("Dr: Kate", "Smith|Jones");
        assert_eq!(look.names(), ("Dr Kate".into(), "SmithJones".into()));
        let taken = vec![Character {
            name: "Kate".into(),
            look: None,
            build: None,
        }];
        assert_eq!(unique("Kate", &taken), "Kate 2");
        assert_eq!(unique("Bob", &taken), "Bob");
    }
}
