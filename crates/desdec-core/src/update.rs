//! Asking GitHub whether there is a newer release, and fetching it intact.
//!
//! Two things this deliberately does not do. It never replaces the running
//! program: the archive lands in a directory the reader chose, and installing
//! it is a thing they do, knowing they are doing it. And it never reaches the
//! network unasked — the caller decides when to look, because a tool that
//! quietly tells a server it was started is a tool that reports on its reader.
//!
//! What it does do is finish the job a download starts. A release publishes a
//! `.sha256` beside each archive; this reads it, hashes what arrived, and
//! refuses anything that does not match — deleting it rather than leaving a
//! half-trusted file on disk under a name that looks right. That answers
//! whether the bytes are intact, and nothing more. It does not answer **who**
//! made them: releases are not signed from v0.4.1 on, and the window says so
//! rather than letting a matching checksum be read as more than it is.

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use crate::analysis::hash;

/// Where releases are published. Not configurable: an update that could be
/// pointed at another repository is a way of installing something else.
const REPOSITORY: &str = "fredza/Desdec";

/// How long any one request may take. Long enough for a slow line, short
/// enough that a wedged connection does not hold a window open all evening.
const CHECK_TIMEOUT: Duration = Duration::from_secs(20);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// The largest archive that will be accepted, as a guard against a redirect
/// pointing somewhere unexpected. The published archives are under ten
/// megabytes; a hundred leaves room to grow without leaving room for a disk.
const MAXIMUM_ARCHIVE: u64 = 100 * 1024 * 1024;

/// What the check could not do.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The request did not get through: no network, a proxy, a timeout.
    Unreachable(String),
    /// GitHub answered something other than a release.
    Unreadable(String),
    /// There is no archive for this operating system and processor in the
    /// release. The reader is told which platform was looked for.
    NoArchiveForThisPlatform { platform: String },
    /// The download does not hash to what the release says it should. The
    /// file is deleted before this is returned.
    ChecksumMismatch { expected: String, found: String },
    /// The release publishes no checksum for its archive, so nothing can be
    /// checked and nothing is kept.
    NoChecksum,
    /// The file could not be written.
    Storage(String),
    /// The archive is larger than [`MAXIMUM_ARCHIVE`].
    TooLarge { size: u64 },
}

impl std::fmt::Display for Error {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unreachable(why) | Self::Unreadable(why) | Self::Storage(why) => {
                write!(out, "{why}")
            }
            Self::NoArchiveForThisPlatform { platform } => write!(out, "{platform}"),
            Self::ChecksumMismatch { expected, found } => write!(out, "{expected} ≠ {found}"),
            Self::NoChecksum => write!(out, "no checksum"),
            Self::TooLarge { size } => write!(out, "{size}"),
        }
    }
}

/// A released version, as three numbers.
///
/// Compared as numbers rather than as text, so `0.10.0` is newer than `0.9.0`.
/// Anything a tag carries beyond the three numbers — a `-rc1`, a `+build` — is
/// kept for display and ignored for ordering: a pre-release is not an update
/// to offer, and [`Release::is_newer_than`] says so.
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    /// Reads `1.2.3`, with or without a leading `v`.
    ///
    /// Returns `None` for anything else, including a tag carrying a
    /// pre-release suffix: those are not offered as updates.
    #[must_use]
    pub fn parse(tag: &str) -> Option<Self> {
        let tag = tag.trim().trim_start_matches('v');
        if tag.contains(['-', '+']) {
            return None;
        }
        let mut parts = tag.split('.');
        let mut number = || parts.next()?.parse::<u32>().ok();
        let version = Self {
            major: number()?,
            minor: number()?,
            patch: number()?,
        };
        parts.next().is_none().then_some(version)
    }

    /// The version this build of Desdec is.
    #[must_use]
    pub fn running() -> Option<Self> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(out, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// One file attached to a release.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published release, and the archive of it that fits this machine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    pub version: Version,
    /// The tag as published, which may say more than the three numbers.
    pub tag: String,
    /// What the release notes say, as GitHub holds them.
    pub notes: String,
    /// The page a reader is sent to when they would rather do it themselves.
    pub page: String,
    /// When it was published, as GitHub writes it.
    pub published: String,
    /// The archive for this operating system and processor.
    pub archive: Asset,
    /// The file naming the archive's SHA-256, when the release publishes one.
    pub checksum: Option<Asset>,
}

impl Release {
    /// Whether this release is worth telling the reader about.
    #[must_use]
    pub fn is_newer_than(&self, running: Version) -> bool {
        self.version > running
    }
}

/// The name the archive for this machine carries, as the workflow builds it.
///
/// Returns `None` on a platform no archive is published for, which is a fact
/// worth reporting rather than a reason to show nothing.
#[must_use]
pub fn platform_archive() -> Option<&'static str> {
    Some(match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "desdec-linux-x86_64-release.tar.gz",
        ("macos", "aarch64") => "desdec-macos-aarch64-release.zip",
        ("windows", "x86_64") => "desdec-windows-x86_64-release.zip",
        _ => return None,
    })
}

/// How this machine is named when there is no archive for it.
#[must_use]
pub fn platform_label() -> String {
    format!("{} {}", std::env::consts::OS, std::env::consts::ARCH)
}

/// Asks GitHub for the newest release.
///
/// # Errors
///
/// [`Error::Unreachable`] when the request does not get through,
/// [`Error::Unreadable`] when the answer is not a release, and
/// [`Error::NoArchiveForThisPlatform`] when the release publishes nothing this
/// machine could run.
pub fn latest() -> Result<Release, Error> {
    let endpoint = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let body = fetch_text(&endpoint)?;
    let value: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| Error::Unreadable(error.to_string()))?;
    read_release(&value)
}

/// Turns GitHub's answer into a [`Release`].
fn read_release(value: &serde_json::Value) -> Result<Release, Error> {
    let tag = value
        .get("tag_name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::Unreadable(String::from("tag_name")))?
        .to_owned();
    let version =
        Version::parse(&tag).ok_or_else(|| Error::Unreadable(format!("tag_name: {tag}")))?;
    let wanted = platform_archive().ok_or_else(|| Error::NoArchiveForThisPlatform {
        platform: platform_label(),
    })?;

    let mut archive = None;
    let mut checksum = None;
    if let Some(assets) = value.get("assets").and_then(serde_json::Value::as_array) {
        for asset in assets {
            let Some(found) = read_asset(asset) else {
                continue;
            };
            if found.name == wanted {
                archive = Some(found);
            } else if found.name == format!("{wanted}.sha256") {
                checksum = Some(found);
            }
        }
    }
    let archive = archive.ok_or_else(|| Error::NoArchiveForThisPlatform {
        platform: platform_label(),
    })?;

    Ok(Release {
        version,
        tag,
        notes: string_at(value, "body"),
        page: string_at(value, "html_url"),
        published: string_at(value, "published_at"),
        archive,
        checksum,
    })
}

fn read_asset(value: &serde_json::Value) -> Option<Asset> {
    Some(Asset {
        name: value.get("name")?.as_str()?.to_owned(),
        url: value.get("browser_download_url")?.as_str()?.to_owned(),
        size: value.get("size").and_then(serde_json::Value::as_u64)?,
    })
}

fn string_at(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// How far a download has got, reported as it goes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Progress {
    pub received: u64,
    /// What the release said the archive weighs, which is what the bar is of.
    pub total: u64,
}

/// Downloads a release's archive into `directory` and checks it.
///
/// The file is written under a temporary name and only takes the archive's own
/// name once its hash matches: a reader who finds the file has a file that was
/// checked, whatever happened while it was arriving. A mismatch deletes what
/// arrived — a wrong archive under the right name is worse than no archive.
///
/// # Errors
///
/// [`Error::NoChecksum`] when the release publishes none, so nothing could be
/// checked; [`Error::ChecksumMismatch`] when what arrived is not what was
/// published; and [`Error::Unreachable`] or [`Error::Storage`] for a download
/// that did not finish or a file that could not be written.
pub fn download(
    release: &Release,
    directory: &Path,
    mut progress: impl FnMut(Progress),
) -> Result<PathBuf, Error> {
    let Some(checksum) = release.checksum.as_ref() else {
        return Err(Error::NoChecksum);
    };
    if release.archive.size > MAXIMUM_ARCHIVE {
        return Err(Error::TooLarge {
            size: release.archive.size,
        });
    }
    let expected = read_checksum(&fetch_text(&checksum.url)?, &release.archive.name)
        .ok_or(Error::NoChecksum)?;

    let bytes = fetch_bytes(&release.archive.url, release.archive.size, &mut progress)?;
    let found = hash::to_hex(&hash::sha256(&bytes));
    if !found.eq_ignore_ascii_case(&expected) {
        return Err(Error::ChecksumMismatch { expected, found });
    }

    keep_verified_archive(directory, &release.archive.name, &bytes, &expected)
}

/// Keeps verified bytes under their release name without overwriting anything.
///
/// A download directory need not exist yet: it is common for a portable copy
/// of Desdec to be pointed at a new folder. The temporary file is synced and
/// then hard-linked into place. Linking, unlike a Unix rename, never replaces
/// a file that appeared meanwhile; an existing matching archive is simply
/// reused. Thus clicking Download twice cannot replace an archive the reader
/// already has.
fn keep_verified_archive(
    directory: &Path,
    name: &str,
    bytes: &[u8],
    expected: &str,
) -> Result<PathBuf, Error> {
    fs::create_dir_all(directory).map_err(|error| Error::Storage(error.to_string()))?;
    let destination = directory.join(name);
    if destination.exists()
        && let Ok(path) = existing_verified_archive(&destination, expected)
    {
        return Ok(path);
    }

    // create_new makes a stale partial file harmless instead of overwriting
    // it. The process id separates concurrent copies of Desdec; the attempt
    // number also copes with a previous interrupted download from this copy.
    let partial = (0..128)
        .map(|attempt| directory.join(format!(".{name}.{}.{}.part", std::process::id(), attempt)))
        .find(|path| !path.exists())
        .ok_or_else(|| Error::Storage(String::from("too many unfinished update downloads")))?;

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)
            .map_err(|error| Error::Storage(error.to_string()))?;
        file.write_all(bytes)
            .map_err(|error| Error::Storage(error.to_string()))?;
        file.sync_all()
            .map_err(|error| Error::Storage(error.to_string()))?;

        // This is an atomic "create destination if absent" on the same
        // filesystem. A rename would overwrite an existing file on Unix.
        match fs::hard_link(&partial, &destination) {
            Ok(()) => {
                fs::remove_file(&partial).map_err(|error| Error::Storage(error.to_string()))?;
                Ok(destination)
            }
            Err(_) if destination.exists() => {
                match existing_verified_archive(&destination, expected) {
                    Ok(path) => Ok(path),
                    Err(_) => beside(directory, name, &partial, expected),
                }
            }
            Err(error) => Err(Error::Storage(error.to_string())),
        }
    })();
    if result.is_err() {
        let _ = fs::remove_file(&partial);
    }
    result
}

/// Puts the verified bytes beside a file of the same name that is not them.
///
/// A reader's download folder keeps what they downloaded before, and an
/// archive of a *previous* release carries exactly the name this one does. The
/// old behaviour refused at that point: the archive had been fetched and its
/// hash checked, and it was then thrown away with an error — so every update
/// from that moment on failed, permanently, until someone found and deleted a
/// file nobody had told them about.
///
/// Overwriting is still out of the question; a file the reader has is theirs.
/// So the new one is kept under the next free name, which is what a browser
/// does and what the reader will recognise in the folder — the journal names
/// the path it actually landed at.
fn beside(directory: &Path, name: &str, partial: &Path, expected: &str) -> Result<PathBuf, Error> {
    // Before the first dot, so `desdec-linux-x86_64.tar.gz` becomes
    // `desdec-linux-x86_64 (2).tar.gz` and not `…tar (2).gz`.
    let (stem, extension) = name.split_once('.').unwrap_or((name, ""));
    for attempt in 2..64_u32 {
        let candidate = directory.join(if extension.is_empty() {
            format!("{stem} ({attempt})")
        } else {
            format!("{stem} ({attempt}).{extension}")
        });
        // A name already taken may well be this very archive, from a download
        // the reader asked for twice. Reused rather than copied again: the
        // folder must not fill with `(2)`, `(3)`, `(4)` of one release.
        if candidate.exists() {
            if let Ok(path) = existing_verified_archive(&candidate, expected) {
                let _ = fs::remove_file(partial);
                return Ok(path);
            }
            continue;
        }
        if fs::hard_link(partial, &candidate).is_ok() {
            fs::remove_file(partial).map_err(|error| Error::Storage(error.to_string()))?;
            return Ok(candidate);
        }
    }
    Err(Error::Storage(format!(
        "{} is taken, and so is every name beside it",
        directory.join(name).display()
    )))
}

/// Returns an already-present archive only when it is exactly the one the
/// release named. A different local file is left untouched.
fn existing_verified_archive(destination: &Path, expected: &str) -> Result<PathBuf, Error> {
    let size = fs::metadata(destination)
        .map_err(|error| Error::Storage(error.to_string()))?
        .len();
    if size > MAXIMUM_ARCHIVE {
        return Err(Error::TooLarge { size });
    }
    let bytes = fs::read(destination).map_err(|error| Error::Storage(error.to_string()))?;
    let found = hash::to_hex(&hash::sha256(&bytes));
    if found.eq_ignore_ascii_case(expected) {
        Ok(destination.to_owned())
    } else {
        Err(Error::Storage(format!(
            "{} already exists and is not the published archive",
            destination.display()
        )))
    }
}

/// The hash for `archive_name` out of a `sha256sum` line.
///
/// The file name matters: a release can carry one checksum per platform, and
/// accepting the first hash in a combined file could verify Linux bytes using
/// the macOS line by mistake.
#[must_use]
pub fn read_checksum(contents: &str, archive_name: &str) -> Option<String> {
    let (word, file_name) = contents.lines().find_map(|line| {
        let mut fields = line.split_whitespace();
        let hash = fields.next()?;
        let file_name = fields.next()?.trim_start_matches('*');
        (fields.next().is_none() && file_name == archive_name).then_some((hash, file_name))
    })?;
    let hexadecimal = word.len() == 64 && word.chars().all(|c| c.is_ascii_hexdigit());
    (file_name == archive_name && hexadecimal).then(|| word.to_ascii_lowercase())
}

/// A `GET` whose body is text.
fn fetch_text(url: &str) -> Result<String, Error> {
    let mut response = request(url, CHECK_TIMEOUT)?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|error| Error::Unreachable(error.to_string()))
}

/// A `GET` whose body is an archive, reported on as it arrives.
fn fetch_bytes(
    url: &str,
    expected: u64,
    progress: &mut impl FnMut(Progress),
) -> Result<Vec<u8>, Error> {
    let mut response = request(url, DOWNLOAD_TIMEOUT)?;
    let mut reader = response.body_mut().as_reader();
    let mut bytes = Vec::with_capacity(usize::try_from(expected).unwrap_or_default());
    // On the heap: sixty-four kilobytes is a good read size and a bad stack
    // frame, and this runs on a thread whose stack the caller did not size.
    let mut window = vec![0_u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut window)
            .map_err(|error| Error::Unreachable(error.to_string()))?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&window[..read]);
        if bytes.len() as u64 > MAXIMUM_ARCHIVE {
            return Err(Error::TooLarge {
                size: bytes.len() as u64,
            });
        }
        progress(Progress {
            received: bytes.len() as u64,
            total: expected,
        });
    }
    Ok(bytes)
}

fn request(url: &str, timeout: Duration) -> Result<ureq::http::Response<ureq::Body>, Error> {
    ureq::get(url)
        // GitHub refuses a request with no user agent, and a request should
        // say what it is rather than pretend to be a browser.
        .header("user-agent", concat!("Desdec/", env!("CARGO_PKG_VERSION")))
        .header("accept", "application/vnd.github+json")
        .config()
        .timeout_global(Some(timeout))
        .build()
        .call()
        .map_err(|error| Error::Unreachable(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_compared_as_numbers_and_not_as_text() {
        let ten = Version::parse("v0.10.0").expect("parses");
        let nine = Version::parse("0.9.0").expect("parses");
        assert!(ten > nine, "0.10.0 is newer than 0.9.0");
    }

    #[test]
    fn a_pre_release_tag_is_not_a_version_to_offer() {
        assert_eq!(Version::parse("v1.0.0-rc1"), None);
        assert_eq!(Version::parse("v1.0.0+build7"), None);
        assert_eq!(Version::parse("v1.0"), None, "three numbers, not two");
        assert_eq!(Version::parse("v1.0.0.1"), None, "three, not four");
    }

    #[test]
    fn the_running_version_is_the_one_cargo_was_told() {
        let running = Version::running().expect("the crate version parses");
        assert_eq!(running.to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_checksum_file_is_read_as_the_hash_it_leads_with() {
        let line =
            "b6b9fc880f039d205a2995ae88ed07d88ead6ddd85f1574f32f7704bf8d04b0a  desdec.tar.gz\n";
        assert_eq!(
            read_checksum(line, "desdec.tar.gz").as_deref(),
            Some("b6b9fc880f039d205a2995ae88ed07d88ead6ddd85f1574f32f7704bf8d04b0a")
        );
    }

    #[test]
    fn anything_that_is_not_a_hash_is_refused() {
        assert_eq!(read_checksum("", "desdec.tar.gz"), None);
        assert_eq!(
            read_checksum("not a hash  desdec.tar.gz", "desdec.tar.gz"),
            None
        );
        // The right length, the wrong alphabet.
        assert_eq!(read_checksum(&"z".repeat(64), "desdec.tar.gz"), None);
    }

    #[test]
    fn a_checksum_for_another_archive_is_refused() {
        let line = format!("{}  desdec-macos.zip\n", "a".repeat(64));
        assert_eq!(read_checksum(&line, "desdec-linux.tar.gz"), None);
    }

    #[test]
    fn a_verified_archive_creates_its_destination_and_can_be_reused() {
        let directory = std::env::temp_dir().join(format!(
            "desdec-update-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let bytes = b"a small verified archive";
        let expected = hash::to_hex(&hash::sha256(bytes));

        let first = keep_verified_archive(&directory, "desdec.tar.gz", bytes, &expected)
            .expect("the new directory and archive are created");
        let again = keep_verified_archive(&directory, "desdec.tar.gz", bytes, &expected)
            .expect("the already verified archive is reused");
        assert_eq!(first, again);
        assert_eq!(fs::read(first).expect("archive is readable"), bytes);

        fs::remove_dir_all(directory).expect("test directory is removed");
    }

    /// The one that made every update fail, for good.
    ///
    /// A reader's download folder keeps what they downloaded before, and the
    /// archive of a previous release carries exactly the name this one does.
    /// The archive was fetched, its hash checked — and then thrown away with
    /// an error, so the next attempt failed the same way, and the one after
    /// that, until someone found and deleted a file nobody had named.
    ///
    /// Found on a real folder: `desdec-linux-x86_64-release.tar.gz` of
    /// 2026-08-21 sitting in front of the release published since.
    #[test]
    fn an_older_archive_of_the_same_name_is_kept_and_the_new_one_lands_beside_it() {
        let directory = std::env::temp_dir().join(format!(
            "desdec-update-beside-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("a folder to download into");

        // What the reader already has, under the name the new one wants.
        let theirs = directory.join("desdec-linux-x86_64-release.tar.gz");
        fs::write(&theirs, b"last month's release").expect("their own file");

        let bytes = b"the release published since";
        let expected = hash::to_hex(&hash::sha256(bytes));
        let kept = keep_verified_archive(
            &directory,
            "desdec-linux-x86_64-release.tar.gz",
            bytes,
            &expected,
        )
        .expect("a download that was fetched and verified is not thrown away");

        assert_eq!(
            fs::read(&theirs).expect("their file is readable"),
            b"last month's release",
            "a file the reader has is theirs, and is not overwritten"
        );
        assert_eq!(
            fs::read(&kept).expect("the new archive is readable"),
            bytes,
            "and the new one is on disk, whole"
        );
        // Before the first dot: `… (2).tar.gz`, not `….tar (2).gz`.
        assert_eq!(
            kept.file_name().and_then(|name| name.to_str()),
            Some("desdec-linux-x86_64-release (2).tar.gz"),
            "under a name the reader will recognise in the folder"
        );

        // And the same download again reuses what it just wrote rather than
        // making a third copy.
        let again = keep_verified_archive(
            &directory,
            "desdec-linux-x86_64-release.tar.gz",
            bytes,
            &expected,
        )
        .expect("the archive is there already");
        assert_eq!(again, kept);

        fs::remove_dir_all(directory).expect("test directory is removed");
    }

    /// GitHub's answer, cut down to the fields this reads.
    fn answer(tag: &str, archive: &str) -> serde_json::Value {
        serde_json::json!({
            "tag_name": tag,
            "body": "What changed.",
            "html_url": "https://example.invalid/releases/tag/v9.9.9",
            "published_at": "2026-08-20T00:00:00Z",
            "assets": [
                {
                    "name": archive,
                    "browser_download_url": "https://example.invalid/archive",
                    "size": 1234,
                },
                {
                    "name": format!("{archive}.sha256"),
                    "browser_download_url": "https://example.invalid/archive.sha256",
                    "size": 101,
                },
                {
                    "name": format!("{archive}.asc"),
                    "browser_download_url": "https://example.invalid/archive.asc",
                    "size": 228,
                },
            ],
        })
    }

    #[test]
    fn a_release_is_read_with_the_archive_that_fits_this_machine() {
        let Some(archive) = platform_archive() else {
            return; // No archive is published for this platform.
        };
        let release = read_release(&answer("v9.9.9", archive)).expect("reads");
        assert_eq!(release.version.to_string(), "9.9.9");
        assert_eq!(release.archive.name, archive);
        assert_eq!(release.archive.size, 1234);
        assert_eq!(
            release.checksum.as_ref().map(|asset| asset.name.as_str()),
            Some(format!("{archive}.sha256").as_str()),
            "the checksum beside it is found too"
        );
        assert!(release.is_newer_than(Version::running().expect("a version")));
    }

    #[test]
    fn a_release_carrying_nothing_for_this_machine_says_so() {
        let error = read_release(&answer("v9.9.9", "desdec-solaris-sparc.tar.gz"))
            .expect_err("no archive for this platform");
        assert!(matches!(error, Error::NoArchiveForThisPlatform { .. }));
    }

    #[test]
    fn a_release_whose_tag_is_not_a_version_is_not_offered() {
        let archive = platform_archive().unwrap_or("desdec-linux-x86_64-release.tar.gz");
        let error = read_release(&answer("nightly", archive)).expect_err("not a version");
        assert!(matches!(error, Error::Unreadable(_)));
    }

    #[test]
    fn the_running_release_is_not_newer_than_itself() {
        let Some(archive) = platform_archive() else {
            return;
        };
        let running = Version::running().expect("a version");
        let release = read_release(&answer(&running.to_string(), archive)).expect("reads");
        assert!(!release.is_newer_than(running), "same version, no offer");
    }
}
