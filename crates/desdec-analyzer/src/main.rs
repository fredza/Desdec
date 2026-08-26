use std::{path::PathBuf, process::ExitCode};

use desdec_analyzer::{ReportOptions, report};

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("desdec-analyzer: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<std::ffi::OsString>) -> Result<(), String> {
    if arguments.len() == 1 && arguments[0] == "--version" {
        println!(
            "desdec-analyzer {} (protocol {})",
            env!("CARGO_PKG_VERSION"),
            desdec_analyzer::PROTOCOL_VERSION
        );
        return Ok(());
    }
    let mut arguments = arguments.into_iter();
    if arguments.next().as_deref() != Some(std::ffi::OsStr::new("report")) {
        return Err("usage: desdec-analyzer report <binary> [--pretty] [--instructions]".into());
    }
    let path = arguments.next().map(PathBuf::from).ok_or_else(|| {
        "usage: desdec-analyzer report <binary> [--pretty] [--instructions]".to_owned()
    })?;
    let mut pretty = false;
    let mut options = ReportOptions::default();
    for argument in arguments {
        match argument.to_str() {
            Some("--pretty") => pretty = true,
            Some("--instructions") => options.instructions = true,
            Some(other) => return Err(format!("unknown option: {other}")),
            None => return Err("option is not valid UTF-8".into()),
        }
    }
    let analysed = desdec_core::analyse_path(&path)
        .map_err(|error| format!("cannot analyse {}: {error}", path.display()))?;
    let json = report(&path, &analysed, options);
    let text = if pretty {
        serde_json::to_string_pretty(&json)
    } else {
        serde_json::to_string(&json)
    }
    .map_err(|error| format!("cannot encode report: {error}"))?;
    println!("{text}");
    Ok(())
}
