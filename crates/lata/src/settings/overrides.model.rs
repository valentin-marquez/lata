//! Launch options set once in Settings that win over what each instance has, without changing
//! what the instances store.

use serde::{Deserialize, Serialize};

use crate::instance::Instance;

/// Each one unset leaves every instance its own value.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Overrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_gb: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jvm_args: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_args: Option<String>,
}

impl Overrides {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The instance as it starts: each override set in place of the instance's own value.
    pub fn apply(&self, instance: &Instance) -> Instance {
        let mut instance = instance.clone();
        if let Some(gb) = self.memory_gb {
            instance.memory_gb = Some(gb);
        }
        if let Some(args) = &self.jvm_args {
            instance.jvm_args = args.clone();
        }
        if let Some(args) = &self.launch_args {
            instance.launch_args = args.clone();
        }
        instance
    }
}

/// The overrides in effect now.
pub fn current() -> Overrides {
    super::load().overrides
}

/// Arguments as typed in Settings: an empty field is no override at all.
pub fn args(typed: &str) -> Option<String> {
    let typed = typed.trim();
    (!typed.is_empty()).then(|| typed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{Overrides, args};
    use crate::instance::Instance;

    fn instance() -> Instance {
        Instance {
            id: "a".into(),
            dir: "a".into(),
            name: "A".into(),
            branch: "public".into(),
            build: None,
            last_played: None,
            icon: None,
            memory_gb: Some(8),
            jvm_args: "-XX:+AlwaysPreTouch".into(),
            launch_args: "-debug".into(),
        }
    }

    #[test]
    fn unset_overrides_keep_the_instance_own_values() {
        let effective = Overrides::default().apply(&instance());
        assert_eq!(effective.memory_gb, Some(8));
        assert_eq!(effective.jvm_args, "-XX:+AlwaysPreTouch");
        assert_eq!(effective.launch_args, "-debug");
    }

    #[test]
    fn set_overrides_win_one_by_one() {
        let memory = Overrides {
            memory_gb: Some(16),
            ..Default::default()
        };
        let effective = memory.apply(&instance());
        assert_eq!(effective.memory_gb, Some(16));
        assert_eq!(effective.launch_args, "-debug", "only what is set wins");

        let everything = Overrides {
            memory_gb: Some(12),
            jvm_args: Some("-XX:+UseZGC".into()),
            launch_args: Some("-nosteam".into()),
        };
        let mut plain = instance();
        plain.memory_gb = None;
        let effective = everything.apply(&plain);
        assert_eq!(
            effective.memory_gb,
            Some(12),
            "wins over the game default too"
        );
        assert_eq!(effective.jvm_args, "-XX:+UseZGC");
        assert_eq!(effective.launch_args, "-nosteam");
        assert_eq!(plain.memory_gb, None, "the instance itself is untouched");
    }

    #[test]
    fn an_empty_field_is_no_override() {
        assert_eq!(args("   "), None);
        assert_eq!(args(" -debug "), Some("-debug".into()));
    }

    #[test]
    fn saved_overrides_read_back() {
        let read = |text: &str| serde_json::from_str::<Overrides>(text).unwrap();
        assert!(read("{}").is_empty());
        let saved = read(r#"{"memory_gb":16,"launch_args":"-debug"}"#);
        assert_eq!(saved.memory_gb, Some(16));
        assert_eq!(saved.jvm_args, None);
        assert_eq!(
            serde_json::to_string(&saved).unwrap(),
            r#"{"memory_gb":16,"launch_args":"-debug"}"#
        );
    }
}
