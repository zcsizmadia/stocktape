//! Turning skin files into a resolved [`Skin`].

use std::fmt;
use std::path::PathBuf;

use toml::{Table, Value};

use crate::library::{SkinLibrary, SkinSource};
use crate::{BadgeSpec, Color, SCHEMA_VERSION, Skin, SparklineSpec, TextSpec};

/// How deep `extends` chains may go.
const MAX_EXTENDS: usize = 16;
/// How many `$token` hops a color may take.
const MAX_TOKEN_HOPS: usize = 8;

/// A problem with a skin, naming the skin and its file when there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkinError {
    /// The skin being resolved or the one in its `extends` chain at fault.
    pub skin: String,
    /// The file, for skins loaded from disk.
    pub path: Option<PathBuf>,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for SkinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "skin `{}`", self.skin)?;
        if let Some(path) = &self.path {
            write!(f, " ({})", path.display())?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for SkinError {}

const TEXT_KEYS: &[&str] = &["family", "size", "weight", "color"];
const SECTIONS: &[(&str, &[&str])] = &[
    (
        "banner",
        &["height", "background", "padding", "spacing", "baseline"],
    ),
    ("text", TEXT_KEYS),
    ("symbol", TEXT_KEYS),
    ("price", TEXT_KEYS),
    (
        "change",
        &["family", "size", "weight", "color", "show", "arrow"],
    ),
    ("percent", &["family", "size", "weight", "color", "show"]),
    ("colors", &["up", "down", "flat"]),
    ("separator", &["color", "width"]),
    (
        "badge",
        &[
            "enabled",
            "up_bg",
            "up_fg",
            "down_bg",
            "down_fg",
            "flat_bg",
            "flat_fg",
            "radius",
            "padding_x",
            "padding_y",
        ],
    ),
    ("sparkline", &["enabled", "width", "height", "stroke"]),
];
const TOP_LEVEL: &[&str] = &["schema", "name", "description", "extends", "tokens"];

struct Layer {
    id: String,
    path: Option<PathBuf>,
    table: Table,
}

fn path_of(source: &SkinSource) -> Option<PathBuf> {
    match source {
        SkinSource::Builtin => None,
        SkinSource::File(p) => Some(p.clone()),
    }
}

/// Parses one file and checks its schema version and keys.
fn load_layer(
    library: &SkinLibrary,
    id: &str,
    wanted_by: Option<&Layer>,
) -> Result<Layer, SkinError> {
    let Some(entry) = library.entry(id) else {
        return Err(match wanted_by {
            Some(child) => SkinError {
                skin: child.id.clone(),
                path: child.path.clone(),
                message: format!("extends unknown skin `{id}`"),
            },
            None => SkinError {
                skin: id.to_owned(),
                path: None,
                message: format!(
                    "no such skin; available: {}",
                    library.ids().collect::<Vec<_>>().join(", ")
                ),
            },
        });
    };
    let path = path_of(&entry.source);
    let err = |message: String| SkinError {
        skin: id.to_owned(),
        path: path.clone(),
        message,
    };
    let table: Table = entry
        .text
        .parse()
        .map_err(|e: toml::de::Error| err(format!("invalid TOML: {}", e.to_string().trim())))?;

    match table.get("schema") {
        Some(Value::Integer(v)) if *v == SCHEMA_VERSION => {}
        Some(Value::Integer(v)) => {
            return Err(err(format!(
                "schema {v} is not supported; this version reads schema {SCHEMA_VERSION}"
            )));
        }
        Some(_) => return Err(err("`schema` must be an integer".to_owned())),
        None => return Err(err(format!("missing `schema = {SCHEMA_VERSION}`"))),
    }

    for (key, value) in &table {
        if let Some((_, keys)) = SECTIONS.iter().find(|(name, _)| name == key) {
            let Value::Table(section) = value else {
                return Err(err(format!("`{key}` must be a table")));
            };
            for k in section.keys() {
                if !keys.contains(&k.as_str()) {
                    return Err(err(format!("unknown key `{key}.{k}`")));
                }
            }
        } else if key == "tokens" {
            let Value::Table(tokens) = value else {
                return Err(err("`tokens` must be a table".to_owned()));
            };
            if let Some((name, _)) = tokens.iter().find(|(_, v)| !v.is_str()) {
                return Err(err(format!("token `{name}` must be a color string")));
            }
        } else if !TOP_LEVEL.contains(&key.as_str()) {
            return Err(err(format!("unknown key `{key}`")));
        }
    }
    for key in ["name", "description", "extends"] {
        if table.get(key).is_some_and(|v| !v.is_str()) {
            return Err(err(format!("`{key}` must be a string")));
        }
    }
    Ok(Layer {
        id: id.to_owned(),
        path,
        table,
    })
}

/// Merges `top` over `base`: tables merge key by key, anything else replaces.
fn merge(base: &mut Table, top: &Table) {
    for (key, value) in top {
        match (base.get_mut(key), value) {
            (Some(Value::Table(b)), Value::Table(t)) => merge(b, t),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

pub(crate) fn resolve(library: &SkinLibrary, id: &str) -> Result<Skin, SkinError> {
    // Collect the chain leaf → root.
    let mut chain = vec![load_layer(library, id, None)?];
    while let Some(parent) = chain
        .last()
        .and_then(|l| l.table.get("extends"))
        .and_then(Value::as_str)
    {
        let parent = parent.to_owned();
        let child = chain.last().expect("chain is non-empty");
        if chain.iter().any(|l| l.id == parent) {
            return Err(SkinError {
                skin: child.id.clone(),
                path: child.path.clone(),
                message: format!("`extends` forms a cycle through `{parent}`"),
            });
        }
        if chain.len() >= MAX_EXTENDS {
            return Err(SkinError {
                skin: id.to_owned(),
                path: chain[0].path.clone(),
                message: format!("`extends` chain is deeper than {MAX_EXTENDS}"),
            });
        }
        let layer = load_layer(library, &parent, Some(child))?;
        chain.push(layer);
    }

    let leaf = &chain[0];
    let mut merged = Table::new();
    for layer in chain.iter().rev() {
        merge(&mut merged, &layer.table);
    }
    let reader = Reader {
        table: &merged,
        skin: id,
        path: leaf.path.clone(),
    };
    reader.skin(leaf)
}

struct Reader<'a> {
    table: &'a Table,
    skin: &'a str,
    path: Option<PathBuf>,
}

impl Reader<'_> {
    fn err(&self, message: String) -> SkinError {
        SkinError {
            skin: self.skin.to_owned(),
            path: self.path.clone(),
            message,
        }
    }

    fn get(&self, section: &str, key: &str) -> Option<&Value> {
        self.table.get(section)?.as_table()?.get(key)
    }

    fn require(&self, section: &str, key: &str) -> Result<&Value, SkinError> {
        self.get(section, key)
            .ok_or_else(|| self.err(format!("missing `{section}.{key}`")))
    }

    fn number(&self, section: &str, key: &str, min: f64, max: f64) -> Result<f32, SkinError> {
        let value = self.require(section, key)?;
        #[allow(clippy::cast_precision_loss)]
        let n = match value {
            Value::Integer(i) => *i as f64,
            Value::Float(f) => *f,
            _ => return Err(self.err(format!("`{section}.{key}` must be a number"))),
        };
        if !(n.is_finite() && (min..=max).contains(&n)) {
            return Err(self.err(format!(
                "`{section}.{key}` must be between {min} and {max}, not {n}"
            )));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(n as f32)
    }

    fn boolean(&self, section: &str, key: &str) -> Result<bool, SkinError> {
        self.require(section, key)?
            .as_bool()
            .ok_or_else(|| self.err(format!("`{section}.{key}` must be true or false")))
    }

    fn string(&self, section: &str, key: &str) -> Result<&str, SkinError> {
        self.require(section, key)?
            .as_str()
            .ok_or_else(|| self.err(format!("`{section}.{key}` must be a string")))
    }

    fn color(&self, section: &str, key: &str) -> Result<Color, SkinError> {
        let mut text = self.string(section, key)?;
        for _ in 0..MAX_TOKEN_HOPS {
            let Some(name) = text.strip_prefix('$') else {
                return text
                    .parse()
                    .map_err(|e| self.err(format!("`{section}.{key}`: {e}")));
            };
            text = self
                .table
                .get("tokens")
                .and_then(Value::as_table)
                .and_then(|t| t.get(name))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    self.err(format!("`{section}.{key}` uses unknown token `${name}`"))
                })?;
        }
        Err(self.err(format!(
            "`{section}.{key}`: token references go deeper than {MAX_TOKEN_HOPS} (cycle?)"
        )))
    }

    /// A text element: its own keys, falling back to `[text]`.
    fn text(&self, section: &str) -> Result<TextSpec, SkinError> {
        let pick = |key: &str| {
            if self.get(section, key).is_some() {
                section
            } else {
                "text"
            }
        };
        let weight = self.number(pick("weight"), "weight", 100.0, 900.0)?;
        Ok(TextSpec {
            family: self.string(pick("family"), "family")?.to_owned(),
            size: self.number(pick("size"), "size", 1.0, 1000.0)?,
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            weight: weight.round() as u16,
            color: self.color(pick("color"), "color")?,
        })
    }

    fn skin(&self, leaf: &Layer) -> Result<Skin, SkinError> {
        let leaf_str = |key: &str| {
            leaf.table
                .get(key)
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let show = |section: &str| -> Result<bool, SkinError> {
            match self.get(section, "show") {
                None => Ok(true),
                Some(_) => self.boolean(section, "show"),
            }
        };
        let change = if show("change")? {
            Some(self.text("change")?)
        } else {
            None
        };
        let percent = if show("percent")? {
            Some(self.text("percent")?)
        } else {
            None
        };
        let arrow = match self.get("change", "arrow") {
            None => true,
            Some(_) => self.boolean("change", "arrow")?,
        };
        let badge = if self.get("badge", "enabled").is_some() && self.boolean("badge", "enabled")? {
            Some(BadgeSpec {
                up: (self.color("badge", "up_bg")?, self.color("badge", "up_fg")?),
                down: (
                    self.color("badge", "down_bg")?,
                    self.color("badge", "down_fg")?,
                ),
                flat: (
                    self.color("badge", "flat_bg")?,
                    self.color("badge", "flat_fg")?,
                ),
                radius: self.number("badge", "radius", 0.0, 10_000.0)?,
                padding_x: self.number("badge", "padding_x", 0.0, 1000.0)?,
                padding_y: self.number("badge", "padding_y", 0.0, 1000.0)?,
            })
        } else {
            None
        };
        let sparkline = if self.get("sparkline", "enabled").is_some()
            && self.boolean("sparkline", "enabled")?
        {
            Some(SparklineSpec {
                width: self.number("sparkline", "width", 1.0, 1000.0)?,
                height: self.number("sparkline", "height", 1.0, 1000.0)?,
                stroke: self.number("sparkline", "stroke", 0.1, 100.0)?,
            })
        } else {
            None
        };
        Ok(Skin {
            id: self.skin.to_owned(),
            name: leaf_str("name").unwrap_or_else(|| self.skin.to_owned()),
            description: leaf_str("description").unwrap_or_default(),
            height: self.number("banner", "height", 8.0, 1000.0)?,
            background: self.color("banner", "background")?,
            padding: self.number("banner", "padding", 0.0, 1000.0)?,
            spacing: self.number("banner", "spacing", 0.0, 1000.0)?,
            baseline: self.number("banner", "baseline", 0.0, 1.0)?,
            symbol: self.text("symbol")?,
            price: self.text("price")?,
            change,
            arrow,
            percent,
            up: self.color("colors", "up")?,
            down: self.color("colors", "down")?,
            flat: self.color("colors", "flat")?,
            separator: self.color("separator", "color")?,
            separator_width: self.number("separator", "width", 0.0, 100.0)?,
            badge,
            sparkline,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{Color, SkinLibrary, SkinSource};

    fn with(id: &str, text: &str) -> SkinLibrary {
        let mut library = SkinLibrary::builtin();
        library.insert(id, text.to_owned(), SkinSource::Builtin);
        library
    }

    fn error(id: &str, text: &str) -> String {
        with(id, text).resolve(id).unwrap_err().to_string()
    }

    #[test]
    fn classic_dark_values() {
        let skin = SkinLibrary::builtin().resolve("classic-dark").unwrap();
        assert_eq!(skin.height, 56.0);
        assert_eq!(skin.background, Color::rgb(0x0E, 0x1A, 0x2F));
        assert_eq!(skin.symbol.weight, 800);
        assert_eq!(skin.symbol.size, 24.0);
        assert_eq!(skin.price.weight, 400);
        assert_eq!(skin.price.color, Color::rgb(0xDC, 0xE3, 0xEE));
        assert_eq!(skin.change.as_ref().unwrap().size, 19.0);
        assert!(skin.arrow && skin.badge.is_none() && skin.sparkline.is_none());
        assert_eq!(skin.separator.a, 0x14);
    }

    #[test]
    fn child_overrides_and_tokens_follow() {
        let library = with(
            "mine",
            r##"
            schema = 1
            extends = "classic-dark"
            [tokens]
            green = "#00FF00"
            [banner]
            height = 40
            "##,
        );
        let skin = library.resolve("mine").unwrap();
        assert_eq!(skin.up, Color::rgb(0, 255, 0), "token redefined by child");
        assert_eq!(skin.height, 40.0);
        assert_eq!(skin.padding, 22.0, "inherited");
        assert_eq!(skin.name, "mine", "name is not inherited");
    }

    #[test]
    fn element_overrides_text_defaults() {
        let skin = SkinLibrary::builtin().resolve("terminal").unwrap();
        assert_eq!(skin.price.family, "JetBrains Mono");
        assert_eq!(skin.symbol.weight, 700);
        assert_eq!(skin.price.size, 17.0);
        assert!(!skin.arrow);
    }

    #[test]
    fn hidden_elements() {
        let skin = SkinLibrary::builtin().resolve("daylight").unwrap();
        assert!(skin.change.is_none());
        assert!(skin.percent.is_some());
        assert!(skin.badge.is_some());
        let glass = SkinLibrary::builtin().resolve("glass").unwrap();
        assert!(glass.sparkline.is_some());
        assert!(glass.background.a < 255);
    }

    #[test]
    fn errors_are_specific() {
        assert!(
            SkinLibrary::builtin()
                .resolve("nope")
                .unwrap_err()
                .to_string()
                .contains("no such skin")
        );
        assert!(error("x", "name = \"x\"").contains("missing `schema = 1`"));
        assert!(error("x", "schema = 2").contains("schema 2 is not supported"));
        assert!(
            error(
                "x",
                "schema = 1\nextends = \"classic-dark\"\n[banner]\nheigth = 3"
            )
            .contains("unknown key `banner.heigth`")
        );
        assert!(
            error("x", "schema = 1\nextends = \"classic-dark\"\ncolour = 1")
                .contains("unknown key `colour`")
        );
        assert!(
            error("x", "schema = 1\nextends = \"missing\"")
                .contains("extends unknown skin `missing`")
        );
        assert!(
            error(
                "x",
                "schema = 1\nextends = \"classic-dark\"\n[colors]\nup = \"$nope\""
            )
            .contains("unknown token `$nope`")
        );
        assert!(
            error(
                "x",
                "schema = 1\nextends = \"classic-dark\"\n[colors]\nup = \"green\""
            )
            .contains("invalid color")
        );
        assert!(
            error(
                "x",
                "schema = 1\nextends = \"classic-dark\"\n[banner]\nbaseline = 2"
            )
            .contains("between 0 and 1")
        );
        assert!(
            error(
                "x",
                "schema = 1\nextends = \"classic-dark\"\n[symbol]\nweight = \"bold\""
            )
            .contains("must be a number")
        );
        assert!(error("x", "schema = 1\n[banner").contains("invalid TOML"));
        assert!(error("x", "schema = 1\nname = \"x\"").contains("missing `"));
    }

    #[test]
    fn cycles_are_detected() {
        let mut library = SkinLibrary::builtin();
        library.insert(
            "a",
            "schema = 1\nextends = \"b\"".to_owned(),
            SkinSource::Builtin,
        );
        library.insert(
            "b",
            "schema = 1\nextends = \"a\"".to_owned(),
            SkinSource::Builtin,
        );
        assert!(library.resolve("a").unwrap_err().message.contains("cycle"));
        let looped = with(
            "t",
            "schema = 1\nextends = \"classic-dark\"\n[tokens]\na = \"$b\"\nb = \"$a\"\n[colors]\nup = \"$a\"",
        );
        assert!(
            looped
                .resolve("t")
                .unwrap_err()
                .message
                .contains("deeper than")
        );
    }
}
