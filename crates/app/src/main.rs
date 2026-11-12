//! The `stocktape` desktop application.

use std::path::PathBuf;
use std::process::ExitCode;

use stocktape_skin::{SkinLibrary, SkinSource};

mod ticker;
mod window;

use stocktape_config::{self as config, Config};

/// Command-line options.
#[derive(Debug, Default, PartialEq)]
struct Options {
    /// Skin id for every banner, overriding the configuration.
    skin: Option<String>,
    /// Banner configuration file.
    config: Option<PathBuf>,
    /// Built-in layout preset, used when there is no configuration file.
    layout: Option<String>,
    /// Folder with user skins (`*.toml`), watched for changes.
    skins_dir: Option<PathBuf>,
    /// Write a demo frame to this PNG instead of opening a window.
    preview: Option<PathBuf>,
    /// Width of the preview in pixels.
    width: Option<u32>,
}

/// What the command line asked for.
#[derive(Debug, PartialEq)]
enum Command {
    Run(Options),
    Preview(Options),
    ListSkins(Options),
    ListLayouts,
    ListDisplays,
    Settings(Options),
    Version,
    Help,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut options = Options::default();
    let mut list = false;
    let mut settings = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-V" | "--version" => return Ok(Command::Version),
            "-h" | "--help" => return Ok(Command::Help),
            "--list-skins" => list = true,
            "--list-layouts" => return Ok(Command::ListLayouts),
            "--list-displays" => return Ok(Command::ListDisplays),
            "--settings" => settings = true,
            "--skin" => options.skin = Some(value("--skin")?),
            "--config" => options.config = Some(PathBuf::from(value("--config")?)),
            "--layout" => options.layout = Some(value("--layout")?),
            "--skins-dir" => options.skins_dir = Some(PathBuf::from(value("--skins-dir")?)),
            "--preview" => options.preview = Some(PathBuf::from(value("--preview")?)),
            "--width" => {
                let v = value("--width")?;
                let width: u32 = v.parse().map_err(|_| format!("invalid --width {v:?}"))?;
                if !(16..=16_384).contains(&width) {
                    return Err(format!("--width must be between 16 and 16384, not {width}"));
                }
                options.width = Some(width);
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    // With --settings, --layout is the template for a new --config file.
    if !settings && options.config.is_some() && options.layout.is_some() {
        return Err("use either --config or --layout, not both".to_owned());
    }
    Ok(if settings {
        Command::Settings(options)
    } else if list {
        Command::ListSkins(options)
    } else if options.preview.is_some() {
        Command::Preview(options)
    } else {
        Command::Run(options)
    })
}

fn version_line() -> String {
    format!("stocktape {}", env!("CARGO_PKG_VERSION"))
}

const HELP: &str = "\
A broadcast-style scrolling stock ticker for the desktop.

Usage: stocktape [OPTIONS]

Without options, shows a demo banner with sample (not real) prices along the
bottom of the primary display. Press Esc while a banner has focus to quit.

Options:
      --config <FILE>     Banner configuration (see docs/configuration.md)
      --layout <NAME>     Built-in layout preset (see --list-layouts)
      --list-layouts      List layout presets and exit
      --list-displays     List displays (names, positions, orientation) and exit
      --settings          Open the settings window (edits the configuration file;
                          running banners pick up saved changes)
      --skin <NAME>       Skin for every banner (see --list-skins)
      --skins-dir <DIR>   Load extra skins from DIR (*.toml); edits reload live
      --list-skins        List available skins and exit
      --preview <FILE>    Write a demo frame to a PNG file instead of opening a window
      --width <PIXELS>    Width of the preview [default: 1280]
  -h, --help              Print help
  -V, --version           Print version";

/// Loads the skin library: built-ins plus `--skins-dir`.
fn library(options: &Options) -> Result<SkinLibrary, String> {
    let mut library = SkinLibrary::builtin();
    if let Some(dir) = &options.skins_dir {
        library
            .load_dir(dir)
            .map_err(|e| format!("can't read skins from {}: {e}", dir.display()))?;
    }
    Ok(library)
}

/// Loads the banner configuration: `--config`, else `--layout`, else the
/// `bottom` preset; `--skin` overrides every banner's skin.
/// The configuration file in use: `--config`, else the standard file if it
/// exists (unless `--layout` picks a preset).
fn config_path(options: &Options) -> Option<PathBuf> {
    match (&options.config, &options.layout) {
        (Some(path), _) => Some(path.clone()),
        (None, Some(_)) => None,
        (None, None) => config::default_path().filter(|p| p.exists()),
    }
}

fn read_config(path: &std::path::Path) -> Result<Config, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
    Config::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn config(options: &Options) -> Result<Config, String> {
    let mut config = match (config_path(options), &options.layout) {
        (Some(path), _) => read_config(&path)?,
        (None, Some(name)) => Config::preset(name)?,
        (None, None) => Config::preset("bottom")?,
    };
    if let Some(skin) = &options.skin {
        for line in config.banners.iter_mut().flat_map(|b| &mut b.lines) {
            line.skin.clone_from(skin);
        }
    }
    Ok(config)
}

/// Opens the settings window on `--config` or the standard file, creating it
/// from `--layout` (default `bottom`) when it doesn't exist yet.
fn settings(options: &Options) -> Result<(), String> {
    let path = options
        .config
        .clone()
        .or_else(config::default_path)
        .ok_or("can't find a configuration folder; use --config FILE")?;
    let config = if path.exists() {
        read_config(&path)?
    } else {
        Config::preset(options.layout.as_deref().unwrap_or("bottom"))?
    };
    let library = library(options)?;
    stocktape_ui::run(path, config, library, Some(Box::new(ticker::preview_frame)))
}

fn list_skins(options: &Options) -> Result<(), String> {
    let library = library(options)?;
    for id in library.ids() {
        let source = match library.source(id) {
            Some(SkinSource::File(path)) => path.display().to_string(),
            _ => "built-in".to_owned(),
        };
        match library.resolve(id) {
            Ok(skin) => println!("{id:<16} {:<16} {} [{source}]", skin.name, skin.description),
            Err(e) => println!("{id:<16} error: {e}"),
        }
    }
    Ok(())
}

fn preview(options: &Options) -> Result<(), String> {
    let library = library(options)?;
    let id = options.skin.as_deref().unwrap_or(SkinLibrary::DEFAULT);
    let skin = library.resolve(id).map_err(|e| e.to_string())?;
    let path = options.preview.as_ref().expect("preview path is set");
    let image = ticker::preview_frame(&skin, options.width.unwrap_or(1280));
    image
        .save_png(path)
        .map_err(|e| format!("can't write {}: {e}", path.display()))?;
    println!(
        "wrote {} ({}x{})",
        path.display(),
        image.width(),
        image.height()
    );
    Ok(())
}

fn main() -> ExitCode {
    let result = match parse_args(std::env::args().skip(1)) {
        Ok(Command::Version) => {
            println!("{}", version_line());
            Ok(())
        }
        Ok(Command::Help) => {
            println!("{HELP}");
            Ok(())
        }
        Ok(Command::ListSkins(options)) => list_skins(&options),
        Ok(Command::Settings(options)) => {
            stocktape_platform::release_own_console();
            settings(&options)
        }
        Ok(Command::ListDisplays) => window::list_displays().map_err(|e| e.to_string()),
        Ok(Command::ListLayouts) => {
            for (name, description) in config::presets() {
                println!("{name:<16} {description}");
            }
            Ok(())
        }
        Ok(Command::Preview(options)) => preview(&options),
        Ok(Command::Run(options)) => config(&options).and_then(|config| {
            // No console window behind the banners when not started from a
            // terminal (Windows).
            stocktape_platform::release_own_console();
            config.check_keys()?;
            let library = library(&options)?;
            // Edits to the configuration file (by hand or from the settings
            // window) apply live.
            let reload = config_path(&options).map(|p| (p, options.skin.clone()));
            window::run(config, library, options.skins_dir, reload).map_err(|e| e.to_string())
        }),
        Err(message) => {
            eprintln!("error: {message}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_arguments_runs() {
        assert_eq!(parse_args(args(&[])), Ok(Command::Run(Options::default())));
    }

    #[test]
    fn version_and_help_flags() {
        assert_eq!(parse_args(args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse_args(args(&["-V"])), Ok(Command::Version));
        assert_eq!(parse_args(args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse_args(args(&["-h"])), Ok(Command::Help));
    }

    #[test]
    fn skin_options() {
        let Ok(Command::Run(o)) = parse_args(args(&["--skin", "terminal", "--skins-dir", "s"]))
        else {
            panic!("expected run");
        };
        assert_eq!(o.skin.as_deref(), Some("terminal"));
        assert_eq!(o.skins_dir, Some(PathBuf::from("s")));
        assert!(matches!(
            parse_args(args(&["--list-skins"])),
            Ok(Command::ListSkins(_))
        ));
        let Ok(Command::Preview(o)) = parse_args(args(&["--preview", "a.png", "--width", "640"]))
        else {
            panic!("expected preview");
        };
        assert_eq!(o.width, Some(640));
    }

    #[test]
    fn bad_arguments_are_errors() {
        assert!(parse_args(args(&["--bogus"])).is_err());
        assert!(parse_args(args(&["--skin"])).is_err());
        assert!(parse_args(args(&["--preview", "a.png", "--width", "abc"])).is_err());
        assert!(parse_args(args(&["--preview", "a.png", "--width", "5"])).is_err());
    }

    #[test]
    fn version_line_names_the_binary() {
        assert!(version_line().starts_with("stocktape "));
    }

    #[test]
    fn preview_writes_a_png() {
        let path =
            std::env::temp_dir().join(format!("stocktape-preview-{}.png", std::process::id()));
        let options = Options {
            skin: Some("daylight".to_owned()),
            preview: Some(path.clone()),
            width: Some(320),
            ..Options::default()
        };
        preview(&options).unwrap();
        let image = tiny_skia::Pixmap::load_png(&path).unwrap();
        assert_eq!((image.width(), image.height()), (320, 52));
        std::fs::remove_file(path).unwrap();
    }
}
