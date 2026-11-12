//! News headlines.

use jiff::Timestamp;

/// One news headline.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Headline {
    /// Who published it, e.g. the feed's name.
    pub source: String,
    /// The headline text.
    pub title: String,
    /// When it was published, if the feed says.
    pub published: Option<Timestamp>,
    /// Link to the article, if any.
    pub link: Option<String>,
}
