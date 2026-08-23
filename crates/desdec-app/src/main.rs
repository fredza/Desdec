//! Native entry point of the Desdec binary explorer.

// Built without this, a Windows binary is a console application: the window
// opens with a black terminal behind it that nothing ever writes to, because
// nothing here prints — the path given on the command line is read in
// `DesdecApp::new` and answered in the window. The one message that would
// have reached that console is the error `main` returns when the window
// cannot be created at all, so the console stays in a debug build, which is
// what `Platform binaries` publishes beside each release for exactly that
// kind of diagnosis.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;

mod annotations;
mod app;
mod callgraph;
mod cli;
mod commands;
mod fonts;
mod i18n;
mod icon;
mod icons;
mod journal;
mod libraries;
mod names;
mod patches;
mod plugins;
mod preferences;
mod script;
mod search;
mod storage;
#[cfg(test)]
mod testing;
mod ui;
mod walk;
mod xrefs;

use app::DesdecApp;

const INITIAL_SIZE: [f32; 2] = [1120.0, 720.0];
const MINIMUM_SIZE: [f32; 2] = [860.0, 560.0];

fn main() -> eframe::Result<()> {
    // Asked and answered before anything else is set up: `--version` should
    // not have to open a window, start a thread or touch the preferences
    // store to say a number that was compiled in.
    let path = match cli::read(std::env::args_os().skip(1)) {
        cli::Request::Say(said) => {
            println!("{said}");
            return Ok(());
        }
        cli::Request::Refuse(refused) => {
            eprintln!("{refused}");
            std::process::exit(2);
        }
        cli::Request::WriteIcon(file) => {
            // No window, no context of the application's own: the icon is
            // drawn on a context built for the purpose and thrown away, so
            // asking for it costs a few milliseconds and touches nothing.
            let image = icon::render(icon::SIDE);
            if let Err(error) = std::fs::write(&file, icon::png(&image)) {
                eprintln!("desdec: {}: {error}", file.display());
                std::process::exit(2);
            }
            return Ok(());
        }
        cli::Request::Window(path) => path,
    };

    // Then, before anything starts a thread: the local time offset can only
    // be read while this process has one thread, and the session's account is
    // stamped with it. See [`journal::capture_local_offset`].
    journal::capture_local_offset();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(INITIAL_SIZE)
            .with_min_inner_size(MINIMUM_SIZE)
            // Names the storage directory holding the saved preferences, so it
            // no longer depends on the translated window title. It is also
            // what a Wayland desktop matches against the name of the
            // `.desktop` file, which is why that file is `Desdec.desktop`.
            .with_app_id("Desdec")
            // The window's own mark, drawn from the same vectors as the
            // interface. Without it a taskbar shows the toolkit's default,
            // which is nothing at all on most desktops.
            .with_icon(icon::window_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Desdec",
        options,
        Box::new(move |creation_context| Ok(Box::new(DesdecApp::new(creation_context, path)))),
    )
}
