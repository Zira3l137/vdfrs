//! The app's own settings, kept between runs in `vdfrs/settings.toml` in the user's config folder.

use std::io;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub theme: ThemeChoice,
    pub material: Material,
}

impl gpui_kit::Global for Prefs {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];
}

/// What the window shows behind its chrome. Mica needs Windows 11 22H2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Material {
    #[default]
    Solid,
    Mica,
    MicaAlt,
}

impl Material {
    pub const ALL: [Self; 3] = [Self::Solid, Self::Mica, Self::MicaAlt];
}

impl Prefs {
    /// The saved settings, or the defaults if there are none or they cannot be read.
    pub fn load() -> Self {
        let Some(path) = path() else { return Self::default() };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|error| {
                eprintln!("vdfrs: ignoring {}: {error}", path.display());
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = path().ok_or_else(|| io::Error::other("there is no config folder"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, self.to_text())
    }

    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(text)
    }

    fn to_text(self) -> String {
        toml::to_string(&self).unwrap_or_default()
    }
}

fn path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("vdfrs").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let prefs = Prefs { theme: ThemeChoice::Dark, material: Material::MicaAlt };
        assert_eq!(prefs.to_text(), "theme = \"dark\"\nmaterial = \"mica-alt\"\n");
        assert_eq!(Prefs::parse(&prefs.to_text()).unwrap(), prefs);
    }

    #[test]
    fn missing_and_unknown_keys_are_fine() {
        assert_eq!(Prefs::parse("").unwrap(), Prefs::default());
        let prefs = Prefs::parse("material = \"mica\"\nfont = \"big\"").unwrap();
        assert_eq!(prefs, Prefs { material: Material::Mica, ..Prefs::default() });
        assert!(Prefs::parse("material = \"acrylic\"").is_err());
    }
}
