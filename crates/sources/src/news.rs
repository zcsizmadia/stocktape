//! News headlines from RSS 2.0 and Atom feeds.
//!
//! Many publishers offer RSS feeds for people to read; check each feed's
//! terms before showing it. Feeds are fetched on a background thread every
//! few minutes; the parser is a pure function tested on recorded feeds.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use stocktape_core::{Headline, Timestamp};

use crate::{FeedEvent, FeedHandle, Update};

/// Longest headline kept, in characters.
const MAX_TITLE: usize = 200;
/// Most headlines taken from one fetch.
const MAX_ITEMS: usize = 50;

/// A parsed feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedFeed {
    /// The feed's own title (RSS `channel/title`, Atom `feed/title`).
    pub title: Option<String>,
    /// Its items, in feed order, with `source` left empty.
    pub items: Vec<Headline>,
}

#[derive(Default)]
struct Item {
    title: String,
    link: Option<String>,
    date: Option<String>,
}

/// Resolves the predefined XML entities and character references; other
/// entities are kept as written.
fn entity(name: &str) -> String {
    match name {
        "amp" => "&".to_owned(),
        "lt" => "<".to_owned(),
        "gt" => ">".to_owned(),
        "quot" => "\"".to_owned(),
        "apos" => "'".to_owned(),
        "nbsp" => " ".to_owned(),
        _ => {
            let code = name
                .strip_prefix("#x")
                .or_else(|| name.strip_prefix("#X"))
                .map(|h| u32::from_str_radix(h, 16))
                .or_else(|| name.strip_prefix('#').map(str::parse::<u32>));
            match code.and_then(Result::ok).and_then(char::from_u32) {
                Some(c) => c.to_string(),
                None => format!("&{name};"),
            }
        }
    }
}

/// Cleans a title: decodes entities that were escaped twice (common in
/// CDATA), drops HTML tags, collapses whitespace and limits the length.
fn clean(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_tag = false;
    while let Some(c) = chars.next() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if in_tag => {}
            '&' => {
                let mut name = String::new();
                while let Some(&n) = chars.peek() {
                    if n == ';' || name.len() > 10 || n.is_whitespace() {
                        break;
                    }
                    name.push(n);
                    chars.next();
                }
                if chars.peek() == Some(&';') {
                    chars.next();
                    out.push_str(&entity(&name));
                } else {
                    out.push('&');
                    out.push_str(&name);
                }
            }
            _ => out.push(c),
        }
    }
    let collapsed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() > MAX_TITLE {
        let cut: String = collapsed.chars().take(MAX_TITLE - 1).collect();
        format!("{}…", cut.trim_end())
    } else {
        collapsed
    }
}

/// Parses an RSS 2.0 `pubDate` (RFC 2822) or Atom date (RFC 3339).
#[must_use]
pub fn parse_date(text: &str) -> Option<Timestamp> {
    let text = text.trim();
    text.parse::<Timestamp>().ok().or_else(|| {
        jiff::fmt::rfc2822::DateTimeParser::new()
            .parse_timestamp(text)
            .ok()
    })
}

fn href(e: &BytesStart<'_>) -> Option<String> {
    let rel = e.try_get_attribute("rel").ok().flatten().and_then(|a| {
        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .ok()
            .map(|v| v.into_owned())
    });
    if rel.as_deref().is_some_and(|r| r != "alternate") {
        return None;
    }
    e.try_get_attribute("href").ok().flatten().and_then(|a| {
        a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .ok()
            .map(|v| v.into_owned())
    })
}

/// Parses an RSS 2.0 or Atom document.
///
/// # Errors
///
/// Returns a message for malformed XML or a document with neither RSS items
/// nor Atom entries.
pub fn parse_feed(xml: &str) -> Result<ParsedFeed, String> {
    let mut reader = Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut feed_title: Option<String> = None;
    let mut item: Option<Item> = None;
    let mut text = String::new();
    let mut items = Vec::new();
    let mut seen_container = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("bad feed XML at byte {}: {e}", reader.buffer_position()))?;
        match event {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                match name.as_str() {
                    "item" | "entry" => item = Some(Item::default()),
                    "channel" | "feed" => seen_container = true,
                    "link" if item.is_some() => {
                        if let (Some(it), Some(h)) = (item.as_mut(), href(&e)) {
                            it.link.get_or_insert(h);
                        }
                    }
                    _ => {}
                }
                text.clear();
                path.push(name);
            }
            Event::Empty(e) => {
                let name = e.local_name();
                if name.as_ref() == b"link"
                    && let (Some(it), Some(h)) = (item.as_mut(), href(&e))
                {
                    it.link.get_or_insert(h);
                }
            }
            Event::Text(t) => {
                text.push_str(&t.decode().map_err(|e| format!("bad feed text: {e}"))?);
            }
            Event::CData(t) => {
                text.push_str(&t.decode().map_err(|e| format!("bad feed text: {e}"))?);
            }
            Event::GeneralRef(r) => {
                let name = r.decode().map_err(|e| format!("bad feed text: {e}"))?;
                text.push_str(&entity(&name));
            }
            Event::End(_) => {
                let name = path.pop().unwrap_or_default();
                let parent = path.last().map(String::as_str);
                match (name.as_str(), item.as_mut()) {
                    ("item" | "entry", Some(_)) => {
                        let done = item.take().expect("item is open");
                        let title = clean(&done.title);
                        if !title.is_empty() && items.len() < MAX_ITEMS {
                            items.push(Headline {
                                source: String::new(),
                                title,
                                published: done.date.as_deref().and_then(parse_date),
                                link: done.link,
                            });
                        }
                    }
                    ("title", Some(it)) => it.title.clone_from(&text),
                    ("link", Some(it)) if !text.trim().is_empty() => {
                        it.link.get_or_insert_with(|| text.trim().to_owned());
                    }
                    ("pubDate" | "published" | "updated" | "date", Some(it)) => {
                        // Prefer the publication date over "updated".
                        if it.date.is_none() || name != "updated" {
                            it.date = Some(text.clone());
                        }
                    }
                    ("title", None) if matches!(parent, Some("channel" | "feed")) => {
                        let t = clean(&text);
                        if feed_title.is_none() && !t.is_empty() {
                            feed_title = Some(t);
                        }
                    }
                    _ => {}
                }
                text.clear();
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !seen_container && items.is_empty() {
        return Err("not an RSS or Atom feed".to_owned());
    }
    Ok(ParsedFeed {
        title: feed_title,
        items,
    })
}

/// Settings for one news feed.
#[derive(Debug, Clone)]
pub struct NewsConfig {
    /// The feed's URL.
    pub url: String,
    /// Label shown before its headlines; the feed's own title when `None`.
    pub name: Option<String>,
    /// Time between fetches.
    pub refresh: Duration,
}

/// Fetches a feed every `refresh` on a background thread and sends its
/// headlines (tagged with the source name) as [`Update::Headlines`].
#[must_use]
pub fn spawn(config: NewsConfig, feed: usize, sink: Sender<FeedEvent>) -> FeedHandle {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::Builder::new()
        .name("news".to_owned())
        .spawn(move || {
            let send = |update| sink.send(FeedEvent { feed, update }).is_ok();
            while !flag.load(Ordering::Relaxed) {
                let result = ureq::get(&config.url)
                    .call()
                    .map_err(|e| e.to_string())
                    .and_then(|mut r| {
                        r.body_mut()
                            .with_config()
                            .limit(10 * 1024 * 1024)
                            .read_to_string()
                            .map_err(|e| e.to_string())
                    })
                    .and_then(|body| parse_feed(&body));
                let update = match result {
                    Ok(parsed) => {
                        let source = config
                            .name
                            .clone()
                            .or(parsed.title)
                            .unwrap_or_else(|| "News".to_owned());
                        let items = parsed
                            .items
                            .into_iter()
                            .map(|h| Headline {
                                source: source.clone(),
                                ..h
                            })
                            .collect();
                        Update::Headlines { source, items }
                    }
                    Err(e) => Update::Status {
                        connected: false,
                        message: format!("news {}: {e}", config.url),
                    },
                };
                if !send(update) {
                    return;
                }
                let end = Instant::now() + config.refresh;
                while !flag.load(Ordering::Relaxed) && Instant::now() < end {
                    std::thread::sleep(Duration::from_millis(200));
                }
            }
        })
        .expect("can spawn the news thread");
    FeedHandle::new(stop, thread)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <channel>
    <title>Markets Desk</title>
    <link>https://example.com/</link>
    <item>
      <title>Stocks rise as &amp; inflation cools</title>
      <link>https://example.com/a</link>
      <pubDate>Thu, 12 Nov 2026 15:30:00 GMT</pubDate>
    </item>
    <item>
      <title><![CDATA[Fed <b>holds</b> rates; chair says &#8220;patience&#8221;]]></title>
      <dc:date>2026-11-12T14:00:00Z</dc:date>
    </item>
    <item><title>   </title></item>
  </channel>
</rss>"#;

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title type="text">Example Atom</title>
  <entry>
    <title>Oil slips on supply worries</title>
    <link rel="alternate" href="https://example.com/oil"/>
    <link rel="edit" href="https://example.com/edit"/>
    <updated>2026-11-12T16:00:00Z</updated>
    <published>2026-11-12T15:00:00Z</published>
  </entry>
</feed>"#;

    #[test]
    fn rss() {
        let feed = parse_feed(RSS).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Markets Desk"));
        assert_eq!(feed.items.len(), 2, "empty titles are skipped");
        assert_eq!(feed.items[0].title, "Stocks rise as & inflation cools");
        assert_eq!(feed.items[0].link.as_deref(), Some("https://example.com/a"));
        assert_eq!(
            feed.items[0].published,
            Some("2026-11-12T15:30:00Z".parse().unwrap())
        );
        assert_eq!(
            feed.items[1].title,
            "Fed holds rates; chair says \u{201c}patience\u{201d}"
        );
        assert_eq!(
            feed.items[1].published,
            Some("2026-11-12T14:00:00Z".parse().unwrap())
        );
    }

    #[test]
    fn atom() {
        let feed = parse_feed(ATOM).unwrap();
        assert_eq!(feed.title.as_deref(), Some("Example Atom"));
        let item = &feed.items[0];
        assert_eq!(item.title, "Oil slips on supply worries");
        assert_eq!(item.link.as_deref(), Some("https://example.com/oil"));
        assert_eq!(
            item.published,
            Some("2026-11-12T15:00:00Z".parse().unwrap())
        );
    }

    #[test]
    fn errors_and_cleaning() {
        assert!(parse_feed("<html><body>hi</body></html>").is_err());
        assert!(parse_feed("<rss><channel><item><title>x</title></rss>").is_err());
        assert_eq!(clean("a  &amp;amp; <i>b</i>\n c"), "a &amp; b c");
        assert_eq!(clean("Tom &amp Jerry"), "Tom &amp Jerry");
        let long = "word ".repeat(100);
        assert_eq!(clean(&long).chars().count(), MAX_TITLE);
        assert_eq!(parse_date("not a date"), None);
    }

    proptest! {
        #[test]
        fn parser_never_panics(text in ".{0,400}") {
            let _ = parse_feed(&text);
            let _ = clean(&text);
            let _ = parse_date(&text);
        }

        #[test]
        fn parser_never_panics_on_feed_shaped_input(title in ".{0,80}", date in ".{0,40}") {
            let xml = format!("<rss><channel><title>{title}</title><item><title>{title}</title><pubDate>{date}</pubDate></item></channel></rss>");
            let _ = parse_feed(&xml);
        }
    }
}
