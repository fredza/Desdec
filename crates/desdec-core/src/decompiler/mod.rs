//! Optional decompilers installed on the machine.
//!
//! Desdec always has its own deterministic pseudo-code; these engines are for
//! when a real decompiler is wanted. Three properties are deliberate:
//!
//! - **Opt in.** Nothing is run unless the user chose an engine. A binary
//!   under analysis is potentially hostile, so no external tool is invoked
//!   behind the user's back.
//! - **No shell.** Arguments are passed as a list, never through `sh`, so a
//!   file name can carry any character without becoming a command.
//! - **Bounded.** Every run has a deadline and is killed past it: `aaa` on a
//!   large binary can take minutes, and the interface must not wait forever.
//!
//! The engines analyse the file statically. None of them executes it.

pub mod cache;
pub mod native;

use std::{
    env,
    ffi::OsStr,
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime},
};

/// Longest a decompiler may run before it is killed.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(90);

/// How often a running engine is checked for completion.
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// Checking whether an engine is usable must stay quick: it happens while the
/// preferences window is open, not as part of a decompilation.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// An external decompiler Desdec knows how to drive.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Engine {
    /// `rizin` with the `rz-ghidra` plugin, which adds the `pdg` command.
    RzGhidra,
    /// Avast's standalone `retdec-decompiler`.
    RetDec,
}

impl Engine {
    pub const ALL: &[Self] = &[Self::RzGhidra, Self::RetDec];

    /// Program looked up on `PATH` when no explicit path is configured.
    #[must_use]
    pub const fn program(self) -> &'static str {
        match self {
            Self::RzGhidra => "rizin",
            Self::RetDec => "retdec-decompiler",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::RzGhidra => "rizin + rz-ghidra",
            Self::RetDec => "RetDec",
        }
    }

    /// Where to get the engine, shown when it is missing. Deliberately a plain
    /// command rather than a link: it is what the user has to run.
    #[must_use]
    pub const fn install_hint(self) -> &'static str {
        match self {
            Self::RzGhidra => "rz-pm -i rz-ghidra",
            Self::RetDec => "github.com/avast/retdec/releases",
        }
    }
}

/// What was found for one engine.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Availability {
    /// The program was found, at this path.
    Found(PathBuf),
    /// Nothing to run: neither the configured path nor `PATH` has it.
    Missing,
    /// The program is there, but a piece it needs is not — `rizin` without the
    /// `rz-ghidra` plugin decompiles nothing, and saying "found" would promise
    /// an engine that cannot answer.
    Incomplete { path: PathBuf, reason: String },
}

impl Availability {
    #[must_use]
    pub const fn is_usable(&self) -> bool {
        matches!(self, Self::Found(_))
    }

    #[must_use]
    pub const fn path(&self) -> Option<&PathBuf> {
        match self {
            Self::Found(path) | Self::Incomplete { path, .. } => Some(path),
            Self::Missing => None,
        }
    }
}

/// Locates an engine, preferring an explicitly configured path.
///
/// A configured path that does not exist is reported as missing rather than
/// silently falling back to `PATH`: the user asked for that one.
#[must_use]
pub fn locate(engine: Engine, configured: Option<&Path>) -> Availability {
    let Some(program) = configured
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| find_on_path(engine.program()))
    else {
        return Availability::Missing;
    };
    if !program.is_file() {
        return Availability::Missing;
    }
    match engine {
        Engine::RzGhidra => rz_ghidra_readiness(&program),
        Engine::RetDec => Availability::Found(program),
    }
}

/// `rizin` alone cannot decompile: without the `rz-ghidra` plugin, `pdg` is
/// not a command at all.
///
/// The probe asks `rizin` for the help of `pdg`. A command that exists prints
/// its usage on standard output; a missing one prints nothing there and an
/// error on standard error — and the exit status is 0 either way, so the
/// output is what has to be read.
///
/// `malloc://` gives rizin a throwaway in-memory file, so nothing on disk is
/// opened and no analysed binary is involved in the check.
fn rz_ghidra_readiness(program: &Path) -> Availability {
    let probe = run_bounded(
        program,
        ["-N", "-q", "-c", "pdg?", "malloc://64"],
        PROBE_TIMEOUT,
    );
    match probe {
        Ok(run) if run.stdout.to_lowercase().contains("pdg") => {
            Availability::Found(program.to_owned())
        }
        Ok(_) => Availability::Incomplete {
            path: program.to_owned(),
            reason: "pdg".to_owned(),
        },
        Err(error) => Availability::Incomplete {
            path: program.to_owned(),
            reason: error.to_string(),
        },
    }
}

/// Walks `PATH` for an executable, the way a shell would.
fn find_on_path(program: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
    })
}

/// Runs `engine` over `binary` and returns the decompiled text.
///
/// `address` selects a function when the engine can be pointed at one; the
/// whole file is decompiled otherwise.
///
/// # Errors
///
/// Returns an error when the engine cannot be started, exceeds `timeout`, or
/// answers nothing usable.
pub fn decompile(
    engine: Engine,
    program: &Path,
    binary: &Path,
    address: Option<u64>,
    timeout: Duration,
) -> io::Result<String> {
    match engine {
        Engine::RzGhidra => rz_ghidra(program, binary, address, timeout),
        Engine::RetDec => retdec(program, binary, timeout),
    }
}

fn rz_ghidra(
    program: &Path,
    binary: &Path,
    address: Option<u64>,
    timeout: Duration,
) -> io::Result<String> {
    // `-N` skips the user's rizinrc, so a personal setting cannot change what
    // is shown here, and `scr.color=0` keeps escape sequences out of the text.
    let seek = address.map_or_else(
        || "s entry0".to_owned(),
        |address| format!("s {address:#x}"),
    );
    let script = format!("aaa; {seek}; pdg");
    let output = run_bounded(
        program,
        [
            OsStr::new("-N"),
            OsStr::new("-q"),
            OsStr::new("-e"),
            OsStr::new("scr.color=0"),
            OsStr::new("-c"),
            OsStr::new(&script),
            OsStr::new("--"),
            binary.as_os_str(),
        ],
        timeout,
    )?;
    usable(output)
}

fn retdec(program: &Path, binary: &Path, timeout: Duration) -> io::Result<String> {
    // RetDec writes its result to a file rather than to stdout.
    let output_file =
        env::temp_dir().join(format!("desdec-retdec-{}-{}.c", std::process::id(), tag()));
    let result = run_bounded(
        program,
        [
            OsStr::new("--cleanup"),
            OsStr::new("-o"),
            output_file.as_os_str(),
            binary.as_os_str(),
        ],
        timeout,
    );
    let decompiled = std::fs::read_to_string(&output_file);
    let _ = std::fs::remove_file(&output_file);
    let run = result?;
    usable(Run {
        stdout: decompiled?,
        stderr: run.stderr,
    })
}

/// A per-run number, so two decompilations never share a temporary file.
fn tag() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos())
}

/// Rejects an empty answer, quoting what the engine complained about.
///
/// An engine that fails usually explains itself on standard error; reporting
/// only "no output" left the user with nothing to act on.
fn usable(run: Run) -> io::Result<String> {
    if run.stdout.trim().is_empty() {
        let complaint = last_meaningful_line(&run.stderr);
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            complaint.map_or_else(
                || "the decompiler produced no output".to_owned(),
                |line| format!("the decompiler produced no output: {line}"),
            ),
        ));
    }
    Ok(run.stdout)
}

/// The last line worth showing from an engine's standard error.
///
/// Engines print progress and warnings there too, so the tail is what tends to
/// carry the reason; blank lines and pure progress noise are skipped.
fn last_meaningful_line(stderr: &str) -> Option<String> {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with("WARN"))
        .map(|line| {
            const LIMIT: usize = 200;
            if line.chars().count() > LIMIT {
                line.chars().take(LIMIT).collect::<String>() + "…"
            } else {
                line.to_owned()
            }
        })
}

/// What a finished process left behind.
struct Run {
    stdout: String,
    stderr: String,
}

/// Runs a program with a deadline, returning its standard output.
///
/// The child is killed when the deadline passes, so a decompiler that hangs on
/// a malformed file cannot keep a thread — or a temporary file — forever.
///
/// Output is drained by a thread while the deadline is watched: a decompiler
/// prints far more than a pipe holds, and a child blocked on a full pipe would
/// never reach the exit this function is waiting for.
fn run_bounded<I, S>(program: &Path, arguments: I, timeout: Duration) -> io::Result<Run>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the decompiler gave no output stream"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("the decompiler gave no error stream"))?;
    let reader = thread::spawn(move || {
        let mut collected = Vec::new();
        let _ = stdout.read_to_end(&mut collected);
        collected
    });
    // Drained on its own thread too: an engine that fills the error pipe would
    // otherwise block waiting for someone to read it, and never exit.
    let complaints = thread::spawn(move || {
        let mut collected = Vec::new();
        let _ = stderr.read_to_end(&mut collected);
        collected
    });

    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    loop {
        match child.try_wait()? {
            Some(_) => break,
            None if Instant::now() >= deadline => {
                // Killing the child closes the pipe, which ends the reader.
                let _ = child.kill();
                let _ = child.wait();
                timed_out = true;
                break;
            }
            None => thread::sleep(POLL_INTERVAL),
        }
    }

    let collected = reader.join().unwrap_or_default();
    let complaints = complaints.join().unwrap_or_default();
    if timed_out {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("the decompiler exceeded {} seconds", timeout.as_secs()),
        ));
    }
    Ok(Run {
        stdout: String::from_utf8_lossy(&collected).into_owned(),
        stderr: String::from_utf8_lossy(&complaints).into_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_engine_names_a_program_and_a_way_to_install_it() {
        for engine in Engine::ALL {
            assert!(!engine.program().is_empty());
            assert!(!engine.label().is_empty());
            assert!(!engine.install_hint().is_empty());
        }
    }

    #[test]
    fn a_configured_path_that_does_not_exist_is_missing_rather_than_ignored() {
        let availability = locate(
            Engine::RetDec,
            Some(Path::new("/nonexistent/retdec-decompiler")),
        );
        assert_eq!(availability, Availability::Missing);
    }

    #[test]
    fn an_engine_nobody_installed_is_reported_missing() {
        // A program name no distribution ships, looked up on the real `PATH`.
        let found = find_on_path("desdec-no-such-decompiler-xyz");
        assert_eq!(found, None);
    }

    /// The deadline is enforced by killing the child, not by waiting for it.
    #[test]
    fn a_program_that_never_finishes_is_killed_at_the_deadline() {
        let Some(sleep) = find_on_path("sleep") else {
            return; // No `sleep` on this platform; nothing to prove here.
        };
        let started = Instant::now();
        let result = run_bounded(&sleep, ["30"], Duration::from_millis(200));

        let error = result
            .map(|_| ())
            .expect_err("a run past its deadline must fail");
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the deadline should end the run, not the program"
        );
    }

    #[test]
    fn output_is_captured_from_a_program_that_finishes() {
        let Some(echo) = find_on_path("echo") else {
            return;
        };
        let run = run_bounded(&echo, ["decompiled"], Duration::from_secs(5))
            .expect("a program that finishes should be readable");
        assert_eq!(run.stdout.trim(), "decompiled");
    }

    /// A `rizin` without the plugin must be reported incomplete, not usable.
    ///
    /// The probe reads standard output, because a missing command is answered
    /// on standard error with a success exit status: judging by the status, or
    /// by "did it run at all", would call an engine ready that decompiles
    /// nothing. Skipped where rizin is not installed.
    #[test]
    fn rizin_without_the_plugin_is_incomplete_rather_than_found() {
        let Some(rizin) = find_on_path("rizin") else {
            return;
        };
        let availability = locate(Engine::RzGhidra, Some(&rizin));

        match availability {
            // Either answer is correct — it depends on this machine — but
            // "found" must mean `pdg` really answered.
            Availability::Found(_) => {
                let probe = run_bounded(
                    &rizin,
                    ["-N", "-q", "-c", "pdg?", "malloc://64"],
                    PROBE_TIMEOUT,
                )
                .expect("the probe runs");
                assert!(
                    probe.stdout.to_lowercase().contains("pdg"),
                    "an engine reported as found must really provide pdg"
                );
            }
            Availability::Incomplete { reason, .. } => assert_eq!(reason, "pdg"),
            Availability::Missing => panic!("rizin was found on PATH a moment ago"),
        }
    }

    #[test]
    fn an_engine_that_prints_nothing_is_an_error_rather_than_an_empty_view() {
        let run = |stdout: &str, stderr: &str| Run {
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        };
        assert!(usable(run("", "")).is_err());
        assert!(usable(run("   \n", "")).is_err());
        assert!(usable(run("int main() {}", "")).is_ok());

        // What the engine complained about reaches the message, so the user
        // has something to act on instead of a bare "no output".
        let reported = usable(run("", "ERROR: cannot open file\n"))
            .expect_err("an empty answer is a failure")
            .to_string();
        assert!(reported.contains("cannot open file"), "got {reported}");
    }
}
