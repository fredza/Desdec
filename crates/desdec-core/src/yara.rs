//! Optional, bounded integration with the local YARA command-line scanner.
//!
//! YARA never receives a shell command: the rules and binary paths are passed
//! as individual arguments. It analyses the bytes statically; Desdec never
//! executes the scanned binary.

use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Longest one YARA scan may run before Desdec terminates it.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(40);

/// One rule YARA reported as matching the analysed file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Match {
    pub rule: String,
    pub namespace: Option<String>,
}

/// Finds YARA, preferring a user-configured executable path.
///
/// Both the classic `yara` client and YARA-X's `yr` client are supported. The
/// latter is tried after the classic name when no explicit path is configured.
#[must_use]
pub fn locate(configured: Option<&Path>) -> Option<PathBuf> {
    configured
        .filter(|path| !path.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .or_else(|| find_on_path("yara"))
        .or_else(|| find_on_path("yr"))
        .filter(|path| path.is_file())
}

/// Runs a supported YARA client and returns the rules that matched.
///
/// The classic client receives `yara -w <rules> <binary>`; YARA-X's `yr`
/// client receives `yr scan --disable-warnings --timeout <seconds> <rules>
/// <binary>`.
///
/// # Errors
///
/// Returns an error when a configured input is unavailable, YARA cannot start,
/// it exceeds `timeout`, or its output cannot be read.
pub fn scan(
    program: &Path,
    rules: &Path,
    binary: &Path,
    timeout: Duration,
) -> io::Result<Vec<Match>> {
    if !rules.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the YARA rules file does not exist",
        ));
    }
    if !binary.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "the binary to scan does not exist",
        ));
    }
    let stdout = if is_yara_x(program) {
        let arguments = vec![
            OsString::from("scan"),
            OsString::from("--disable-warnings"),
            OsString::from("--timeout"),
            OsString::from(timeout.as_secs().to_string()),
            rules.as_os_str().to_owned(),
            binary.as_os_str().to_owned(),
        ];
        run_bounded(program, arguments, timeout)?
    } else {
        run_bounded(
            program,
            [OsStr::new("-w"), rules.as_os_str(), binary.as_os_str()],
            timeout,
        )?
    };
    Ok(parse(&stdout))
}

fn is_yara_x(program: &Path) -> bool {
    program.file_name().is_some_and(|name| name == "yr")
}

fn find_on_path(program: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|paths| {
        env::split_paths(&paths)
            .map(|directory| directory.join(program))
            .find(|candidate| candidate.is_file())
    })
}

fn run_bounded<I, S>(program: &Path, arguments: I, timeout: Duration) -> io::Result<String>
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
        .ok_or_else(|| io::Error::other("YARA gave no output stream"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("YARA gave no error stream"))?;
    let output = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        bytes
    });
    let complaints = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => thread::sleep(POLL_INTERVAL),
        }
    };

    let stdout = String::from_utf8_lossy(&output.join().unwrap_or_default()).into_owned();
    let stderr = String::from_utf8_lossy(&complaints.join().unwrap_or_default()).into_owned();
    let Some(status) = status else {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            format!("YARA exceeded {} seconds", timeout.as_secs()),
        ));
    };
    if !status.success() {
        return Err(io::Error::other(last_line(&stderr)));
    }
    if !stderr.trim().is_empty() && stdout.trim().is_empty() {
        return Err(io::Error::other(last_line(&stderr)));
    }
    Ok(stdout)
}

/// Normal YARA output is `<rule> <scanned path>`. A namespaced rule is printed
/// as `<namespace>:<rule> <scanned path>`.
fn parse(output: &str) -> Vec<Match> {
    output
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| !name.is_empty())
        .map(|name| match name.split_once(':') {
            Some((namespace, rule)) if !namespace.is_empty() && !rule.is_empty() => Match {
                rule: rule.to_owned(),
                namespace: Some(namespace.to_owned()),
            },
            _ => Match {
                rule: name.to_owned(),
                namespace: None,
            },
        })
        .collect()
}

fn last_line(stderr: &str) -> String {
    stderr
        .lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty())
        .unwrap_or("YARA produced no output")
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_and_namespaced_rules() {
        assert_eq!(
            parse("suspicious /tmp/sample\nmalware:packed /tmp/sample\n"),
            vec![
                Match {
                    rule: "suspicious".to_owned(),
                    namespace: None,
                },
                Match {
                    rule: "packed".to_owned(),
                    namespace: Some("malware".to_owned()),
                },
            ]
        );
    }

    #[test]
    fn detects_the_yara_x_client_by_its_standard_name() {
        assert!(is_yara_x(Path::new("/opt/yara-x/yr")));
        assert!(!is_yara_x(Path::new("/usr/bin/yara")));
    }

    #[test]
    fn missing_rules_are_rejected_before_starting_a_process() {
        let error = scan(
            Path::new("/not-run"),
            Path::new("/nonexistent/desdec-rules.yar"),
            Path::new("/nonexistent/desdec-binary"),
            DEFAULT_TIMEOUT,
        )
        .expect_err("a missing rules file must fail");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
