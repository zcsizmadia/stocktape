//! The `stocktape` desktop application.

use std::path::PathBuf;
use std::process::ExitCode;

use stocktape_skin::{SkinLibrary, SkinSource};

mod ticker;
mod window;

/// Command-line options.
#[derive(Debug, Default, PartialEq)]
struct Options {
    /// Skin id; the default skin when `None`.
    skin: Option<String>,
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
    Version,
    Help,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut options = Options::default();
    let mut list = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or_else(|| format!("{name} needs a value"));
        match arg.as_str() {
            "-V" | "--version" => return Ok(Command::Version),
            "-h" | "--help" => return Ok(Command::Help),
            "--list-skins" => list = true,
            "--skin" => options.skin = Some(value("--skin")?),
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
    Ok(if list {
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
bottom of the primary display. Press Esc while it has focus to quit.

Options:
      --skin <NAME>       Skin to use (see --list-skins)
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
        Ok(Command::Preview(options)) => preview(&options),
        Ok(Command::Run(options)) => library(&options).and_then(|library| {
            window::run(library, options.skin, options.skins_dir).map_err(|e| e.to_string())
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
