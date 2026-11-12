//! The banner configuration file, `stocktape.toml`.
//!
//! ```toml
//! [[banner]]
//! name = "Watchlist"
//! place = "bottom"        # top | bottom | left | right | around
//! displays = "primary"    # primary | all | landscape | portrait | index | name, or a list
//! exclude = "portrait"    # same forms; displays to leave out
//! skin = "classic-dark"
//! speed = 120             # logical pixels per second
//! offset = 0              # distance from the screen edge
//! inset = [0, 0]          # space left free at the start and end of the edge
//! reverse = false         # flow the other way
//! dock = true             # reserve screen space (edges only, where supported)
//! strategy = "demo"       # demo | watchlist | movers
//! ```
//!
//! Several `[[banner]]` tables make several banners. Built-in presets cover
//! the common layouts (`--layout top-bottom`, `span`, `around`, ...).

use stocktape_layout::{DisplaySelector, Edge, Placement, PlacementOptions};
use toml::{Table, Value};

/// Which strategy a banner uses (until strategies are configurable in full).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyKind {
    /// Sector groups with movers mixed in, capped and highlighted.
    Demo,
    /// The watchlist only, in sector groups.
    Watchlist,
    /// Big movers only.
    Movers,
}

/// One banner.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerConfig {
    /// Display name, for messages.
    pub name: String,
    /// Where it goes.
    pub placement: Placement,
    /// Offset, insets and direction; the thickness comes from the skin.
    pub options: PlacementOptions,
    /// Skin id.
    pub skin: String,
    /// Scroll speed in logical pixels per second.
    pub speed: f64,
    /// What it shows.
    pub strategy: StrategyKind,
    /// Reserve screen space so other windows stay clear (edge placements;
    /// where the platform supports it).
    pub dock: bool,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Banners, in the order they were written.
    pub banners: Vec<BannerConfig>,
}

const PRESETS: &[(&str, &str, &str)] = &[
    (
        "bottom",
        "One banner along the bottom of the primary display",
        r#"[[banner]]
name = "Watchlist"
place = "bottom"
"#,
    ),
    (
        "top-bottom",
        "Movers on top, the watchlist along the bottom",
        r#"[[banner]]
name = "Movers"
place = "top"
skin = "terminal"
strategy = "movers"
speed = 90

[[banner]]
name = "Watchlist"
place = "bottom"
"#,
    ),
    (
        "span",
        "One belt along the bottom of every landscape display, flowing from screen to screen",
        r#"[[banner]]
name = "Watchlist"
place = "bottom"
displays = "landscape"
"#,
    ),
    (
        "around",
        "One belt running around the edges of the primary display",
        r#"[[banner]]
name = "Watchlist"
place = "around"
speed = 160
"#,
    ),
    (
        "around-desktop",
        "One belt running around the outer edge of all landscape displays",
        r#"[[banner]]
name = "Watchlist"
place = "around"
displays = "landscape"
speed = 200
"#,
    ),
];

/// Built-in presets: `(name, description)`.
pub fn presets() -> impl Iterator<Item = (&'static str, &'static str)> {
    PRESETS
        .iter()
        .map(|(name, description, _)| (*name, *description))
}

impl Config {
    /// A built-in preset by name.
    ///
    /// # Errors
    ///
    /// Returns an error naming the available presets if there is none by that
    /// name.
    pub fn preset(name: &str) -> Result<Self, String> {
        let (_, _, text) = PRESETS.iter().find(|(n, _, _)| *n == name).ok_or_else(|| {
            let names: Vec<&str> = PRESETS.iter().map(|p| p.0).collect();
            format!("no layout preset `{name}`; available: {}", names.join(", "))
        })?;
        Self::parse(text)
    }

    /// Parses a configuration file's text.
    ///
    /// # Errors
    ///
    /// Returns a message naming the banner and key for syntax errors, unknown
    /// keys and invalid values.
    pub fn parse(text: &str) -> Result<Self, String> {
        let table: Table = text
            .parse()
            .map_err(|e: toml::de::Error| format!("invalid TOML: {}", e.to_string().trim()))?;
        if let Some(key) = table.keys().find(|k| *k != "banner") {
            return Err(format!("unknown key `{key}`"));
        }
        let Some(Value::Array(list)) = table.get("banner") else {
            return Err("no banners: add a [[banner]] table".to_owned());
        };
        let banners = list
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let t = v
                    .as_table()
                    .ok_or_else(|| format!("banner {} must be a table", i + 1))?;
                banner(t).map_err(|e| format!("banner {}: {e}", i + 1))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if banners.is_empty() {
            return Err("no banners: add a [[banner]] table".to_owned());
        }
        Ok(Self { banners })
    }
}

const KEYS: &[&str] = &[
    "name", "place", "displays", "exclude", "skin", "speed", "offset", "inset", "reverse",
    "strategy",
];

fn number(t: &Table, key: &str, default: f64, min: f64, max: f64) -> Result<f64, String> {
    #[allow(clippy::cast_precision_loss)]
    let v = match t.get(key) {
        None => return Ok(default),
        Some(Value::Integer(i)) => *i as f64,
        Some(Value::Float(f)) => *f,
        Some(_) => return Err(format!("`{key}` must be a number")),
    };
    if v.is_finite() && (min..=max).contains(&v) {
        Ok(v)
    } else {
        Err(format!("`{key}` must be between {min} and {max}"))
    }
}

fn string<'a>(t: &'a Table, key: &str, default: &'a str) -> Result<&'a str, String> {
    match t.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_str()
            .ok_or_else(|| format!("`{key}` must be a string")),
    }
}

/// One display reference: an index, a keyword, or (part of) a display name.
fn display_ref(key: &str, value: &Value) -> Result<DisplaySelector, String> {
    match value {
        Value::Integer(i) => usize::try_from(*i)
            .map(DisplaySelector::Index)
            .map_err(|_| format!("`{key}` index must not be negative")),
        Value::String(s) => Ok(match s.as_str() {
            "primary" => DisplaySelector::Primary,
            "all" => DisplaySelector::All,
            "landscape" => DisplaySelector::Landscape,
            "portrait" => DisplaySelector::Portrait,
            other => {
                let name = other.strip_prefix("name:").unwrap_or(other).trim();
                if name.is_empty() {
                    return Err(format!("`{key}` has an empty display name"));
                }
                DisplaySelector::Name(name.to_owned())
            }
        }),
        _ => Err(format!(
            "`{key}` entries must be display names, keywords or indexes"
        )),
    }
}

/// A `displays`/`exclude` value: one reference or a list of them.
fn display_set(key: &str, value: &Value) -> Result<DisplaySelector, String> {
    match value {
        Value::Array(list) if list.is_empty() => Err(format!("`{key}` list is empty")),
        Value::Array(list) => list
            .iter()
            .map(|v| display_ref(key, v))
            .collect::<Result<Vec<_>, _>>()
            .map(DisplaySelector::AnyOf),
        other => display_ref(key, other),
    }
}

fn displays(t: &Table) -> Result<DisplaySelector, String> {
    let include = match t.get("displays") {
        None => DisplaySelector::Primary,
        Some(v) => display_set("displays", v)?,
    };
    Ok(match t.get("exclude") {
        None => include,
        Some(v) => DisplaySelector::Except(Box::new(include), Box::new(display_set("exclude", v)?)),
    })
}

fn banner(t: &Table) -> Result<BannerConfig, String> {
    if let Some(key) = t.keys().find(|k| !KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    let displays = displays(t)?;
    let placement = match string(t, "place", "bottom")? {
        "top" => Placement::Edge {
            displays,
            edge: Edge::Top,
        },
        "bottom" => Placement::Edge {
            displays,
            edge: Edge::Bottom,
        },
        "left" => Placement::Edge {
            displays,
            edge: Edge::Left,
        },
        "right" => Placement::Edge {
            displays,
            edge: Edge::Right,
        },
        "around" => Placement::Around { displays },
        other => {
            return Err(format!(
                "`place` must be top, bottom, left, right or around, not {other:?}"
            ));
        }
    };
    let inset = match t.get("inset") {
        None => (0.0, 0.0),
        Some(Value::Array(a)) if a.len() == 2 => {
            let side = |v: &Value| match v {
                Value::Integer(i) if *i >= 0 =>
                {
                    #[allow(clippy::cast_precision_loss)]
                    Ok(*i as f64)
                }
                Value::Float(f) if f.is_finite() && *f >= 0.0 => Ok(*f),
                _ => Err("`inset` values must be non-negative numbers".to_owned()),
            };
            (side(&a[0])?, side(&a[1])?)
        }
        Some(_) => return Err("`inset` must be [start, end]".to_owned()),
    };
    let dock = match t.get("dock") {
        None => true,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`dock` must be true or false".to_owned())?,
    };
    let reverse = match t.get("reverse") {
        None => false,
        Some(v) => v
            .as_bool()
            .ok_or_else(|| "`reverse` must be true or false".to_owned())?,
    };
    let strategy = match string(t, "strategy", "demo")? {
        "demo" => StrategyKind::Demo,
        "watchlist" => StrategyKind::Watchlist,
        "movers" => StrategyKind::Movers,
        other => {
            return Err(format!(
                "`strategy` must be demo, watchlist or movers, not {other:?}"
            ));
        }
    };
    Ok(BannerConfig {
        name: string(t, "name", "Banner")?.to_owned(),
        placement,
        options: PlacementOptions {
            offset: number(t, "offset", 0.0, 0.0, 10_000.0)?,
            inset,
            reverse,
            ..PlacementOptions::default()
        },
        skin: string(t, "skin", stocktape_skin::SkinLibrary::DEFAULT)?.to_owned(),
        speed: number(t, "speed", 120.0, 0.0, 5_000.0)?,
        strategy,
        dock,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_preset_parses() {
        for (name, _) in presets() {
            let config = Config::preset(name).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert!(!config.banners.is_empty());
        }
        assert!(
            Config::preset("nope")
                .unwrap_err()
                .contains("available: bottom")
        );
    }

    #[test]
    fn defaults() {
        let c = Config::parse("[[banner]]").unwrap();
        let b = &c.banners[0];
        assert_eq!(
            b.placement,
            Placement::Edge {
                displays: DisplaySelector::Primary,
                edge: Edge::Bottom
            }
        );
        assert_eq!(b.skin, "classic-dark");
        assert_eq!(b.speed, 120.0);
        assert_eq!(b.strategy, StrategyKind::Demo);
    }

    #[test]
    fn full_banner() {
        let c = Config::parse(
            r#"
            [[banner]]
            name = "Side"
            place = "right"
            displays = "name:DELL"
            skin = "glass"
            speed = 80.5
            offset = 10
            inset = [20, 30.5]
            reverse = true
            strategy = "movers"

            [[banner]]
            place = "around"
            displays = 1
            "#,
        )
        .unwrap();
        let b = &c.banners[0];
        assert_eq!(
            b.placement,
            Placement::Edge {
                displays: DisplaySelector::Name("DELL".into()),
                edge: Edge::Right
            }
        );
        assert_eq!(b.options.inset, (20.0, 30.5));
        assert!(b.options.reverse);
        assert_eq!(b.options.offset, 10.0);
        assert_eq!(
            c.banners[1].placement,
            Placement::Around {
                displays: DisplaySelector::Index(1)
            }
        );
    }

    #[test]
    fn display_lists_and_exclusions() {
        let c = Config::parse(
            r#"
            [[banner]]
            displays = ["DISPLAY2", 0, "name:DELL"]
            [[banner]]
            displays = "all"
            exclude = "portrait"
            [[banner]]
            displays = "landscape"
            "#,
        )
        .unwrap();
        let selector = |i: usize| match &c.banners[i].placement {
            Placement::Edge { displays, .. } | Placement::Around { displays } => displays.clone(),
        };
        assert_eq!(
            selector(0),
            DisplaySelector::AnyOf(vec![
                DisplaySelector::Name("DISPLAY2".into()),
                DisplaySelector::Index(0),
                DisplaySelector::Name("DELL".into()),
            ])
        );
        assert_eq!(
            selector(1),
            DisplaySelector::Except(
                Box::new(DisplaySelector::All),
                Box::new(DisplaySelector::Portrait)
            )
        );
        assert_eq!(selector(2), DisplaySelector::Landscape);
    }

    #[test]
    fn errors_name_the_problem() {
        let err = |text: &str| Config::parse(text).unwrap_err();
        assert!(err("").contains("no banners"));
        assert!(err("[[banner]]\nplcae = \"top\"").contains("banner 1: unknown key `plcae`"));
        assert!(err("[[banner]]\nplace = \"middle\"").contains("`place` must be"));
        assert!(err("[[banner]]\ndisplays = \"\"").contains("empty display name"));
        assert!(err("[[banner]]\ndisplays = []").contains("list is empty"));
        assert!(err("[[banner]]\nexclude = [true]").contains("`exclude` entries"));
        assert!(err("[[banner]]\ndisplays = -1").contains("negative"));
        assert!(err("[[banner]]\nspeed = -5").contains("between"));
        assert!(err("[[banner]]\ninset = [1]").contains("[start, end]"));
        assert!(err("[[banner]]\nstrategy = \"random\"").contains("`strategy` must be"));
        assert!(err("[[banner]]\n[[banner]]\nreverse = 1").contains("banner 2"));
        assert!(err("title = 1").contains("unknown key `title`"));
        assert!(err("[[banner").contains("invalid TOML"));
    }
}
