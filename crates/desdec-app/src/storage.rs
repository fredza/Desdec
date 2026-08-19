//! Where Desdec keeps what belongs to the reader.
//!
//! Two things are written outside the application now — the notes taken on a
//! binary, and the plugins installed to work on one — and both are the
//! reader's own. They go under the platform's directory for user *data*, not
//! under a cache: a cache is a place the system is entitled to empty, and
//! neither an afternoon of annotations nor an installed plugin is something
//! anyone should lose to a disk cleanup.
//!
//! Nothing here is written to on its own. Each caller creates its own
//! subdirectory when it first has something to put in it, so a Desdec that was
//! opened once and never written in leaves nothing behind.

use std::path::PathBuf;

/// The `desdec` directory under the platform's user data location.
///
/// `None` when the environment says nothing about where that is, which is
/// answered by doing without rather than by guessing at a path: a notes file
/// written somewhere unexpected is worse than a notes file not written.
#[must_use]
pub fn data_directory() -> Option<PathBuf> {
    let base = if cfg!(target_os = "windows") {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
            })
    }?;
    Some(base.join("desdec"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_directory_is_named_after_the_application() {
        // The environment of the machine running the tests decides whether
        // there is one at all; what it must never be is a directory belonging
        // to something else.
        if let Some(directory) = data_directory() {
            assert_eq!(directory.file_name().unwrap_or_default(), "desdec");
        }
    }
}
