//! The stocktape settings window (egui).
//!
//! It edits the configuration file: banners and their lines, feeds, the
//! watchlist, and skins with a live preview. Saving writes the file (keeping
//! the previous one as `.bak`); a running stocktape picks the change up
//! within a moment. API keys are never shown or written: feeds name the
//! environment variable that holds the key.
//!
//! Part of [stocktape](https://github.com/zcsizmadia/stocktape). See
//! `docs/design-notes.md` for the design.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use eframe::egui;
use stocktape_config::{
    BannerConfig, Config, Content, FeedConfig, KeySource, LineConfig, NewsConfig, Provider,
    StrategyKind, presets, selector_from_text, selector_to_text,
};
use stocktape_core::Symbol;
use stocktape_layout::{DisplaySelector, Edge, Placement, PlacementOptions};
use stocktape_skin::{Skin, SkinLibrary};
use stocktape_strategies::Group;
use tiny_skia::Pixmap;

/// Renders a skin preview `width` pixels wide (supplied by the app, which
/// owns the ticker).
pub type PreviewFn = Box<dyn FnMut(&Skin, u32) -> Pixmap>;

/// Width of skin previews, in pixels.
const PREVIEW_WIDTH: u32 = 900;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Banners,
    Feeds,
    Watchlist,
    Skins,
}

impl Tab {
    const ALL: [Self; 4] = [Self::Banners, Self::Feeds, Self::Watchlist, Self::Skins];

    fn label(self) -> &'static str {
        match self {
            Self::Banners => "Banners",
            Self::Feeds => "Feeds",
            Self::Watchlist => "Watchlist",
            Self::Skins => "Skins",
        }
    }
}

/// Text being edited for fields whose text isn't valid yet.
#[derive(Default)]
struct Drafts {
    /// Per banner: (displays, exclude).
    displays: Vec<(String, String)>,
    /// Per watchlist group: symbols.
    symbols: Vec<String>,
}

/// The settings window's state.
pub struct SettingsApp {
    path: PathBuf,
    config: Config,
    saved: String,
    library: SkinLibrary,
    tab: Tab,
    status: Option<(String, bool)>,
    preview: Option<PreviewFn>,
    previews: HashMap<String, egui::TextureHandle>,
    selected_skin: String,
    drafts: Drafts,
}

impl std::fmt::Debug for SettingsApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SettingsApp")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

fn displays_of(placement: &Placement) -> &DisplaySelector {
    match placement {
        Placement::Edge { displays, .. } | Placement::Around { displays } => displays,
    }
}

fn set_displays(placement: &mut Placement, selector: DisplaySelector) {
    match placement {
        Placement::Edge { displays, .. } | Placement::Around { displays } => *displays = selector,
    }
}

fn split_selector(selector: &DisplaySelector) -> (String, String) {
    match selector {
        DisplaySelector::Except(keep, drop) => (selector_to_text(keep), selector_to_text(drop)),
        other => (selector_to_text(other), String::new()),
    }
}

fn place_name(placement: &Placement) -> &'static str {
    match placement {
        Placement::Edge {
            edge: Edge::Top, ..
        } => "top",
        Placement::Edge {
            edge: Edge::Bottom, ..
        } => "bottom",
        Placement::Edge {
            edge: Edge::Left, ..
        } => "left",
        Placement::Edge {
            edge: Edge::Right, ..
        } => "right",
        Placement::Around { .. } => "around",
    }
}

fn with_place(placement: &Placement, name: &str) -> Placement {
    let displays = displays_of(placement).clone();
    let edge = match name {
        "top" => Edge::Top,
        "left" => Edge::Left,
        "right" => Edge::Right,
        "around" => return Placement::Around { displays },
        _ => Edge::Bottom,
    };
    Placement::Edge { displays, edge }
}

/// Parses watchlist text: symbols separated by commas, spaces or new lines.
fn parse_symbols(text: &str) -> Result<Vec<Symbol>, String> {
    text.split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<Symbol>().map_err(|e| format!("{s}: {e}")))
        .collect()
}

fn symbols_text(group: &Group) -> String {
    group
        .symbols
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

impl SettingsApp {
    /// Opens `config` (loaded from, and saved to, `path`).
    #[must_use]
    pub fn new(
        path: PathBuf,
        config: Config,
        library: SkinLibrary,
        preview: Option<PreviewFn>,
    ) -> Self {
        // A file that doesn't exist yet starts out unsaved.
        let saved = if path.exists() {
            config.to_toml()
        } else {
            String::new()
        };
        let selected_skin = SkinLibrary::DEFAULT.to_owned();
        let mut app = Self {
            path,
            config,
            saved,
            library,
            tab: Tab::Banners,
            status: None,
            preview,
            previews: HashMap::new(),
            selected_skin,
            drafts: Drafts::default(),
        };
        app.reset_drafts();
        app
    }

    /// The configuration as currently edited.
    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// Whether there are unsaved changes.
    #[must_use]
    pub fn is_dirty(&self) -> bool {
        self.config.to_toml() != self.saved
    }

    fn reset_drafts(&mut self) {
        self.drafts.displays = self
            .config
            .banners
            .iter()
            .map(|b| split_selector(displays_of(&b.placement)))
            .collect();
        self.drafts.symbols = self
            .config
            .watchlist
            .iter()
            .flatten()
            .map(symbols_text)
            .collect();
    }

    /// Writes the configuration, keeping the previous file as `.bak`.
    ///
    /// # Errors
    ///
    /// Returns a message if the file can't be written, or if the edited
    /// configuration wouldn't read back (which would be a bug).
    pub fn save(&mut self) -> Result<(), String> {
        let text = self.config.to_toml();
        Config::parse(&text)
            .map_err(|e| format!("not saved, the result doesn't read back: {e}"))?;
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("can't create {}: {e}", dir.display()))?;
        }
        if self.path.exists() {
            let backup = self.path.with_extension("toml.bak");
            std::fs::copy(&self.path, &backup)
                .map_err(|e| format!("can't keep a backup at {}: {e}", backup.display()))?;
        }
        std::fs::write(&self.path, &text)
            .map_err(|e| format!("can't write {}: {e}", self.path.display()))?;
        self.saved = text;
        Ok(())
    }

    /// Draws the whole window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("actions").show(ui, |ui| self.actions(ui));
        egui::Panel::left("tabs").exact_size(140.0).show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading("stocktape");
            ui.add_space(8.0);
            for tab in Tab::ALL {
                ui.selectable_value(&mut self.tab, tab, tab.label());
            }
        });
        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink(false)
                .show(ui, |ui| match self.tab {
                    Tab::Banners => self.banners(ui),
                    Tab::Feeds => self.feeds(ui),
                    Tab::Watchlist => self.watchlist(ui),
                    Tab::Skins => self.skins(ui),
                });
        });
    }

    fn actions(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let dirty = self.is_dirty();
            if ui.add_enabled(dirty, egui::Button::new("Save")).clicked() {
                self.status = Some(match self.save() {
                    Ok(()) => (
                        "Saved. Running banners update automatically.".to_owned(),
                        false,
                    ),
                    Err(e) => (e, true),
                });
            }
            if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked()
                && let Ok(config) = Config::parse(&self.saved)
            {
                self.config = config;
                self.reset_drafts();
                self.status = Some(("Reverted to the saved file.".to_owned(), false));
            }
            ui.label(egui::RichText::new(self.path.display().to_string()).weak());
        });
        if let Some((message, error)) = &self.status {
            let text = egui::RichText::new(message);
            ui.label(if *error {
                text.color(ui.visuals().error_fg_color)
            } else {
                text
            });
        }
        ui.add_space(6.0);
    }

    fn banners(&mut self, ui: &mut egui::Ui) {
        ui.heading("Banners");
        ui.horizontal(|ui| {
            let label = ui.label("Start from a preset:");
            egui::ComboBox::from_id_salt("preset")
                .selected_text("choose…")
                .show_ui(ui, |ui| {
                    for (name, description) in presets() {
                        if ui
                            .selectable_label(false, name)
                            .on_hover_text(description)
                            .clicked()
                            && let Ok(preset) = Config::preset(name)
                        {
                            self.config.banners = preset.banners;
                            self.reset_drafts();
                        }
                    }
                })
                .response
                .labelled_by(label.id);
        });
        ui.separator();
        let skins: Vec<String> = self.library.ids().map(str::to_owned).collect();
        let mut remove = None;
        for i in 0..self.config.banners.len() {
            let title = format!(
                "{}  ·  {}",
                self.config.banners[i].name,
                place_name(&self.config.banners[i].placement)
            );
            egui::CollapsingHeader::new(title)
                .id_salt(("banner", i))
                .default_open(true)
                .show(ui, |ui| {
                    if banner_editor(
                        ui,
                        i,
                        &mut self.config.banners[i],
                        &mut self.drafts.displays[i],
                        &skins,
                    ) {
                        remove = Some(i);
                    }
                });
        }
        if let Some(i) = remove {
            self.config.banners.remove(i);
            self.drafts.displays.remove(i);
        }
        ui.add_space(8.0);
        if ui.button("Add banner").clicked() {
            self.config.banners.push(BannerConfig {
                name: format!("Banner {}", self.config.banners.len() + 1),
                placement: Placement::Edge {
                    displays: DisplaySelector::Primary,
                    edge: Edge::Bottom,
                },
                options: PlacementOptions::default(),
                lines: vec![default_line()],
                dock: true,
            });
            self.drafts
                .displays
                .push(("primary".to_owned(), String::new()));
        }
    }

    fn feeds(&mut self, ui: &mut egui::Ui) {
        ui.heading("Feeds");
        ui.label("Feeds are tried in order: the first is the primary, later ones take over a symbol when the ones before go quiet.");
        ui.label(egui::RichText::new("API keys are never shown or saved here: each feed reads its key from an environment variable.").weak());
        ui.separator();
        if self.config.feeds.is_empty() {
            ui.label("No feeds: banners show sample prices.");
        }
        let mut remove = None;
        let mut swap = None;
        let count = self.config.feeds.len();
        for (i, feed) in self.config.feeds.iter_mut().enumerate() {
            ui.push_id(("feed", i), |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label(format!("{}. Provider", i + 1));
                        egui::ComboBox::from_id_salt("provider")
                            .selected_text(feed.provider.name())
                            .show_ui(ui, |ui| {
                                for p in Provider::ALL {
                                    ui.selectable_value(&mut feed.provider, p, p.name());
                                }
                            })
                            .response
                            .labelled_by(label.id);
                        if ui.add_enabled(i > 0, egui::Button::new("Up")).clicked() {
                            swap = Some((i, i - 1));
                        }
                        if ui
                            .add_enabled(i + 1 < count, egui::Button::new("Down"))
                            .clicked()
                        {
                            swap = Some((i, i + 1));
                        }
                        if ui.button("Remove feed").clicked() {
                            remove = Some(i);
                        }
                    });
                    feed_editor(ui, feed);
                });
            });
        }
        if let Some((a, b)) = swap {
            self.config.feeds.swap(a, b);
        }
        if let Some(i) = remove {
            self.config.feeds.remove(i);
        }
        ui.add_space(8.0);
        if ui.button("Add feed").clicked() {
            self.config.feeds.push(FeedConfig {
                provider: Provider::Finnhub,
                key_source: KeySource::Env(Provider::Finnhub.default_key_env().to_owned()),
                api_key: String::new(),
                names: Vec::new(),
                refresh: Duration::from_secs(300),
                requests_per_minute: 60,
                stream: true,
            });
        }
        ui.add_space(16.0);
        ui.heading("News");
        ui.label("RSS or Atom feeds for lines that show headlines. Check each feed's terms.");
        let mut remove_news = None;
        for (i, n) in self.config.news.iter_mut().enumerate() {
            ui.push_id(("news", i), |ui| {
                ui.horizontal(|ui| {
                    let label = ui.label("URL");
                    ui.add(egui::TextEdit::singleline(&mut n.url).desired_width(360.0))
                        .labelled_by(label.id);
                    let label = ui.label("Label");
                    let mut name = n.name.clone().unwrap_or_default();
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut name)
                                .hint_text("feed title")
                                .desired_width(120.0),
                        )
                        .labelled_by(label.id)
                        .changed()
                    {
                        n.name = (!name.trim().is_empty()).then(|| name.trim().to_owned());
                    }
                    let mut secs = n.refresh.as_secs_f64();
                    if ui
                        .add(
                            egui::DragValue::new(&mut secs)
                                .range(60.0..=86_400.0)
                                .prefix("every ")
                                .suffix(" s"),
                        )
                        .changed()
                    {
                        n.refresh = Duration::from_secs_f64(secs);
                    }
                    if ui.button("Remove news feed").clicked() {
                        remove_news = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove_news {
            self.config.news.remove(i);
        }
        if ui.button("Add news feed").clicked() {
            self.config.news.push(NewsConfig {
                url: "https://".to_owned(),
                name: None,
                refresh: Duration::from_secs(300),
            });
        }
    }

    fn watchlist(&mut self, ui: &mut egui::Ui) {
        ui.heading("Watchlist");
        let mut custom = self.config.watchlist.is_some();
        if ui
            .checkbox(
                &mut custom,
                "Use my own watchlist (otherwise the demo list)",
            )
            .changed()
        {
            self.config.watchlist = custom.then(|| {
                vec![Group {
                    title: Some("Watchlist".to_owned()),
                    symbols: vec![
                        "AAPL".parse().expect("valid"),
                        "MSFT".parse().expect("valid"),
                    ],
                }]
            });
            self.reset_drafts();
        }
        let Some(groups) = &mut self.config.watchlist else {
            return;
        };
        ui.label(egui::RichText::new("Symbols are separated by commas or spaces; use EXCHANGE:TICKER for other exchanges, e.g. XLON:VOD.").weak());
        ui.separator();
        let mut remove = None;
        for (i, group) in groups.iter_mut().enumerate() {
            ui.push_id(("group", i), |ui| {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let label = ui.label("Title");
                        let mut title = group.title.clone().unwrap_or_default();
                        if ui
                            .text_edit_singleline(&mut title)
                            .labelled_by(label.id)
                            .changed()
                        {
                            group.title =
                                (!title.trim().is_empty()).then(|| title.trim().to_owned());
                        }
                        if ui.button("Remove group").clicked() {
                            remove = Some(i);
                        }
                    });
                    let draft = &mut self.drafts.symbols[i];
                    let label = ui.label("Symbols");
                    if ui
                        .add(
                            egui::TextEdit::multiline(draft)
                                .desired_rows(2)
                                .desired_width(f32::INFINITY),
                        )
                        .labelled_by(label.id)
                        .changed()
                        && let Ok(symbols) = parse_symbols(draft)
                    {
                        group.symbols = symbols;
                    }
                    if let Err(e) = parse_symbols(draft) {
                        ui.colored_label(ui.visuals().error_fg_color, e);
                    }
                });
            });
        }
        if let Some(i) = remove {
            groups.remove(i);
            self.drafts.symbols.remove(i);
        }
        if ui.button("Add group").clicked() {
            groups.push(Group {
                title: None,
                symbols: Vec::new(),
            });
            self.drafts.symbols.push(String::new());
        }
    }

    fn skins(&mut self, ui: &mut egui::Ui) {
        ui.heading("Skins");
        ui.label(egui::RichText::new("Pick a skin to preview it. Skins are set per banner line on the Banners tab; see docs/skins.md to make your own.").weak());
        ui.separator();
        let ids: Vec<String> = self.library.ids().map(str::to_owned).collect();
        ui.horizontal_wrapped(|ui| {
            for id in &ids {
                ui.selectable_value(&mut self.selected_skin, id.clone(), id);
            }
        });
        ui.add_space(8.0);
        match self.library.resolve(&self.selected_skin) {
            Ok(skin) => {
                ui.strong(&skin.name);
                ui.label(&skin.description);
                if let Some(texture) = self.preview_texture(ui.ctx(), &skin) {
                    let size = texture.size_vec2();
                    let width = ui.available_width().min(size.x);
                    ui.add(
                        egui::Image::new(&texture)
                            .fit_to_exact_size(egui::vec2(width, size.y * width / size.x)),
                    );
                }
            }
            Err(e) => {
                ui.colored_label(ui.visuals().error_fg_color, e.to_string());
            }
        }
    }

    fn preview_texture(&mut self, ctx: &egui::Context, skin: &Skin) -> Option<egui::TextureHandle> {
        if let Some(t) = self.previews.get(&skin.id) {
            return Some(t.clone());
        }
        let render = self.preview.as_mut()?;
        let pixmap = render(skin, PREVIEW_WIDTH);
        let image = egui::ColorImage::from_rgba_premultiplied(
            [pixmap.width() as usize, pixmap.height() as usize],
            pixmap.data(),
        );
        let texture = ctx.load_texture(
            format!("skin-{}", skin.id),
            image,
            egui::TextureOptions::LINEAR,
        );
        self.previews.insert(skin.id.clone(), texture.clone());
        Some(texture)
    }
}

fn default_line() -> LineConfig {
    LineConfig {
        skin: SkinLibrary::DEFAULT.to_owned(),
        speed: 120.0,
        strategy: StrategyKind::Demo,
        content: Content::Quotes,
        messages: Vec::new(),
    }
}

/// Edits one banner; returns true when "Remove banner" was clicked.
fn banner_editor(
    ui: &mut egui::Ui,
    index: usize,
    banner: &mut BannerConfig,
    draft: &mut (String, String),
    skins: &[String],
) -> bool {
    let mut remove = false;
    egui::Grid::new(("banner-grid", index)).num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
        let label = ui.label("Name");
        ui.text_edit_singleline(&mut banner.name).labelled_by(label.id);
        ui.end_row();

        let label = ui.label("Place");
        let current = place_name(&banner.placement);
        egui::ComboBox::from_id_salt(("place", index))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for name in ["top", "bottom", "left", "right", "around"] {
                    if ui.selectable_label(current == name, name).clicked() {
                        banner.placement = with_place(&banner.placement, name);
                    }
                }
            })
            .response
            .labelled_by(label.id);
        ui.end_row();

        let label = ui.label("Displays");
        ui.horizontal(|ui| {
            let mut changed = ui
                .add(egui::TextEdit::singleline(&mut draft.0).hint_text("primary").desired_width(220.0))
                .labelled_by(label.id)
                .on_hover_text("primary, all, landscape, portrait, or display names/indexes separated by commas")
                .changed();
            for keyword in ["primary", "all", "landscape"] {
                if ui.small_button(keyword).clicked() {
                    keyword.clone_into(&mut draft.0);
                    changed = true;
                }
            }
            let except = ui.label("except");
            changed |= ui
                .add(egui::TextEdit::singleline(&mut draft.1).hint_text("e.g. portrait").desired_width(120.0))
                .labelled_by(except.id)
                .changed();
            if changed {
                apply_displays(banner, draft);
            }
        });
        ui.end_row();
        if let Err(e) = selector_from_text(&draft.0).and(selector_from_text(&draft.1)) {
            ui.label("");
            ui.colored_label(ui.visuals().error_fg_color, e);
            ui.end_row();
        }

        ui.label("Options");
        ui.horizontal(|ui| {
            ui.checkbox(&mut banner.dock, "Reserve screen space")
                .on_hover_text("Keep maximized windows and the taskbar clear (edge banners)");
            ui.checkbox(&mut banner.options.reverse, "Reverse flow");
            ui.label("Offset");
            ui.add(egui::DragValue::new(&mut banner.options.offset).range(0.0..=2000.0).suffix(" px"));
        });
        ui.end_row();
    });

    ui.label("Lines (top first)");
    let mut remove_line = None;
    let mut swap = None;
    let count = banner.lines.len();
    for (l, line) in banner.lines.iter_mut().enumerate() {
        ui.push_id(("line", index, l), |ui| {
            ui.horizontal(|ui| {
                let label = ui.label("Skin");
                egui::ComboBox::from_id_salt("skin")
                    .selected_text(&line.skin)
                    .show_ui(ui, |ui| {
                        for id in skins {
                            ui.selectable_value(&mut line.skin, id.clone(), id);
                        }
                    })
                    .response
                    .labelled_by(label.id);
                ui.add(
                    egui::DragValue::new(&mut line.speed)
                        .range(0.0..=2000.0)
                        .speed(2.0)
                        .suffix(" px/s"),
                );
                let label = ui.label("Shows");
                egui::ComboBox::from_id_salt("content")
                    .selected_text(line.content.name())
                    .show_ui(ui, |ui| {
                        for c in Content::ALL {
                            ui.selectable_value(&mut line.content, c, c.name());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                let label = ui.label("Quotes by");
                egui::ComboBox::from_id_salt("strategy")
                    .selected_text(line.strategy.name())
                    .show_ui(ui, |ui| {
                        for s in StrategyKind::ALL {
                            ui.selectable_value(&mut line.strategy, s, s.name());
                        }
                    })
                    .response
                    .labelled_by(label.id);
                if ui.add_enabled(l > 0, egui::Button::new("Up")).clicked() {
                    swap = Some((l, l - 1));
                }
                if ui
                    .add_enabled(l + 1 < count, egui::Button::new("Down"))
                    .clicked()
                {
                    swap = Some((l, l + 1));
                }
                if ui
                    .add_enabled(count > 1, egui::Button::new("Remove line"))
                    .clicked()
                {
                    remove_line = Some(l);
                }
            });
            ui.horizontal(|ui| {
                let label = ui.label("Messages");
                // Separated by "|"; kept exactly as typed so editing is smooth.
                let mut text = line.messages.join("|");
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut text)
                            .hint_text("e.g. Markets close at 4 PM | Welcome")
                            .desired_width(420.0),
                    )
                    .labelled_by(label.id)
                    .changed()
                {
                    line.messages = if text.trim().is_empty() {
                        Vec::new()
                    } else {
                        text.split('|').map(str::to_owned).collect()
                    };
                }
            });
        });
    }
    if let Some((a, b)) = swap {
        banner.lines.swap(a, b);
    }
    if let Some(l) = remove_line {
        banner.lines.remove(l);
    }
    ui.horizontal(|ui| {
        if ui.button("Add line").clicked() {
            let line = banner.lines.last().cloned().unwrap_or_else(default_line);
            banner.lines.push(line);
        }
        if ui.button("Remove banner").clicked() {
            remove = true;
        }
    });
    ui.add_space(6.0);
    remove
}

fn apply_displays(banner: &mut BannerConfig, draft: &(String, String)) {
    let Ok(include) = selector_from_text(&draft.0) else {
        return;
    };
    let selector = if draft.1.trim().is_empty() {
        include
    } else {
        match selector_from_text(&draft.1) {
            Ok(exclude) => DisplaySelector::Except(Box::new(include), Box::new(exclude)),
            Err(_) => return,
        }
    };
    set_displays(&mut banner.placement, selector);
}

fn feed_editor(ui: &mut egui::Ui, feed: &mut FeedConfig) {
    egui::Grid::new("feed-grid")
        .num_columns(2)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            if feed.provider != Provider::Mock {
                ui.label("API key");
                match &mut feed.key_source {
                    KeySource::Env(var) => {
                        ui.horizontal(|ui| {
                            let label = ui.label("from environment variable");
                            ui.text_edit_singleline(var).labelled_by(label.id);
                            let found =
                                std::env::var_os(var.as_str()).is_some_and(|v| !v.is_empty());
                            if found {
                                ui.label("found");
                            } else {
                                ui.colored_label(ui.visuals().warn_fg_color, "not set");
                            }
                        });
                    }
                    KeySource::Inline => {
                        ui.horizontal(|ui| {
                            ui.label("stored in the file (hidden)");
                            if ui.button("Use an environment variable instead").clicked() {
                                feed.key_source =
                                    KeySource::Env(feed.provider.default_key_env().to_owned());
                                feed.api_key.clear();
                            }
                        });
                    }
                }
                ui.end_row();
            }

            ui.label("Snapshot every");
            let mut secs = feed.refresh.as_secs_f64();
            if ui
                .add(
                    egui::DragValue::new(&mut secs)
                        .range(10.0..=86_400.0)
                        .suffix(" s"),
                )
                .changed()
            {
                feed.refresh = Duration::from_secs_f64(secs);
            }
            ui.end_row();

            match feed.provider {
                Provider::Finnhub => {
                    ui.label("Live trades");
                    ui.checkbox(&mut feed.stream, "Stream every trade (off: snapshots only)");
                    ui.end_row();
                }
                Provider::Massive => {
                    ui.label("Plan limit");
                    ui.add(
                        egui::DragValue::new(&mut feed.requests_per_minute)
                            .range(1..=100_000)
                            .suffix(" requests/min"),
                    );
                    ui.end_row();
                }
                Provider::Mock => {}
            }
        });
}

/// Opens the settings window for the configuration file at `path` and runs
/// until it is closed.
///
/// # Errors
///
/// Returns a message if the window can't be opened.
pub fn run(
    path: PathBuf,
    config: Config,
    library: SkinLibrary,
    preview: Option<PreviewFn>,
) -> Result<(), String> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("stocktape settings")
            .with_inner_size([1000.0, 720.0]),
        ..Default::default()
    };
    let app = SettingsApp::new(path, config, library, preview);
    eframe::run_native(
        "stocktape settings",
        options,
        Box::new(move |_| Ok(Box::new(Window(app)))),
    )
    .map_err(|e| e.to_string())
}

struct Window(SettingsApp);

impl eframe::App for Window {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.0.ui(ui);
    }
}

#[cfg(test)]
mod tests;
