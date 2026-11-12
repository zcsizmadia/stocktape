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

use std::time::Duration;

use stocktape_core::Symbol;
use stocktape_layout::{DisplaySelector, Edge, Placement, PlacementOptions};
use stocktape_strategies::Group;
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

/// One line of a banner.
#[derive(Debug, Clone, PartialEq)]
pub struct LineConfig {
    /// Skin id; its height is the line's thickness.
    pub skin: String,
    /// Scroll speed in logical pixels per second.
    pub speed: f64,
    /// What it shows.
    pub strategy: StrategyKind,
}

/// One banner.
#[derive(Debug, Clone, PartialEq)]
pub struct BannerConfig {
    /// Display name, for messages.
    pub name: String,
    /// Where it goes.
    pub placement: Placement,
    /// Offset, insets and direction; the thickness comes from the lines'
    /// skins.
    pub options: PlacementOptions,
    /// Lines from top to bottom (never empty). A banner without
    /// `[[banner.line]]` tables has one line.
    pub lines: Vec<LineConfig>,
    /// Reserve screen space so other windows stay clear (edge placements;
    /// where the platform supports it).
    pub dock: bool,
}

/// The whole configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Banners, in the order they were written.
    pub banners: Vec<BannerConfig>,
    /// Data feeds, primary first; empty means sample data.
    pub feeds: Vec<FeedConfig>,
    /// Watchlist groups; `None` means the demo watchlist.
    pub watchlist: Option<Vec<Group>>,
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
        "two-tier",
        "Two lines along the bottom: fast movers above the slower watchlist",
        r#"[[banner]]
name = "Two-tier"
place = "bottom"

[[banner.line]]
strategy = "movers"
speed = 180

[[banner.line]]
skin = "daylight"
strategy = "watchlist"
speed = 90
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
        if let Some(key) = table
            .keys()
            .find(|k| !["banner", "feed", "watchlist"].contains(&k.as_str()))
        {
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
        Ok(Self {
            banners,
            feeds: feeds(&table)?,
            watchlist: watchlist(&table)?,
        })
    }
}

const KEYS: &[&str] = &[
    "name", "place", "displays", "exclude", "skin", "speed", "offset", "inset", "reverse",
    "strategy", "line",
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
    // Banner-level skin, speed and strategy are the defaults for its lines.
    let default = LineConfig {
        skin: string(t, "skin", stocktape_skin::SkinLibrary::DEFAULT)?.to_owned(),
        speed: number(t, "speed", 120.0, 0.0, 5_000.0)?,
        strategy: strategy(t, StrategyKind::Demo)?,
    };
    let lines = match t.get("line") {
        None => vec![default],
        Some(Value::Array(list)) if !list.is_empty() => list
            .iter()
            .enumerate()
            .map(|(i, v)| {
                let line = v
                    .as_table()
                    .ok_or_else(|| format!("line {} must be a table", i + 1))?;
                parse_line(line, &default).map_err(|e| format!("line {}: {e}", i + 1))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err("`line` must be one or more [[banner.line]] tables".to_owned()),
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
        lines,
        dock,
    })
}

fn strategy(t: &Table, default: StrategyKind) -> Result<StrategyKind, String> {
    let Some(value) = t.get("strategy") else {
        return Ok(default);
    };
    match value.as_str() {
        Some("demo") => Ok(StrategyKind::Demo),
        Some("watchlist") => Ok(StrategyKind::Watchlist),
        Some("movers") => Ok(StrategyKind::Movers),
        _ => Err(format!(
            "`strategy` must be demo, watchlist or movers, not {value}"
        )),
    }
}

const LINE_KEYS: &[&str] = &["skin", "speed", "strategy"];

fn parse_line(t: &Table, default: &LineConfig) -> Result<LineConfig, String> {
    if let Some(key) = t.keys().find(|k| !LINE_KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    Ok(LineConfig {
        skin: string(t, "skin", &default.skin)?.to_owned(),
        speed: number(t, "speed", default.speed, 0.0, 5_000.0)?,
        strategy: strategy(t, default.strategy)?,
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
        assert_eq!(
            b.lines,
            [LineConfig {
                skin: "classic-dark".into(),
                speed: 120.0,
                strategy: StrategyKind::Demo
            }]
        );
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
    fn lines_inherit_banner_defaults() {
        let c = Config::parse(
            r#"
            [[banner]]
            skin = "terminal"
            speed = 80

            [[banner.line]]
            strategy = "movers"
            speed = 240

            [[banner.line]]
            skin = "daylight"
            "#,
        )
        .unwrap();
        let lines = &c.banners[0].lines;
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[0],
            LineConfig {
                skin: "terminal".into(),
                speed: 240.0,
                strategy: StrategyKind::Movers
            }
        );
        assert_eq!(
            lines[1],
            LineConfig {
                skin: "daylight".into(),
                speed: 80.0,
                strategy: StrategyKind::Demo
            }
        );
        let err = Config::parse("[[banner]]\n[[banner.line]]\nsped = 3").unwrap_err();
        assert!(
            err.contains("banner 1: line 1: unknown key `sped`"),
            "{err}"
        );
        let err = Config::parse("[[banner]]\nline = 3").unwrap_err();
        assert!(err.contains("[[banner.line]]"), "{err}");
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

/// A data feed.
#[derive(Clone, PartialEq)]
pub struct FeedConfig {
    /// Which provider.
    pub provider: Provider,
    /// The API key, read from the file or the environment. Never printed.
    pub api_key: String,
    /// Provider names for symbols whose default mapping is wrong.
    pub names: Vec<(Symbol, String)>,
    /// Time between snapshot rounds.
    pub refresh: Duration,
    /// Stream live trades; without it, only snapshots every `refresh`.
    pub stream: bool,
}

impl std::fmt::Debug for FeedConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let key = if self.api_key.is_empty() { "" } else { "****" };
        f.debug_struct("FeedConfig")
            .field("provider", &self.provider)
            .field("api_key", &key)
            .field("names", &self.names)
            .field("refresh", &self.refresh)
            .field("stream", &self.stream)
            .finish()
    }
}

/// Supported data providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provider {
    /// Sample prices from the built-in mock feed.
    Mock,
    /// Finnhub (REST and WebSocket).
    Finnhub,
}

const FEED_KEYS: &[&str] = &[
    "provider",
    "api_key",
    "api_key_env",
    "symbols",
    "refresh",
    "stream",
];

/// `[[feed]]` tables, primary first. The API key comes from `api_key_env`
/// (default `FINNHUB_API_KEY`) unless `api_key` is set in the file.
fn feeds(table: &Table) -> Result<Vec<FeedConfig>, String> {
    feeds_with(table, &|name| std::env::var(name).ok())
}

fn feeds_with(
    table: &Table,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Vec<FeedConfig>, String> {
    let Some(value) = table.get("feed") else {
        return Ok(Vec::new());
    };
    let Value::Array(list) = value else {
        return Err("`feed` must be one or more [[feed]] tables".to_owned());
    };
    list.iter()
        .enumerate()
        .map(|(i, v)| {
            let t = v
                .as_table()
                .ok_or_else(|| format!("feed {} must be a table", i + 1))?;
            feed(t, env).map_err(|e| format!("feed {}: {e}", i + 1))
        })
        .collect()
}

fn feed(t: &Table, env: &dyn Fn(&str) -> Option<String>) -> Result<FeedConfig, String> {
    if let Some(key) = t.keys().find(|k| !FEED_KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    let provider = match string(t, "provider", "")? {
        "finnhub" => Provider::Finnhub,
        "mock" => Provider::Mock,
        "" => return Err("missing `provider` (finnhub or mock)".to_owned()),
        other => {
            return Err(format!("`provider` must be finnhub or mock, not {other:?}"));
        }
    };
    let api_key = match t.get("api_key") {
        Some(v) => v
            .as_str()
            .ok_or_else(|| "`api_key` must be a string".to_owned())?
            .to_owned(),
        None if provider == Provider::Finnhub => {
            let var = string(t, "api_key_env", "FINNHUB_API_KEY")?;
            env(var).filter(|k| !k.trim().is_empty()).ok_or_else(|| {
                format!("no API key: set the {var} environment variable (or `api_key`)")
            })?
        }
        None => String::new(),
    };
    let names = match t.get("symbols") {
        None => Vec::new(),
        Some(Value::Table(map)) => map
            .iter()
            .map(|(symbol, name)| {
                let parsed: Symbol = symbol
                    .parse()
                    .map_err(|e| format!("`symbols`: {symbol:?}: {e}"))?;
                let name = name
                    .as_str()
                    .ok_or_else(|| format!("`symbols.{symbol}` must be a string"))?;
                Ok((parsed, name.to_owned()))
            })
            .collect::<Result<_, String>>()?,
        Some(_) => {
            return Err("`symbols` must be a table of symbol = \"provider name\"".to_owned());
        }
    };
    Ok(FeedConfig {
        provider,
        api_key: api_key.trim().to_owned(),
        names,
        refresh: Duration::from_secs_f64(number(t, "refresh", 300.0, 10.0, 86_400.0)?),
        stream: match t.get("stream") {
            None => true,
            Some(v) => v
                .as_bool()
                .ok_or_else(|| "`stream` must be true or false".to_owned())?,
        },
    })
}

const WATCHLIST_KEYS: &[&str] = &["title", "symbols"];

/// `[[watchlist]]` groups.
fn watchlist(table: &Table) -> Result<Option<Vec<Group>>, String> {
    let Some(value) = table.get("watchlist") else {
        return Ok(None);
    };
    let Value::Array(list) = value else {
        return Err("`watchlist` must be one or more [[watchlist]] tables".to_owned());
    };
    let groups = list
        .iter()
        .enumerate()
        .map(|(i, v)| watchlist_group(v).map_err(|e| format!("watchlist group {}: {e}", i + 1)))
        .collect::<Result<Vec<_>, _>>()?;
    if groups.iter().all(|g| g.symbols.is_empty()) {
        return Err("the watchlist has no symbols".to_owned());
    }
    Ok(Some(groups))
}

fn watchlist_group(v: &Value) -> Result<Group, String> {
    let t = v.as_table().ok_or_else(|| "must be a table".to_owned())?;
    if let Some(key) = t.keys().find(|k| !WATCHLIST_KEYS.contains(&k.as_str())) {
        return Err(format!("unknown key `{key}`"));
    }
    let title = match t.get("title") {
        None => None,
        Some(v) => Some(
            v.as_str()
                .ok_or_else(|| "`title` must be a string".to_owned())?
                .to_owned(),
        ),
    };
    let Some(Value::Array(symbols)) = t.get("symbols") else {
        return Err("needs `symbols = [\"AAPL\", ...]`".to_owned());
    };
    let symbols = symbols
        .iter()
        .map(|s| {
            let s = s
                .as_str()
                .ok_or_else(|| "symbols must be strings".to_owned())?;
            s.parse::<Symbol>().map_err(|e| format!("{s:?}: {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Group { title, symbols })
}

#[cfg(test)]
mod feed_tests {
    use super::*;

    fn parse_feeds(text: &str, env: &[(&str, &str)]) -> Result<Vec<FeedConfig>, String> {
        let table: Table = text.parse().unwrap();
        feeds_with(&table, &|name| {
            env.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        })
    }

    #[test]
    fn finnhub_key_from_environment() {
        let feeds = parse_feeds(
            "[[feed]]\nprovider = \"finnhub\"",
            &[("FINNHUB_API_KEY", " k3y ")],
        )
        .unwrap();
        assert_eq!(feeds[0].provider, Provider::Finnhub);
        assert_eq!(feeds[0].api_key, "k3y");
        let custom = parse_feeds(
            "[[feed]]\nprovider = \"finnhub\"\napi_key_env = \"MY_KEY\"",
            &[("MY_KEY", "abc")],
        )
        .unwrap();
        assert_eq!(custom[0].api_key, "abc");
        let missing = parse_feeds("[[feed]]\nprovider = \"finnhub\"", &[]).unwrap_err();
        assert!(
            missing.contains("set the FINNHUB_API_KEY environment variable"),
            "{missing}"
        );
    }

    #[test]
    fn keys_never_show_in_debug_output() {
        let feeds = parse_feeds(
            "[[feed]]\nprovider = \"finnhub\"\napi_key = \"secret-key\"",
            &[],
        )
        .unwrap();
        let shown = format!("{feeds:?}");
        assert!(!shown.contains("secret-key"), "{shown}");
        assert!(shown.contains("****"));
    }

    #[test]
    fn refresh_and_stream() {
        let feeds = parse_feeds(
            "[[feed]]
provider = \"finnhub\"
api_key = \"k\"
refresh = 60
stream = false",
            &[],
        )
        .unwrap();
        assert_eq!(feeds[0].refresh, Duration::from_secs(60));
        assert!(!feeds[0].stream);
        let defaults = parse_feeds(
            "[[feed]]
provider = \"mock\"",
            &[],
        )
        .unwrap();
        assert_eq!(defaults[0].refresh, Duration::from_secs(300));
        assert!(defaults[0].stream);
        assert!(
            parse_feeds(
                "[[feed]]
provider = \"mock\"
refresh = 1",
                &[]
            )
            .unwrap_err()
            .contains("between 10")
        );
    }

    #[test]
    fn symbol_names_and_errors() {
        let feeds = parse_feeds(
            "[[feed]]\nprovider = \"finnhub\"\napi_key = \"k\"\n[feed.symbols]\n\"BTC-USD\" = \"COINBASE:BTC-USD\"",
            &[],
        )
        .unwrap();
        assert_eq!(
            feeds[0].names,
            [("BTC-USD".parse().unwrap(), "COINBASE:BTC-USD".to_owned())]
        );
        assert!(
            parse_feeds("[[feed]]", &[])
                .unwrap_err()
                .contains("missing `provider`")
        );
        assert!(
            parse_feeds("[[feed]]\nprovider = \"yahoo\"", &[])
                .unwrap_err()
                .contains("finnhub or mock")
        );
        assert!(
            parse_feeds("[[feed]]\nprovider = \"mock\"\ntoken = 1", &[])
                .unwrap_err()
                .contains("unknown key `token`")
        );
    }

    #[test]
    fn watchlist_groups() {
        let c = Config::parse(
            r#"
            [[banner]]
            [[watchlist]]
            title = "Indices"
            symbols = ["SPY", "QQQ"]
            [[watchlist]]
            symbols = ["XLON:VOD"]
            "#,
        )
        .unwrap();
        let groups = c.watchlist.unwrap();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].title.as_deref(), Some("Indices"));
        assert_eq!(groups[1].symbols[0].to_string(), "XLON:VOD");
        let err = Config::parse("[[banner]]\n[[watchlist]]\nsymbols = [\"A B\"]").unwrap_err();
        assert!(err.contains("watchlist group 1"), "{err}");
        assert!(Config::parse("[[banner]]\n[[watchlist]]\ntitle = \"x\"").is_err());
        assert!(Config::parse("[[banner]]").unwrap().watchlist.is_none());
    }
}
