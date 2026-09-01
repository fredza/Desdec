//! What the command line asks for, read before a window is opened.
//!
//! Until now the first argument was taken as a path whatever it said, so
//! `desdec --version` opened the window and reported that a file called
//! `--version` could not be read. That is the wrong answer to a question
//! every installed program is asked — not least by the script that installed
//! it, which has no other way to check what it just put in place.
//!
//! **On Windows this speaks into the void in a release build.** The binary is
//! linked as a windows subsystem application there, so it has no console to
//! print to; see the note at the top of `main.rs`. Attaching the parent
//! console needs `unsafe`, which this workspace forbids, and a message box
//! would be a worse answer than none for something a script pipes. A debug
//! build keeps its console and prints normally.

use std::ffi::OsString;
use std::path::PathBuf;

/// What the arguments amount to.
#[derive(Debug, Eq, PartialEq)]
pub enum Request {
    /// Open the window, on this file if one was named.
    Window(Option<PathBuf>),
    /// Write this to standard output and stop, successfully.
    Say(String),
    /// Write this to standard error and stop, unsuccessfully.
    Refuse(String),
    /// Write the application's own icon here, as a PNG, and stop.
    ///
    /// For the installer, which puts the icon where a desktop looks for it.
    /// Asking the program that was just installed for its mark is the only
    /// way the menu entry cannot come to show an older one than the window.
    WriteIcon(PathBuf),
}

/// The version, as the About dialog spells it, with the program's name ahead
/// of it — which is what a `--version` line is expected to carry.
fn version() -> String {
    format!(
        "desdec v{} · {}",
        env!("CARGO_PKG_VERSION"),
        env!("DESDEC_BUILD")
    )
}

/// Written in English, alone among the strings this program shows.
///
/// Everything else is translated, and the language is the reader's own choice
/// — kept in the preferences, which live in a store `eframe` has not opened
/// yet at the moment this is printed. Guessing a language here to answer
/// `--help` would guess wrong for anyone whose preference is not the default.
const USAGE: &str = "\
Desdec — a binary explorer.

Usage: desdec [options] [file]

  file             Analyse this file as soon as the window opens.

  -h, --help       Show this message and exit.
  -V, --version    Show the version and exit.
  --write-icon <file>
                   Write the application's icon to <file> and exit.
                   The format follows the extension: .ico for Windows,
                   .icns for a macOS bundle, a PNG for anything else.
                   The installers use this to put the icon beside the
                   desktop entry, the shortcut or the bundle.
  --               Read everything after this as a path, even if it
                   begins with a dash.

With no file, the window opens empty and offers to open one.";

/// Reads the arguments, which are the ones after the program's own name.
///
/// An unknown option is refused rather than taken for a filename. A mistyped
/// `--verison` would otherwise open a window reporting that a file of that
/// name does not exist, which tells the reader nothing about what they got
/// wrong.
pub fn read<I>(arguments: I) -> Request
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let Some(argument) = arguments.next() else {
        return Request::Window(None);
    };
    // Only an argument that is valid text can be an option. A path need not
    // be — a filename on Linux is bytes — so anything else is one.
    let Some(text) = argument.to_str() else {
        return Request::Window(Some(PathBuf::from(argument)));
    };
    match text {
        "-h" | "--help" => Request::Say(USAGE.to_owned()),
        "-V" | "--version" => Request::Say(version()),
        "--write-icon" => arguments.next().map_or_else(
            || Request::Refuse(format!("desdec: --write-icon needs a file\n\n{USAGE}")),
            |file| Request::WriteIcon(PathBuf::from(file)),
        ),
        "--" => Request::Window(arguments.next().map(PathBuf::from)),
        // A lone `-` is a filename by every convention there is, and never
        // an option.
        _ if text.starts_with('-') && text != "-" => {
            Request::Refuse(format!("desdec: unknown option: {text}\n\n{USAGE}"))
        }
        _ => Request::Window(Some(PathBuf::from(argument))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_args(arguments: &[&str]) -> Request {
        read(arguments.iter().map(OsString::from))
    }

    fn window(path: &str) -> Request {
        Request::Window(Some(PathBuf::from(path)))
    }

    #[test]
    fn no_arguments_open_an_empty_window() {
        assert_eq!(read_args(&[]), Request::Window(None));
    }

    #[test]
    fn a_path_is_the_file_to_analyse() {
        assert_eq!(read_args(&["/bin/ls"]), window("/bin/ls"));
    }

    #[test]
    fn the_version_names_the_crate_it_was_built_from() {
        let Request::Say(said) = read_args(&["--version"]) else {
            panic!("--version should say something and stop");
        };
        assert!(
            said.contains(env!("CARGO_PKG_VERSION")),
            "the version line does not carry the crate version: {said}"
        );
        assert_eq!(read_args(&["-V"]), Request::Say(said));
    }

    #[test]
    fn help_says_what_the_arguments_are() {
        let Request::Say(said) = read_args(&["--help"]) else {
            panic!("--help should say something and stop");
        };
        // The list of options is the whole point of the message: a `--help`
        // that forgets to mention one of them is worse than none.
        for option in ["--help", "--version", "--"] {
            assert!(said.contains(option), "{option} is missing from --help");
        }
        assert_eq!(read_args(&["-h"]), Request::Say(said));
    }

    /// The icon goes where it is told, and nowhere by default: a `--write-icon`
    /// with nothing after it must not write to a file called nothing.
    #[test]
    fn the_icon_is_written_where_the_argument_says() {
        assert_eq!(
            read_args(&["--write-icon", "/tmp/desdec.png"]),
            Request::WriteIcon(PathBuf::from("/tmp/desdec.png"))
        );
        let Request::Refuse(refused) = read_args(&["--write-icon"]) else {
            panic!("--write-icon with no file should be refused");
        };
        assert!(refused.contains("--write-icon"), "{refused}");
    }

    /// The reason this module exists: neither of these used to be an answer.
    #[test]
    fn an_option_is_never_taken_for_a_filename() {
        assert!(matches!(read_args(&["--version"]), Request::Say(_)));
        assert!(matches!(read_args(&["--verison"]), Request::Refuse(_)));
    }

    #[test]
    fn a_refusal_repeats_the_usage_so_the_reader_can_fix_it() {
        let Request::Refuse(refused) = read_args(&["--nope"]) else {
            panic!("an unknown option should be refused");
        };
        assert!(refused.contains("--nope"), "{refused}");
        assert!(refused.contains("Usage:"), "{refused}");
    }

    #[test]
    fn a_path_that_begins_with_a_dash_is_reachable_after_a_double_dash() {
        assert_eq!(read_args(&["--", "-weird-name"]), window("-weird-name"));
        // And `--` on its own is not a request to open anything.
        assert_eq!(read_args(&["--"]), Request::Window(None));
    }

    /// `-` names a file in every shell that has ever run, and standard input
    /// is not something this program reads: it is a path like any other.
    #[test]
    fn a_lone_dash_is_a_filename() {
        assert_eq!(read_args(&["-"]), window("-"));
    }

    /// A filename need not be valid UTF-8 on Linux, and one that is not must
    /// still open.
    #[test]
    fn a_path_that_is_not_text_is_still_a_path() {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStringExt as _;
            let raw = OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]);
            assert_eq!(
                read(vec![raw.clone()]),
                Request::Window(Some(PathBuf::from(raw)))
            );
        }
    }
}
