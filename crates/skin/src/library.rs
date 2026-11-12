//! The set of available skins: built-in ones plus files from a folder.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::Skin;
use crate::resolve::{SkinError, resolve};

const BUILTIN: [(&str, &str); 4] = [
    ("classic-dark", include_str!("../builtin/classic-dark.toml")),
    ("daylight", include_str!("../builtin/daylight.toml")),
    ("terminal", include_str!("../builtin/terminal.toml")),
    ("glass", include_str!("../builtin/glass.toml")),
];

/// Where a skin's text came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkinSource {
    /// Shipped with stocktape.
    Builtin,
    /// Loaded from a file.
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub(crate) text: String,
    pub(crate) source: SkinSource,
}

/// Available skins, by id (the file name without `.toml`).
///
/// ```
/// use stocktape_skin::SkinLibrary;
///
/// let library = SkinLibrary::builtin();
/// let skin = library.resolve("daylight").unwrap();
/// assert_eq!(skin.name, "Daylight");
/// assert!(skin.badge.is_some());
/// ```
#[derive(Debug, Clone)]
pub struct SkinLibrary {
    entries: BTreeMap<String, Entry>,
}

impl SkinLibrary {
    /// The id of the skin used when none is chosen.
    pub const DEFAULT: &'static str = "classic-dark";

    /// The built-in skins only.
    #[must_use]
    pub fn builtin() -> Self {
        let entries = BUILTIN
            .iter()
            .map(|(id, text)| {
                (
                    (*id).to_owned(),
                    Entry {
                        text: (*text).to_owned(),
                        source: SkinSource::Builtin,
                    },
                )
            })
            .collect();
        Self { entries }
    }

    /// Adds or replaces a skin from TOML text. A skin with the same id as a
    /// built-in one replaces it.
    pub fn insert(&mut self, id: &str, text: String, source: SkinSource) {
        self.entries.insert(id.to_owned(), Entry { text, source });
    }

    /// Loads every `*.toml` file in `dir` (not recursive). Returns the ids
    /// loaded. Files are only read here; errors in their content show up when
    /// they are resolved.
    ///
    /// # Errors
    ///
    /// Returns an error if the folder or a file can't be read.
    pub fn load_dir(&mut self, dir: &Path) -> std::io::Result<Vec<String>> {
        let mut loaded = Vec::new();
        let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
            })
            .collect();
        paths.sort();
        for path in paths {
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned) else {
                continue;
            };
            let text = std::fs::read_to_string(&path)?;
            self.insert(&id, text, SkinSource::File(path));
            loaded.push(id);
        }
        Ok(loaded)
    }

    /// Skin ids, sorted.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }

    /// Where a skin came from.
    #[must_use]
    pub fn source(&self, id: &str) -> Option<&SkinSource> {
        self.entries.get(id).map(|e| &e.source)
    }

    pub(crate) fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Resolves a skin: follows `extends`, merges, substitutes tokens and
    /// checks every value.
    ///
    /// # Errors
    ///
    /// Returns an error naming the skin, and the file when there is one, for
    /// unknown skins, inheritance cycles, unsupported schema versions, TOML
    /// syntax errors, unknown keys, missing or invalid values and unknown
    /// tokens.
    pub fn resolve(&self, id: &str) -> Result<Skin, SkinError> {
        resolve(self, id)
    }
}

impl Default for SkinLibrary {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_resolve() {
        let library = SkinLibrary::builtin();
        let ids: Vec<&str> = library.ids().collect();
        assert_eq!(ids, ["classic-dark", "daylight", "glass", "terminal"]);
        for id in ids {
            let skin = library.resolve(id).unwrap_or_else(|e| panic!("{id}: {e}"));
            assert_eq!(skin.id, id);
            assert!(!skin.name.is_empty() && !skin.description.is_empty());
            assert_eq!(library.source(id), Some(&SkinSource::Builtin));
        }
    }

    #[test]
    fn loads_a_folder_and_overrides_builtins() {
        let dir = std::env::temp_dir().join(format!("stocktape-skins-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("mine.toml"),
            "schema = 1\nname = \"Mine\"\nextends = \"classic-dark\"\n[tokens]\nnavy = \"#000000\"\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("daylight.toml"),
            "schema = 1\nextends = \"terminal\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();

        let mut library = SkinLibrary::builtin();
        let loaded = library.load_dir(&dir).unwrap();
        assert_eq!(loaded, ["daylight", "mine"]);
        assert_eq!(
            library.resolve("mine").unwrap().background,
            "#000000".parse().unwrap()
        );
        assert_eq!(
            library.resolve("daylight").unwrap().symbol.family,
            "JetBrains Mono"
        );
        assert!(matches!(library.source("mine"), Some(SkinSource::File(_))));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
