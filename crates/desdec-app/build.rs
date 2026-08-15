//! Resolves the build identifier shown in the About dialog.
//!
//! The version itself comes from `CARGO_PKG_VERSION`, so `Cargo.toml` stays the
//! single source of truth. Only the build identifier needs resolving here.

use std::path::Path;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-env-changed=DESDEC_BUILD");

    // A release pipeline can pin its own identifier, such as a CI run number.
    let build = env_build()
        .or_else(git_build)
        .unwrap_or_else(|| "dev".to_owned());

    println!("cargo:rustc-env=DESDEC_BUILD={build}");
}

fn env_build() -> Option<String> {
    let value = std::env::var("DESDEC_BUILD").ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The short commit hash, with a `-dirty` suffix when the working tree has
/// uncommitted changes.
///
/// Tags are excluded on purpose. `describe` would otherwise report the nearest
/// one, and since tags here mirror the crate version, the About dialog would
/// read `v0.1.0 · v0.1.0-1-g7a3798a`. The build identifier answers a different
/// question than the version: which commit produced this binary.
fn git_build() -> Option<String> {
    watch_git_head();

    let output = Command::new("git")
        .args(["describe", "--always", "--dirty", "--exclude=*"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let described = String::from_utf8(output.stdout).ok()?;
    let trimmed = described.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// Rebuilds when the identifier could have changed.
///
/// Two things move it, and both must be watched:
///
/// - **The commit.** `HEAD` when a branch is checked out, `refs` and
///   `packed-refs` when a commit lands on the current one.
/// - **The working tree**, which decides the `-dirty` suffix. Without the
///   source directories here, the build script never reran on an edit, so a
///   build made from modified sources kept claiming a clean commit — the About
///   dialog said the binary was something it was not.
///
/// Paths are only declared when they exist: pointing `rerun-if-changed` at a
/// missing file would rebuild the crate on every single run once the sources
/// are unpacked outside a checkout.
fn watch_git_head() {
    for path in [
        "../../.git/HEAD",
        "../../.git/refs",
        "../../.git/packed-refs",
        "../../Cargo.toml",
        "../desdec-core/src",
        "../desdec-core/Cargo.toml",
        "src",
        "Cargo.toml",
    ] {
        if Path::new(path).exists() {
            println!("cargo:rerun-if-changed={path}");
        }
    }
}
