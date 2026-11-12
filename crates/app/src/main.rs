//! The `stocktape` desktop application.

use std::process::ExitCode;

mod ticker;
mod window;

/// What the command line asked for.
#[derive(Debug, PartialEq, Eq)]
enum Command {
    Run,
    Version,
    Help,
}

fn parse_args<I: IntoIterator<Item = String>>(args: I) -> Result<Command, String> {
    let mut command = Command::Run;
    for arg in args {
        match arg.as_str() {
            "-V" | "--version" => command = Command::Version,
            "-h" | "--help" => command = Command::Help,
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    Ok(command)
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
  -h, --help     Print help
  -V, --version  Print version";

fn main() -> ExitCode {
    match parse_args(std::env::args().skip(1)) {
        Ok(Command::Version) => println!("{}", version_line()),
        Ok(Command::Help) => println!("{HELP}"),
        Ok(Command::Run) => {
            if let Err(e) = window::run() {
                eprintln!("error: {e}");
                return ExitCode::FAILURE;
            }
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{HELP}");
            return ExitCode::from(2);
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn no_arguments_runs() {
        assert_eq!(parse_args(args(&[])), Ok(Command::Run));
    }

    #[test]
    fn version_and_help_flags() {
        assert_eq!(parse_args(args(&["--version"])), Ok(Command::Version));
        assert_eq!(parse_args(args(&["-V"])), Ok(Command::Version));
        assert_eq!(parse_args(args(&["--help"])), Ok(Command::Help));
        assert_eq!(parse_args(args(&["-h"])), Ok(Command::Help));
    }

    #[test]
    fn unknown_argument_is_an_error() {
        assert!(parse_args(args(&["--bogus"])).is_err());
    }

    #[test]
    fn version_line_names_the_binary() {
        assert!(version_line().starts_with("stocktape "));
    }
}
