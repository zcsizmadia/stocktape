//! UI tests: the real widgets, driven through egui's accessibility tree.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use super::*;

fn temp_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("stocktape-ui-{}-{name}.toml", std::process::id()))
}

fn app(name: &str, config: Config) -> SettingsApp {
    SettingsApp::new(temp_path(name), config, SkinLibrary::builtin(), None)
}

fn harness(app: SettingsApp) -> Harness<'static, SettingsApp> {
    let mut h = Harness::builder()
        .with_size(egui::vec2(1000.0, 900.0))
        .build_ui_state(|ui, app: &mut SettingsApp| app.ui(ui), app);
    h.run();
    h
}

#[test]
fn opens_clean_with_save_disabled() {
    let path = temp_path("clean");
    let config = Config::preset("bottom").unwrap();
    std::fs::write(&path, config.to_toml()).unwrap();
    let h = harness(SettingsApp::new(
        path.clone(),
        config,
        SkinLibrary::builtin(),
        None,
    ));
    let _ = std::fs::remove_file(&path);
    assert!(!h.state().is_dirty());
    let save = h.get_by_label("Save");
    assert!(save.accesskit_node().is_disabled());
}

#[test]
fn adding_a_banner_and_a_line() {
    let mut h = harness(app("add", Config::preset("bottom").unwrap()));
    h.get_by_label("Add banner").click();
    h.run();
    assert_eq!(h.state().config().banners.len(), 2);
    assert!(h.state().is_dirty());
    // Each banner has its own "Add line" button; use the last one.
    h.get_all_by_label("Add line").last().unwrap().click();
    h.run();
    assert_eq!(h.state().config().banners[1].lines.len(), 2);
}

#[test]
fn presets_replace_the_banners() {
    let mut h = harness(app("preset", Config::preset("bottom").unwrap()));
    h.get_by_label("Start from a preset:").click();
    h.run();
    h.get_by_label("two-tier").click();
    h.run();
    assert_eq!(h.state().config().banners[0].lines.len(), 2);
}

#[test]
fn saving_writes_a_file_that_reads_back() {
    let path = temp_path("save");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("toml.bak"));
    let mut h = harness(SettingsApp::new(
        path.clone(),
        Config::preset("top-bottom").unwrap(),
        SkinLibrary::builtin(),
        None,
    ));
    h.get_by_label("Add banner").click();
    h.run();
    h.get_by_label("Save").click();
    h.run();
    assert!(!h.state().is_dirty());
    let saved = Config::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(&saved, h.state().config());
    // A second save keeps the first file as a backup.
    h.get_all_by_label("Remove banner").last().unwrap().click();
    h.run();
    h.get_by_label("Save").click();
    h.run();
    assert!(path.with_extension("toml.bak").exists());
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("toml.bak"));
}

#[test]
fn feeds_never_show_keys() {
    let mut config = Config::preset("bottom").unwrap();
    config.feeds.push(FeedConfig {
        provider: Provider::Finnhub,
        key_source: KeySource::Inline,
        api_key: "super-secret-key".to_owned(),
        names: Vec::new(),
        refresh: Duration::from_secs(300),
        requests_per_minute: 60,
        stream: true,
    });
    let mut h = harness(app("keys", config));
    h.get_by_label("Feeds").click();
    h.run();
    assert!(h.query_by_label_contains("super-secret-key").is_none());
    assert!(
        h.query_by_label_contains("stored in the file (hidden)")
            .is_some()
    );
    h.get_by_label("Use an environment variable instead")
        .click();
    h.run();
    let feed = &h.state().config().feeds[0];
    assert_eq!(
        feed.key_source,
        KeySource::Env("FINNHUB_API_KEY".to_owned())
    );
    assert!(feed.api_key.is_empty());
}

#[test]
fn watchlist_toggle_and_symbol_parsing() {
    let mut h = harness(app("watch", Config::preset("bottom").unwrap()));
    h.get_by_label("Watchlist").click();
    h.run();
    h.get_by_label("Use my own watchlist (otherwise the demo list)")
        .click();
    h.run();
    let groups = h.state().config().watchlist.clone().unwrap();
    assert_eq!(groups[0].symbols.len(), 2);
    assert_eq!(parse_symbols("AAPL, msft\nXLON:VOD").unwrap().len(), 3);
    assert!(parse_symbols("AAPL, A B$").is_err());
}

#[test]
fn display_text_updates_the_placement() {
    let mut banner = Config::preset("bottom").unwrap().banners.remove(0);
    let draft = ("all".to_owned(), "portrait".to_owned());
    apply_displays(&mut banner, &draft);
    assert_eq!(
        displays_of(&banner.placement),
        &DisplaySelector::Except(
            Box::new(DisplaySelector::All),
            Box::new(DisplaySelector::Portrait)
        )
    );
    // Invalid text leaves the placement alone.
    apply_displays(&mut banner, &("a,,b".to_owned(), String::new()));
    assert!(matches!(
        displays_of(&banner.placement),
        DisplaySelector::Except(..)
    ));
    assert_eq!(
        with_place(&banner.placement, "around"),
        Placement::Around {
            displays: displays_of(&banner.placement).clone()
        }
    );
}

#[test]
fn a_new_file_starts_unsaved() {
    let path = temp_path("new");
    let _ = std::fs::remove_file(&path);
    let h = harness(SettingsApp::new(
        path,
        Config::preset("bottom").unwrap(),
        SkinLibrary::builtin(),
        None,
    ));
    assert!(h.state().is_dirty());
    assert!(!h.get_by_label("Save").accesskit_node().is_disabled());
}

#[test]
fn skins_tab_shows_a_preview() {
    let calls = std::rc::Rc::new(std::cell::Cell::new(0));
    let counter = calls.clone();
    let preview: PreviewFn = Box::new(move |skin, width| {
        counter.set(counter.get() + 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let height = skin.height.ceil() as u32;
        let mut p = Pixmap::new(width, height).unwrap();
        p.fill(tiny_skia::Color::from_rgba8(14, 26, 47, 255));
        p
    });
    let mut a = app("skins", Config::preset("bottom").unwrap());
    a.preview = Some(preview);
    a.tab = Tab::Skins;
    let mut h = harness(a);
    assert!(
        h.query_by_label_contains("Two-tier broadcast look")
            .is_some()
    );
    h.get_by_label("glass").click();
    h.run();
    h.run();
    assert!(h.query_by_label_contains("Translucent overlay").is_some());
    // One render per skin viewed; repaints reuse the texture.
    assert_eq!(calls.get(), 2);
}
