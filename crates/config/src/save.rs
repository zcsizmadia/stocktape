//! Writing a configuration back to TOML, plus helpers shared with the
//! settings window.

use std::path::PathBuf;

use stocktape_layout::{DisplaySelector, Edge, Placement};
use toml::value::Array;
use toml::{Table, Value};

use crate::{
    BannerConfig, Config, Content, FeedConfig, KeySource, LineConfig, Provider, StrategyKind,
};

/// The standard location of the configuration file: `stocktape/stocktape.toml`
/// in the platform's config folder (`%APPDATA%` on Windows,
/// `~/.config` on Linux, `~/Library/Application Support` on macOS).
#[must_use]
pub fn default_path() -> Option<PathBuf> {
    config_dir(&|name| std::env::var_os(name)).map(|d| d.join("stocktape").join("stocktape.toml"))
}

/// The platform's per-user configuration folder, from environment variables.
fn config_dir(env: &dyn Fn(&str) -> Option<std::ffi::OsString>) -> Option<PathBuf> {
    let non_empty = |name: &str| env(name).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(windows) {
        non_empty("APPDATA")
    } else if cfg!(target_os = "macos") {
        non_empty("HOME").map(|h| h.join("Library").join("Application Support"))
    } else {
        non_empty("XDG_CONFIG_HOME")
            .filter(|p| p.is_absolute())
            .or_else(|| non_empty("HOME").map(|h| h.join(".config")))
    }
}

impl StrategyKind {
    /// Every strategy, for pickers.
    pub const ALL: [Self; 3] = [Self::Demo, Self::Watchlist, Self::Movers];

    /// The name used in the configuration file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Demo => "demo",
            Self::Watchlist => "watchlist",
            Self::Movers => "movers",
        }
    }
}

/// Writes a display selector the way `displays` and `exclude` accept it in
/// text form: a keyword, or names and indexes separated by commas
/// (`"landscape"`, `"DISPLAY1, 2"`).
#[must_use]
pub fn selector_to_text(selector: &DisplaySelector) -> String {
    match selector {
        DisplaySelector::Primary => "primary".to_owned(),
        DisplaySelector::All => "all".to_owned(),
        DisplaySelector::Landscape => "landscape".to_owned(),
        DisplaySelector::Portrait => "portrait".to_owned(),
        DisplaySelector::Name(n) => n.clone(),
        DisplaySelector::Index(i) => i.to_string(),
        DisplaySelector::AnyOf(list) => list
            .iter()
            .map(selector_to_text)
            .collect::<Vec<_>>()
            .join(", "),
        DisplaySelector::Except(keep, _) => selector_to_text(keep),
    }
}

/// Reads the text form back: a keyword, or comma-separated names and
/// indexes. An empty text means the primary display.
///
/// # Errors
///
/// Returns a message for an empty entry in a list.
pub fn selector_from_text(text: &str) -> Result<DisplaySelector, String> {
    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() == 1 && parts[0].is_empty() {
        return Ok(DisplaySelector::Primary);
    }
    let one = |p: &str| -> Result<DisplaySelector, String> {
        if p.is_empty() {
            return Err("an entry in the list is empty".to_owned());
        }
        Ok(match p {
            "primary" => DisplaySelector::Primary,
            "all" => DisplaySelector::All,
            "landscape" => DisplaySelector::Landscape,
            "portrait" => DisplaySelector::Portrait,
            _ => match p.parse::<usize>() {
                Ok(i) => DisplaySelector::Index(i),
                Err(_) => DisplaySelector::Name(p.strip_prefix("name:").unwrap_or(p).to_owned()),
            },
        })
    };
    if parts.len() == 1 {
        one(parts[0])
    } else {
        parts
            .into_iter()
            .map(one)
            .collect::<Result<_, _>>()
            .map(DisplaySelector::AnyOf)
    }
}

fn selector_value(selector: &DisplaySelector) -> Value {
    match selector {
        DisplaySelector::Index(i) => Value::Integer(i64::try_from(*i).unwrap_or(i64::MAX)),
        DisplaySelector::AnyOf(list) => {
            Value::Array(list.iter().map(selector_value).collect::<Array>())
        }
        other => Value::String(selector_to_text(other)),
    }
}

fn number(v: f64) -> Value {
    #[allow(clippy::cast_possible_truncation)]
    if v.fract() == 0.0 && v.abs() < 1e15 {
        Value::Integer(v as i64)
    } else {
        Value::Float(v)
    }
}

fn line_table(t: &mut Table, line: &LineConfig) {
    t.insert("skin".into(), Value::String(line.skin.clone()));
    t.insert("speed".into(), number(line.speed));
    t.insert(
        "strategy".into(),
        Value::String(line.strategy.name().into()),
    );
    if line.content != Content::Quotes {
        t.insert("content".into(), Value::String(line.content.name().into()));
    }
    if !line.messages.is_empty() {
        t.insert(
            "messages".into(),
            Value::Array(
                line.messages
                    .iter()
                    .map(|m| Value::String(m.clone()))
                    .collect(),
            ),
        );
    }
}

fn banner_table(b: &BannerConfig) -> Table {
    let mut t = Table::new();
    t.insert("name".into(), Value::String(b.name.clone()));
    let (place, displays) = match &b.placement {
        Placement::Edge { displays, edge } => (
            match edge {
                Edge::Top => "top",
                Edge::Bottom => "bottom",
                Edge::Left => "left",
                Edge::Right => "right",
            },
            displays,
        ),
        Placement::Around { displays } => ("around", displays),
    };
    t.insert("place".into(), Value::String(place.into()));
    let (include, exclude) = match displays {
        DisplaySelector::Except(keep, drop) => (keep.as_ref(), Some(drop.as_ref())),
        other => (other, None),
    };
    t.insert("displays".into(), selector_value(include));
    if let Some(drop) = exclude {
        t.insert("exclude".into(), selector_value(drop));
    }
    if b.options.offset != 0.0 {
        t.insert("offset".into(), number(b.options.offset));
    }
    if b.options.inset != (0.0, 0.0) {
        t.insert(
            "inset".into(),
            Value::Array(
                vec![number(b.options.inset.0), number(b.options.inset.1)]
                    .into_iter()
                    .collect(),
            ),
        );
    }
    if b.options.reverse {
        t.insert("reverse".into(), Value::Boolean(true));
    }
    if !b.dock {
        t.insert("dock".into(), Value::Boolean(false));
    }
    match b.lines.as_slice() {
        [single] => line_table(&mut t, single),
        lines => {
            let list: Array = lines
                .iter()
                .map(|l| {
                    let mut lt = Table::new();
                    line_table(&mut lt, l);
                    Value::Table(lt)
                })
                .collect();
            t.insert("line".into(), Value::Array(list));
        }
    }
    t
}

fn feed_table(f: &FeedConfig) -> Table {
    let mut t = Table::new();
    t.insert("provider".into(), Value::String(f.provider.name().into()));
    match &f.key_source {
        // A key that was written in the file stays there; keys from the
        // environment are never written.
        KeySource::Inline => {
            t.insert("api_key".into(), Value::String(f.api_key.clone()));
        }
        KeySource::Env(var) if var != f.provider.default_key_env() => {
            t.insert("api_key_env".into(), Value::String(var.clone()));
        }
        KeySource::Env(_) => {}
    }
    #[allow(clippy::cast_possible_truncation)]
    t.insert("refresh".into(), number(f.refresh.as_secs_f64()));
    if f.provider == Provider::Finnhub && !f.stream {
        t.insert("stream".into(), Value::Boolean(false));
    }
    if f.provider == Provider::Massive {
        t.insert(
            "requests_per_minute".into(),
            Value::Integer(i64::from(f.requests_per_minute)),
        );
    }
    if !f.names.is_empty() {
        let names: Table = f
            .names
            .iter()
            .map(|(s, n)| (s.to_string(), Value::String(n.clone())))
            .collect();
        t.insert("symbols".into(), Value::Table(names));
    }
    t
}

impl Config {
    /// The configuration as TOML text that [`Config::parse`] reads back to an
    /// equal configuration. API keys from the environment are never written.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut root = Table::new();
        if !self.feeds.is_empty() {
            root.insert(
                "feed".into(),
                Value::Array(
                    self.feeds
                        .iter()
                        .map(|f| Value::Table(feed_table(f)))
                        .collect(),
                ),
            );
        }
        if let Some(groups) = &self.watchlist {
            root.insert(
                "watchlist".into(),
                Value::Array(
                    groups
                        .iter()
                        .map(|g| {
                            let mut t = Table::new();
                            if let Some(title) = &g.title {
                                t.insert("title".into(), Value::String(title.clone()));
                            }
                            t.insert(
                                "symbols".into(),
                                Value::Array(
                                    g.symbols
                                        .iter()
                                        .map(|s| Value::String(s.to_string()))
                                        .collect(),
                                ),
                            );
                            Value::Table(t)
                        })
                        .collect(),
                ),
            );
        }
        if !self.news.is_empty() {
            root.insert(
                "news".into(),
                Value::Array(
                    self.news
                        .iter()
                        .map(|n| {
                            let mut t = Table::new();
                            t.insert("url".into(), Value::String(n.url.clone()));
                            if let Some(name) = &n.name {
                                t.insert("name".into(), Value::String(name.clone()));
                            }
                            t.insert("refresh".into(), number(n.refresh.as_secs_f64()));
                            Value::Table(t)
                        })
                        .collect(),
                ),
            );
        }
        root.insert(
            "banner".into(),
            Value::Array(
                self.banners
                    .iter()
                    .map(|b| Value::Table(banner_table(b)))
                    .collect(),
            ),
        );
        format!(
            "# stocktape configuration (see docs/configuration.md).\n\
             # Written by the settings window; comments are not kept.\n\n{root}"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presets;

    fn round_trip(c: &Config) {
        let text = c.to_toml();
        let back = Config::parse(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        assert_eq!(&back, c, "{text}");
    }

    #[test]
    fn presets_round_trip() {
        for (name, _) in presets() {
            round_trip(&Config::preset(name).unwrap());
        }
    }

    #[test]
    fn everything_round_trips() {
        let c = Config::parse(
            r#"
            [[feed]]
            provider = "finnhub"
            api_key = "inline-key"
            refresh = 120
            stream = false
            [feed.symbols]
            "BTC-USD" = "COINBASE:BTC-USD"

            [[feed]]
            provider = "massive"
            api_key_env = "MY_MASSIVE"
            requests_per_minute = 100
            refresh = 15.5

            [[watchlist]]
            title = "Tech"
            symbols = ["AAPL", "XLON:VOD"]
            [[watchlist]]
            symbols = ["BRK.B"]

            [[banner]]
            name = "Side"
            place = "right"
            displays = ["DISPLAY2", 0]
            exclude = "portrait"
            offset = 12
            inset = [10, 20.5]
            reverse = true
            dock = false
            skin = "glass"
            speed = 77.5
            strategy = "movers"

            [[banner]]
            place = "around"
            displays = "landscape"
            [[banner.line]]
            strategy = "movers"
            speed = 180
            [[banner.line]]
            skin = "daylight"
            content = "mixed"
            messages = ["Markets close at 4 PM", "Hello"]

            [[banner]]
            content = "headlines"

            [[news]]
            url = "https://example.com/markets.rss"
            name = "Desk"
            refresh = 600
            [[news]]
            url = "http://example.org/atom.xml"
            "#,
        )
        .unwrap();
        round_trip(&c);
    }

    #[test]
    fn environment_keys_are_never_written() {
        let c = Config::parse("[[feed]]\nprovider = \"finnhub\"\n[[banner]]").unwrap();
        let mut with_key = c.clone();
        with_key.feeds[0].api_key = "from-environment".into();
        let text = with_key.to_toml();
        assert!(!text.contains("from-environment"), "{text}");
        assert!(!text.contains("api_key"), "{text}");
    }

    #[test]
    fn selector_text() {
        for text in ["primary", "all", "landscape", "portrait", "DISPLAY1", "2"] {
            assert_eq!(selector_to_text(&selector_from_text(text).unwrap()), text);
        }
        let list = selector_from_text("DISPLAY2, 0 , name:DELL").unwrap();
        assert_eq!(
            list,
            DisplaySelector::AnyOf(vec![
                DisplaySelector::Name("DISPLAY2".into()),
                DisplaySelector::Index(0),
                DisplaySelector::Name("DELL".into()),
            ])
        );
        assert_eq!(selector_to_text(&list), "DISPLAY2, 0, DELL");
        assert_eq!(selector_from_text("  ").unwrap(), DisplaySelector::Primary);
        assert!(selector_from_text("a,,b").is_err());
    }

    #[test]
    fn config_dir_from_environment() {
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                vars.iter()
                    .find(|(k, _)| *k == name)
                    .map(|(_, v)| std::ffi::OsString::from(*v))
            }
        };
        if cfg!(windows) {
            assert_eq!(
                config_dir(&env(&[("APPDATA", r"C:UsersmeAppDataRoaming")])),
                Some(PathBuf::from(r"C:UsersmeAppDataRoaming"))
            );
            assert_eq!(config_dir(&env(&[])), None);
        } else if !cfg!(target_os = "macos") {
            assert_eq!(
                config_dir(&env(&[("XDG_CONFIG_HOME", "/x"), ("HOME", "/h")])),
                Some(PathBuf::from("/x"))
            );
            assert_eq!(
                config_dir(&env(&[("XDG_CONFIG_HOME", "relative"), ("HOME", "/h")])),
                Some(PathBuf::from("/h/.config"))
            );
        }
    }

    #[test]
    fn default_path_ends_in_stocktape() {
        if let Some(p) = default_path() {
            assert!(p.ends_with("stocktape/stocktape.toml"));
        }
    }
}
