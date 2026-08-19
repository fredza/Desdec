//! Scripts someone else wrote, installed to run on binaries the reader opens.
//!
//! A plugin is a directory: a manifest saying what it is and what it needs,
//! and the script itself. Nothing is compiled, nothing is loaded into this
//! process, and no code arrives as machine code — a plugin is text, read by
//! the same sandboxed engine the reader's own scripts run in, which is what
//! makes installing one a smaller decision than installing a native plugin
//! into a debugger.
//!
//! What it *is* still matters, because a plugin is code from somewhere else.
//! So a manifest asks for permissions and does not take them: the list is put
//! in front of the reader, in their own language, and a plugin that was never
//! enabled never runs. One that is enabled runs with exactly what was granted
//! — a plugin that asks for notes and later starts asking for patches is
//! stopped until the reader has seen the new list.
//!
//! None of this is a defence against the machine's owner. It is a defence
//! against surprise: what a plugin can do is written down before it does it.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::script::Permission;

/// What the file holding a plugin's manifest is called.
pub const MANIFEST: &str = "plugin.ron";

/// The longest a plugin's script may be, so a stray gigabyte in the plugin
/// directory is not read into memory to find out it is not a script.
const MAXIMUM_SOURCE: u64 = 4 * 1024 * 1024;

/// When a plugin runs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub enum Hook {
    /// Once, after a binary has been analysed. This is the one that makes a
    /// plugin worth installing: the reader opens a file and the names are
    /// already there.
    OnOpen,
    /// Only when the reader asks for it, from the plugin list or the command
    /// palette.
    OnDemand,
}

impl Hook {
    pub const ALL: &[Self] = &[Self::OnOpen, Self::OnDemand];

    #[must_use]
    pub const fn label(self) -> crate::i18n::Text {
        match self {
            Self::OnOpen => crate::i18n::Text::HookOnOpen,
            Self::OnDemand => crate::i18n::Text::HookOnDemand,
        }
    }
}

/// What a plugin says about itself.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    /// Shown to the reader. The directory's name is the identity; this is the
    /// name, and the two are allowed to differ.
    pub name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    /// The script, named relative to the plugin's own directory.
    pub script: String,
    pub hooks: Vec<Hook>,
    pub permissions: Vec<Permission>,
}

/// A plugin that could be read, with the script it runs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Plugin {
    /// The directory's name, which is what consent is remembered against: two
    /// plugins may call themselves the same thing, and only one of them can
    /// occupy a directory.
    pub id: String,
    pub directory: PathBuf,
    pub manifest: Manifest,
    pub source: String,
}

impl Plugin {
    /// What the reader calls it: its own name, or the directory it is in when
    /// the manifest gives none.
    #[must_use]
    pub fn title(&self) -> &str {
        let name = self.manifest.name.trim();
        if name.is_empty() { &self.id } else { name }
    }

    #[must_use]
    pub fn runs_on(&self, hook: Hook) -> bool {
        self.manifest.hooks.contains(&hook)
    }

    /// The permissions it asks for, in a fixed order and without repeats, so
    /// the list the reader agreed to can be compared with the list it asks for
    /// today.
    #[must_use]
    pub fn wanted(&self) -> Vec<Permission> {
        let mut wanted = self.manifest.permissions.clone();
        wanted.sort_unstable();
        wanted.dedup();
        wanted
    }
}

/// A directory that looks like a plugin and could not be read as one.
///
/// Kept and shown rather than skipped: a plugin that silently does not appear
/// is one the reader goes looking for in the wrong place.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Broken {
    pub id: String,
    pub directory: PathBuf,
    /// Said in the words of whatever refused it — the parser's line and
    /// column, or the file system's reason.
    pub reason: String,
}

/// Everything found in the plugin directory.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Installed {
    pub plugins: Vec<Plugin>,
    pub broken: Vec<Broken>,
}

impl Installed {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty() && self.broken.is_empty()
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Plugin> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }
}

/// Where plugins are installed.
#[must_use]
pub fn directory() -> Option<PathBuf> {
    Some(crate::storage::data_directory()?.join("plugins"))
}

/// Reads every plugin in `directory`, in name order.
///
/// A directory that is not there is no plugins rather than an error: not
/// having installed any is the ordinary case, and it is not a failure to
/// report.
#[must_use]
pub fn read(directory: &Path) -> Installed {
    let mut installed = Installed::default();
    let Ok(entries) = std::fs::read_dir(directory) else {
        return installed;
    };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    found.sort();
    for path in found {
        match read_one(&path) {
            Ok(plugin) => installed.plugins.push(plugin),
            Err(reason) => installed.broken.push(Broken {
                id: name_of(&path),
                directory: path,
                reason,
            }),
        }
    }
    installed
}

/// Reads one plugin directory, or says what stopped it.
fn read_one(directory: &Path) -> Result<Plugin, String> {
    let manifest_path = directory.join(MANIFEST);
    let text =
        std::fs::read_to_string(&manifest_path).map_err(|error| format!("{MANIFEST}: {error}"))?;
    let manifest: Manifest =
        ron::from_str(&text).map_err(|error| format!("{MANIFEST}: {error}"))?;
    let script = script_path(directory, &manifest.script)?;
    let length = std::fs::metadata(&script)
        .map_err(|error| format!("{}: {error}", manifest.script))?
        .len();
    if length > MAXIMUM_SOURCE {
        return Err(format!(
            "{}: {length} bytes is larger than a script can be",
            manifest.script
        ));
    }
    let source = std::fs::read_to_string(&script)
        .map_err(|error| format!("{}: {error}", manifest.script))?;
    Ok(Plugin {
        id: name_of(directory),
        directory: directory.to_path_buf(),
        manifest,
        source,
    })
}

/// The script named by a manifest, once it is established that it is inside
/// the plugin's own directory.
///
/// A manifest is a file from somewhere else, and `../../.ssh/id_ed25519` is a
/// perfectly good relative path. A plugin names one file, in its own
/// directory, and anything else is refused by name rather than followed.
fn script_path(directory: &Path, named: &str) -> Result<PathBuf, String> {
    let named = named.trim();
    if named.is_empty() {
        return Err(format!("{MANIFEST}: no script is named"));
    }
    let candidate = Path::new(named);
    let one_component = candidate
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
        && candidate.components().count() == 1;
    if !one_component {
        return Err(format!(
            "{named}: a plugin's script is one file in the plugin's own directory"
        ));
    }
    Ok(directory.join(candidate))
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// What the reader has decided about one plugin.
///
/// Absent from the preferences means never decided, which is not the same as
/// disabled: a plugin that was never looked at is listed as waiting for an
/// answer rather than silently switched off.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Consent {
    pub enabled: bool,
    /// Exactly what was granted, so a plugin whose manifest starts asking for
    /// more is stopped until the reader has seen the new list.
    pub granted: Vec<Permission>,
}

impl Consent {
    /// Whether what was granted covers what the plugin asks for today.
    #[must_use]
    pub fn covers(&self, wanted: &[Permission]) -> bool {
        wanted
            .iter()
            .all(|permission| self.granted.contains(permission))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plugin_in(directory: &Path, manifest: &str, script: &str) {
        std::fs::create_dir_all(directory).expect("the test directory can be created");
        std::fs::write(directory.join(MANIFEST), manifest).expect("the manifest can be written");
        std::fs::write(directory.join("plugin.rhai"), script).expect("the script can be written");
    }

    fn temporary(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("desdec-plugins-{name}"));
        let _ = std::fs::remove_dir_all(&directory);
        directory
    }

    #[test]
    fn a_plugin_is_read_with_its_script() {
        let root = temporary("read");
        plugin_in(
            &root.join("namer"),
            r#"(name: "Namer", version: "1.0", script: "plugin.rhai", hooks: [OnOpen], permissions: [WriteNotes])"#,
            "label(entry(), \"start\");",
        );
        let installed = read(&root);
        assert!(installed.broken.is_empty(), "{:?}", installed.broken);
        let plugin = installed.get("namer").expect("the plugin was read");
        assert_eq!(plugin.title(), "Namer");
        assert!(plugin.runs_on(Hook::OnOpen));
        assert!(!plugin.runs_on(Hook::OnDemand));
        assert_eq!(plugin.wanted(), vec![Permission::WriteNotes]);
        assert!(plugin.source.contains("label"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_script_outside_the_plugins_directory_is_refused() {
        let root = temporary("escape");
        plugin_in(
            &root.join("greedy"),
            r#"(name: "Greedy", script: "../../../etc/passwd")"#,
            "",
        );
        let installed = read(&root);
        assert!(installed.plugins.is_empty());
        let broken = installed.broken.first().expect("it was refused, not read");
        assert!(
            broken.reason.contains("own directory"),
            "the refusal says why: {}",
            broken.reason
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_manifest_that_does_not_parse_is_listed_rather_than_skipped() {
        let root = temporary("broken");
        plugin_in(&root.join("half"), "(name: \"Half\"", "");
        let installed = read(&root);
        assert!(installed.plugins.is_empty());
        assert_eq!(installed.broken.len(), 1, "the directory is still listed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_plugin_without_a_name_is_called_after_its_directory() {
        let root = temporary("unnamed");
        plugin_in(&root.join("quiet"), r#"(script: "plugin.rhai")"#, "");
        let installed = read(&root);
        assert_eq!(
            installed.plugins.first().map(Plugin::title),
            Some("quiet"),
            "{:?}",
            installed.broken
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn no_plugin_directory_is_no_plugins_and_no_complaint() {
        let installed = read(&temporary("absent"));
        assert!(installed.is_empty());
    }

    #[test]
    fn consent_does_not_cover_a_permission_added_after_it_was_given() {
        let consent = Consent {
            enabled: true,
            granted: vec![Permission::WriteNotes],
        };
        assert!(consent.covers(&[Permission::WriteNotes]));
        assert!(!consent.covers(&[Permission::WriteNotes, Permission::ProposePatches]));
    }
}
