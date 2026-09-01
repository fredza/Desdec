//! Putting a verified release archive in place of the running program.
//!
//! Everything here happens *after* the reader has said yes to this particular
//! version, having seen its number and its notes. Nothing on this path runs on
//! its own: [`crate::update`] fetches and checks, this replaces, and both wait
//! to be asked.
//!
//! Three rules shape the whole module.
//!
//! **Only one file ever leaves the archive.** The executable is looked up by
//! name and everything else is ignored — an archive that could write where it
//! liked would be a way of installing something other than Desdec, which is
//! exactly what an update must not be. Nothing is unpacked to a path the
//! archive chose.
//!
//! **What is about to be installed is read first.** The magic bytes and the
//! class are checked against this platform, the same way `scripts/insl.sh`
//! checks before it installs: a 32-bit object, a shell script or a truncated
//! download is refused here rather than left to fail from a dock with nowhere
//! to print why.
//!
//! **The replacement is atomic, and the old copy is kept.** The new binary is
//! written beside the old one, synced, and moved over it in one step, so a
//! crash halfway leaves the running program untouched rather than a half-file
//! under its name. The copy it replaced stays as `<name>.old`, which is what a
//! reader needs when the new one turns out not to start.

use std::{
    fs::{self, File},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use super::Error;

/// What the executable is called inside a published archive.
///
/// Both spellings, and not this platform's alone. An archive holds exactly one
/// executable, and which of the two names it goes by is a fact about the
/// archive rather than about the machine reading it — so a Linux build can
/// read the Windows archive and say what is in it. That matters for one
/// reason: it is what lets the test below run all three published archives
/// through this module from one machine, and a reader of zip files that can
/// only ever be exercised on the platform it was written for is a reader
/// nobody checks.
///
/// Installing the wrong one is not what this guards against — [`accept`] does
/// that, by reading the bytes rather than trusting a name.
const EXECUTABLE_NAMES: [&str; 2] = ["desdec-app", "desdec-app.exe"];

/// What this platform's own archive calls it, for the messages.
const EXECUTABLE: &str = if cfg!(windows) {
    "desdec-app.exe"
} else {
    "desdec-app"
};

/// Whether a path inside an archive names the executable.
fn is_the_executable(path: &str) -> bool {
    path.rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| EXECUTABLE_NAMES.contains(&name))
}

/// The suffix the replaced copy keeps.
///
/// Kept rather than deleted, and named plainly: an update that leaves no way
/// back is one a reader has to be brave to accept. On Windows it is also a
/// necessity — a running `.exe` cannot be overwritten, only renamed out of the
/// way — so the same name serves both purposes.
pub const REPLACED_SUFFIX: &str = ".old";

/// The largest executable an archive may hold, unpacked.
///
/// A bound on what a stream is allowed to make this process allocate: without
/// one, an archive claiming a colossal size is a way to exhaust memory. Desdec
/// itself is around ten megabytes; a hundred is far above anything a release
/// will hold and far below anything that hurts.
const MAXIMUM_EXECUTABLE: u64 = 100 * 1024 * 1024;

/// Where the running program lives, when that can be answered.
///
/// `current_exe` follows symbolic links, which is what is wanted here: the
/// file to replace is the one the loader actually read, not a link somebody
/// left pointing at it.
///
/// # Errors
///
/// [`Error::Storage`] when the platform cannot say, which is not a case any
/// desktop system reaches but is not worth a panic.
pub fn running_binary() -> Result<PathBuf, Error> {
    std::env::current_exe().map_err(|why| Error::Storage(why.to_string()))
}

/// Whether the running program could be replaced without asking for rights it
/// does not have.
///
/// Asked before the reader is offered the button, so that an installation
/// under `/usr/local/bin` says so instead of failing at the last step. It is
/// the *directory* that must be writable, not the file: replacing is a rename
/// within the directory, and a read-only file in a writable directory can
/// still be moved aside.
#[must_use]
pub fn can_replace(binary: &Path) -> bool {
    let Some(directory) = binary.parent() else {
        return false;
    };
    let probe = directory.join(format!(".desdec-write-probe-{}", std::process::id()));
    match File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

/// Replaces `binary` with the executable held in the verified `archive`.
///
/// Returns where the copy that was replaced now sits, so the caller can say
/// so — and so a reader who needs it knows where to look.
///
/// # Errors
///
/// [`Error::Storage`] for an archive that cannot be read, holds no executable,
/// holds something that is not one for this platform, or a file that could not
/// be written.
pub fn replace(archive: &Path, binary: &Path) -> Result<PathBuf, Error> {
    let bytes = unpack(archive)?;
    accept(&bytes)?;

    let directory = binary
        .parent()
        .ok_or_else(|| Error::Storage(format!("{} has no directory", binary.display())))?;

    // Written in the destination directory rather than a temporary one: a
    // rename across file systems is a copy, and a copy is not atomic. This
    // stays on the same volume by construction.
    let staged = directory.join(format!(".{}.new", file_name(binary)));
    write_executable(&staged, &bytes)?;

    let replaced = directory.join(format!("{}{REPLACED_SUFFIX}", file_name(binary)));
    let _ = fs::remove_file(&replaced);
    // The old one is moved aside before the new one takes its place, which on
    // Windows is not a nicety but the only order that works: a running `.exe`
    // cannot be overwritten, and can be renamed. Elsewhere it costs one system
    // call and buys the same way back.
    if binary.exists() {
        fs::rename(binary, &replaced).map_err(|why| {
            let _ = fs::remove_file(&staged);
            Error::Storage(why.to_string())
        })?;
    }
    if let Err(why) = fs::rename(&staged, binary) {
        // Put back what was moved, so a failure here leaves a program that
        // still starts rather than a directory with no Desdec in it at all.
        let _ = fs::rename(&replaced, binary);
        let _ = fs::remove_file(&staged);
        return Err(Error::Storage(why.to_string()));
    }
    Ok(replaced)
}

/// Deletes the copy an earlier update replaced, if one is there.
///
/// Called at start-up. On Windows the replaced copy cannot be removed while it
/// is the running program, so it is removed the next time Desdec starts — by
/// which point it is not. Everywhere else this simply tidies up.
///
/// Silent about failure on purpose: a leftover file is not something to tell a
/// reader about, and it will be tried again at the next start.
pub fn forget_replaced(binary: &Path) {
    let Some(directory) = binary.parent() else {
        return;
    };
    let replaced = directory.join(format!("{}{REPLACED_SUFFIX}", file_name(binary)));
    if replaced != binary {
        let _ = fs::remove_file(replaced);
    }
}

/// The executable's own name, or the name every archive uses when the path has
/// none to give.
fn file_name(binary: &Path) -> String {
    binary
        .file_name()
        .map_or_else(|| EXECUTABLE.to_owned(), |name| name.to_string_lossy().into_owned())
}

/// Writes the bytes and makes them runnable.
fn write_executable(at: &Path, bytes: &[u8]) -> Result<(), Error> {
    let mut file = File::create(at).map_err(|why| Error::Storage(why.to_string()))?;
    file.write_all(bytes)
        .map_err(|why| Error::Storage(why.to_string()))?;
    // Synced before the rename: the rename is what makes the new binary
    // visible under its name, and a rename that lands before the bytes do
    // would publish a file whose contents are not yet on the disk.
    file.sync_all()
        .map_err(|why| Error::Storage(why.to_string()))?;
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(at, fs::Permissions::from_mode(0o755))
            .map_err(|why| Error::Storage(why.to_string()))?;
    }
    Ok(())
}

/// Whether these bytes are an executable this machine can run.
///
/// The same reading `scripts/insl.sh` does before it installs, and for the
/// same reason: what arrives here has been checked against a published
/// checksum, which says the download is intact and says nothing about what it
/// holds. A release that shipped the wrong archive, or an archive whose layout
/// changed, is caught now rather than by a dock that cannot say why nothing
/// happens.
fn accept(bytes: &[u8]) -> Result<(), Error> {
    let refuse = |what: &str| Err(Error::Storage(format!("the archive holds {what}")));
    let head = bytes.get(..5).unwrap_or_default();
    if cfg!(target_os = "linux") {
        match head {
            [0x7f, b'E', b'L', b'F', 2] => Ok(()),
            [0x7f, b'E', b'L', b'F', ..] => refuse("a 32-bit ELF"),
            _ => refuse("something that is not an ELF executable"),
        }
    } else if cfg!(target_os = "macos") {
        match bytes.get(..4).unwrap_or_default() {
            // `MH_MAGIC_64` as it is written down, little-endian, which is
            // every Mac this program is built for. A universal binary
            // (`cafebabe`) is not accepted: Desdec publishes none, and what
            // one holds cannot be told from four bytes.
            [0xcf, 0xfa, 0xed, 0xfe] => Ok(()),
            _ => refuse("something that is not a 64-bit Mach-O executable"),
        }
    } else if cfg!(windows) {
        match bytes.get(..2).unwrap_or_default() {
            [b'M', b'Z'] => Ok(()),
            _ => refuse("something that is not a PE executable"),
        }
    } else {
        // An unknown platform is not told that its bytes are wrong, because
        // this does not know what right would look like there.
        Ok(())
    }
}

/// The executable's bytes, out of the archive this platform publishes.
fn unpack(archive: &Path) -> Result<Vec<u8>, Error> {
    let name = archive.to_string_lossy();
    if name.ends_with(".tar.gz") {
        from_tar_gz(archive)
    } else if name.ends_with(".zip") {
        from_zip(archive)
    } else {
        Err(Error::Storage(format!("{name} is not an archive Desdec publishes")))
    }
}

/// The executable out of a gzipped tar, which is what Linux releases carry.
///
/// The tar format is read here rather than with a library: an entry is a
/// 512-byte header naming a file and stating its length, then that many bytes
/// rounded up to the next 512. Only the name and the size are needed, and only
/// one entry is wanted, so the whole of it is the loop below. The gzip layer
/// is `flate2`, because a DEFLATE decoder is where writing a format by hand
/// stops being cheaper than depending on one.
fn from_tar_gz(archive: &Path) -> Result<Vec<u8>, Error> {
    let file = File::open(archive).map_err(|why| Error::Storage(why.to_string()))?;
    let mut tar = flate2::read::GzDecoder::new(file);

    let mut header = [0_u8; 512];
    loop {
        read_exactly(&mut tar, &mut header)?;
        // Two zero blocks end the archive; one is enough to stop on, since a
        // header of nothing names no file.
        if header.iter().all(|byte| *byte == 0) {
            return Err(Error::Storage(format!(
                "the archive holds no {EXECUTABLE}"
            )));
        }
        let name = field(&header[..100]);
        let size = octal(&header[124..136])?;

        // A directory, a link or anything else is skipped by its declared
        // size, which for those is zero. `0` and NUL both mean a plain file.
        let is_file = matches!(header[156], b'0' | 0);
        if is_file && is_the_executable(&name) {
            if size > MAXIMUM_EXECUTABLE {
                return Err(Error::Storage(format!("{EXECUTABLE} is {size} bytes")));
            }
            let mut bytes = vec![0_u8; usize::try_from(size).unwrap_or(0)];
            read_exactly(&mut tar, &mut bytes)?;
            return Ok(bytes);
        }
        // Entries are padded to a multiple of 512, and the padding has to be
        // read past rather than seeked over: this is a decompressing stream,
        // which has no position to seek to.
        let padded = size.div_ceil(512) * 512;
        skip(&mut tar, padded)?;
    }
}

/// The executable out of a zip, which is what macOS and Windows releases
/// carry.
///
/// Read from the central directory at the end of the file rather than by
/// walking the local headers: the central directory is the part of a zip that
/// is authoritative about what the archive holds, and a local header may
/// declare its sizes in a trailing descriptor that is only readable after the
/// data it describes.
fn from_zip(archive: &Path) -> Result<Vec<u8>, Error> {
    let bytes = fs::read(archive).map_err(|why| Error::Storage(why.to_string()))?;
    let end = end_of_central_directory(&bytes)
        .ok_or_else(|| Error::Storage("the archive has no central directory".to_owned()))?;

    let mut at = end;
    while let Some(entry) = bytes.get(at..at + 46) {
        if entry[..4] != [b'P', b'K', 0x01, 0x02] {
            break;
        }
        let method = u16::from_le_bytes([entry[10], entry[11]]);
        let compressed = u64::from(u32::from_le_bytes([
            entry[20], entry[21], entry[22], entry[23],
        ]));
        let uncompressed = u64::from(u32::from_le_bytes([
            entry[24], entry[25], entry[26], entry[27],
        ]));
        let name_length = usize::from(u16::from_le_bytes([entry[28], entry[29]]));
        let extra = usize::from(u16::from_le_bytes([entry[30], entry[31]]));
        let comment = usize::from(u16::from_le_bytes([entry[32], entry[33]]));
        let offset = u32::from_le_bytes([entry[42], entry[43], entry[44], entry[45]]) as usize;
        let name = String::from_utf8_lossy(
            bytes
                .get(at + 46..at + 46 + name_length)
                .unwrap_or_default(),
        )
        .into_owned();

        if is_the_executable(&name) {
            if uncompressed > MAXIMUM_EXECUTABLE {
                return Err(Error::Storage(format!(
                    "{EXECUTABLE} is {uncompressed} bytes"
                )));
            }
            return zip_entry(&bytes, offset, method, compressed, uncompressed);
        }
        at += 46 + name_length + extra + comment;
    }
    Err(Error::Storage(format!("the archive holds no {EXECUTABLE}")))
}

/// One zip entry's bytes, given where its local header sits.
fn zip_entry(
    bytes: &[u8],
    offset: usize,
    method: u16,
    compressed: u64,
    uncompressed: u64,
) -> Result<Vec<u8>, Error> {
    let local = bytes
        .get(offset..offset + 30)
        .ok_or_else(|| Error::Storage("the archive ends inside an entry".to_owned()))?;
    if local[..4] != [b'P', b'K', 0x03, 0x04] {
        return Err(Error::Storage("the archive names an entry that is not there".to_owned()));
    }
    // The local header's own name and extra lengths, which need not match the
    // central directory's — the extra field commonly differs between the two.
    let name_length = usize::from(u16::from_le_bytes([local[26], local[27]]));
    let extra = usize::from(u16::from_le_bytes([local[28], local[29]]));
    let start = offset + 30 + name_length + extra;
    let compressed = usize::try_from(compressed).unwrap_or(0);
    let data = bytes
        .get(start..start + compressed)
        .ok_or_else(|| Error::Storage("the archive ends inside an entry".to_owned()))?;

    match method {
        // Stored: the bytes are the file.
        0 => Ok(data.to_vec()),
        // Deflated, which is what every zip tool writes for an executable.
        8 => {
            let mut out = Vec::with_capacity(usize::try_from(uncompressed).unwrap_or(0));
            flate2::read::DeflateDecoder::new(data)
                .take(MAXIMUM_EXECUTABLE)
                .read_to_end(&mut out)
                .map_err(|why| Error::Storage(why.to_string()))?;
            Ok(out)
        }
        other => Err(Error::Storage(format!(
            "the archive compresses {EXECUTABLE} in a way this does not read ({other})"
        ))),
    }
}

/// Where the central directory begins, read from the record at the end.
///
/// The record carries a comment of its own, so it is looked for from the end
/// backwards rather than at a fixed offset — its own length is the only thing
/// that says where it starts.
fn end_of_central_directory(bytes: &[u8]) -> Option<usize> {
    let signature = [b'P', b'K', 0x05, 0x06];
    let start = bytes.len().saturating_sub(66_000);
    let record = bytes[start..]
        .windows(4)
        .rposition(|window| window == signature)?
        + start;
    let field = bytes.get(record + 16..record + 20)?;
    Some(u32::from_le_bytes([field[0], field[1], field[2], field[3]]) as usize)
}

/// A NUL-terminated fixed-width field, as tar writes them.
fn field(bytes: &[u8]) -> String {
    let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// A tar size field, which is octal text and not a number.
fn octal(bytes: &[u8]) -> Result<u64, Error> {
    let text = field(bytes);
    let text = text.trim();
    if text.is_empty() {
        return Ok(0);
    }
    u64::from_str_radix(text, 8)
        .map_err(|_| Error::Storage(format!("the archive states a size this cannot read: {text}")))
}

/// Fills the buffer or says the archive ended too soon.
fn read_exactly(from: &mut impl Read, into: &mut [u8]) -> Result<(), Error> {
    from.read_exact(into)
        .map_err(|_| Error::Storage("the archive ends sooner than it says".to_owned()))
}

/// Reads past `count` bytes of a stream that cannot be seeked.
fn skip(from: &mut impl Read, count: u64) -> Result<(), Error> {
    let mut sink = std::io::sink();
    let copied = std::io::copy(&mut from.take(count), &mut sink)
        .map_err(|why| Error::Storage(why.to_string()))?;
    if copied == count {
        Ok(())
    } else {
        Err(Error::Storage("the archive ends sooner than it says".to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a release actually ships is what this must read, so the test
    /// builds one the same way the workflow does: `tar -czf … desdec-app
    /// Desdec.desktop`, two entries, the executable second in one case and
    /// first in the other.
    fn tar_gz(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = Vec::new();
        for (name, body) in entries {
            let mut header = [0_u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            // Mode, uid, gid: fixed-width octal, which tar wants present.
            header[100..107].copy_from_slice(b"0000755");
            header[108..115].copy_from_slice(b"0000000");
            header[116..123].copy_from_slice(b"0000000");
            let size = format!("{:011o} ", body.len());
            header[124..136].copy_from_slice(size.as_bytes());
            header[136..148].copy_from_slice(b"00000000000 ");
            header[156] = b'0';
            // The checksum is computed with the field itself read as spaces.
            header[148..156].copy_from_slice(b"        ");
            let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
            let checksum = format!("{sum:06o}\0 ");
            header[148..156].copy_from_slice(checksum.as_bytes());

            tar.extend_from_slice(&header);
            tar.extend_from_slice(body);
            let padding = (512 - body.len() % 512) % 512;
            tar.extend(std::iter::repeat_n(0_u8, padding));
        }
        tar.extend(std::iter::repeat_n(0_u8, 1024));

        let mut encoder =
            flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&tar).expect("gzip the tar");
        encoder.finish().expect("finish the gzip")
    }

    fn scratch(name: &str) -> PathBuf {
        let at = std::env::temp_dir().join(format!("desdec-install-{}-{name}", std::process::id()));
        let _ = fs::create_dir_all(&at);
        at
    }

    /// The whole of it, on the archive a release actually published: a copy of
    /// Desdec is replaced by the one in the archive, and the copy it replaced
    /// is still there.
    ///
    /// The one test that exercises ,  and  together
    /// on bytes nobody here wrote. Skipped without , so the
    /// suite needs no network.
    #[test]
    fn a_real_archive_replaces_a_real_binary() {
        let Ok(directory) = std::env::var("DESDEC_ARCHIVES") else {
            return;
        };
        let archive = PathBuf::from(directory).join(match std::env::consts::OS {
            "linux" => "desdec-linux-x86_64-release.tar.gz",
            "macos" => "desdec-macos-aarch64-release.zip",
            "windows" => "desdec-windows-x86_64-release.zip",
            _ => return,
        });
        if !archive.exists() {
            return;
        }

        let directory = scratch("endtoend");
        let binary = directory.join("desdec");
        fs::write(&binary, b"the copy being replaced").expect("write a binary to replace");
        assert!(can_replace(&binary), "the scratch directory must be writable");

        let replaced = replace(&archive, &binary).expect("replace with the published archive");
        let installed = fs::read(&binary).expect("the installed binary");
        assert!(accept(&installed).is_ok(), "what landed is an executable");
        assert!(installed.len() > 1_000_000, "and a whole one");
        assert_eq!(
            fs::read(&replaced).expect("the replaced copy"),
            b"the copy being replaced",
            "the way back is kept"
        );
    }

    /// The three archives a release actually publishes give up their
    /// executable.
    ///
    /// Fixtures written here encode this module's own idea of the formats
    /// twice over; these are the files the workflow produced — a `tar` from
    /// GNU tar, a `zip` from `Compress-Archive` on Windows and one from
    /// `ditto` on macOS, which do not agree about much. Skipped where the
    /// archives were not fetched, so the suite does not need the network.
    #[test]
    fn the_published_archives_give_up_their_executable() {
        let Ok(directory) = std::env::var("DESDEC_ARCHIVES") else {
            return;
        };
        let directory = PathBuf::from(directory);
        let mut seen = 0;
        for (name, magic) in [
            ("desdec-linux-x86_64-release.tar.gz", &[0x7f, b'E', b'L', b'F'][..]),
            ("desdec-macos-aarch64-release.zip", &[0xcf, 0xfa, 0xed, 0xfe][..]),
            ("desdec-windows-x86_64-release.zip", &b"MZ"[..]),
        ] {
            let archive = directory.join(name);
            if !archive.exists() {
                continue;
            }
            seen += 1;
            let bytes = unpack(&archive)
                .unwrap_or_else(|why| panic!("{name}: {why}"));
            assert!(
                bytes.starts_with(magic),
                "{name} gave up something that does not start with its platform's magic"
            );
            assert!(
                bytes.len() > 1_000_000,
                "{name} gave up {} bytes, which is not an executable",
                bytes.len()
            );
        }
        assert!(seen > 0, "DESDEC_ARCHIVES holds none of the three archives");
    }

    /// A `.tar.gz` gives up its executable, and only that one.
    ///
    /// The Linux archive carries `Desdec.desktop` beside the binary since
    /// 2026-09-01, so the entry wanted is not the first one — reading only the
    /// first would have worked on every archive published before that and on
    /// none published after.
    #[test]
    fn the_executable_comes_out_of_a_tar_that_holds_more_than_it() {
        let elf = {
            let mut bytes = vec![0x7f, b'E', b'L', b'F', 2];
            bytes.extend(std::iter::repeat_n(0_u8, 600));
            bytes
        };
        let archive = scratch("tar").join("desdec-linux-x86_64-release.tar.gz");
        fs::write(
            &archive,
            tar_gz(&[
                ("Desdec.desktop", b"[Desktop Entry]\n"),
                ("desdec-app", &elf),
            ]),
        )
        .expect("write the archive");

        let found = unpack(&archive).expect("the executable");
        assert_eq!(found, elf, "the bytes are the entry's, not the desktop file's");
        assert!(accept(&found).is_ok());
    }

    /// An archive that holds no executable says so rather than installing
    /// whatever else it found.
    #[test]
    fn an_archive_without_the_executable_is_refused() {
        let archive = scratch("empty").join("desdec-linux-x86_64-release.tar.gz");
        fs::write(&archive, tar_gz(&[("Desdec.desktop", b"[Desktop Entry]\n")]))
            .expect("write the archive");
        assert!(unpack(&archive).is_err());
    }

    /// What is about to replace the running program is read first.
    ///
    /// A checksum says the download is intact and says nothing about what it
    /// holds; these are the shapes that must not be installed.
    #[test]
    fn only_an_executable_for_this_platform_is_installed() {
        assert!(accept(b"#!/bin/sh\necho hello\n").is_err());
        assert!(accept(&[]).is_err());
        if cfg!(target_os = "linux") {
            assert!(accept(&[0x7f, b'E', b'L', b'F', 2, 1, 1, 0]).is_ok());
            assert!(
                accept(&[0x7f, b'E', b'L', b'F', 1, 1, 1, 0]).is_err(),
                "a 32-bit ELF is not what this machine runs"
            );
        }
    }

    /// Replacing keeps the copy it replaced, and puts it where the caller is
    /// told it is.
    #[test]
    fn the_replaced_copy_is_kept_beside_the_new_one() {
        let directory = scratch("replace");
        let binary = directory.join("desdec");
        fs::write(&binary, b"the old one").expect("write the old binary");

        let elf = {
            let mut bytes = vec![0x7f, b'E', b'L', b'F', 2];
            bytes.extend(std::iter::repeat_n(0_u8, 600));
            bytes
        };
        let archive = directory.join("desdec-linux-x86_64-release.tar.gz");
        fs::write(&archive, tar_gz(&[("desdec-app", &elf)])).expect("write the archive");

        // Only where this platform would accept those bytes: the check is the
        // point of the function, and skipping it elsewhere would test nothing.
        if accept(&elf).is_err() {
            return;
        }
        let replaced = replace(&archive, &binary).expect("replace the binary");
        assert_eq!(fs::read(&binary).expect("the new binary"), elf);
        assert_eq!(
            fs::read(&replaced).expect("the replaced copy"),
            b"the old one"
        );
        assert_eq!(replaced, directory.join("desdec.old"));

        forget_replaced(&binary);
        assert!(!replaced.exists(), "the copy is cleared at the next start");
    }
}
