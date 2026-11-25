//! JSON persistence for the workspace store. Opaque to every other module.
//!
//! The file is a sequence of compact JSON values, one per line: a header with
//! the active id and allocator, then one value per workspace. A damaged line
//! therefore cannot prevent the complete entries around it from being read.

use anyhow::Context as _;
use canvas_types::{CustomAgent, WorkspaceId, WorkspaceIcon, WorkspaceState, WorkspaceStore};
use serde::Deserialize;
use serde_json::json;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const CONFIG_DIR: &str = "auro-canvas";
const CONFIG_FILE: &str = "workspaces.json";
const INITIAL_NAME: &str = "AURI";

/// Why a save was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveError {
    /// The file on disk could not be parsed; overwriting it could destroy user data.
    UnreadableConfig(String),
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnreadableConfig(reason) => {
                write!(f, "config file is unreadable; refusing to overwrite it: {reason}")
            }
        }
    }
}

impl std::error::Error for SaveError {}

/// Result of reading the config file into a store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// No config file yet; the store holds the initial workspace.
    Fresh,
    /// The file was read without errors.
    Loaded,
    /// Complete entries were read, but the file also contained damage.
    Recovered(String),
    /// Nothing usable was found; the store holds the initial workspace.
    Corrupt(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    #[serde(default)]
    active: Option<WorkspaceId>,
    #[serde(default = "default_next_id")]
    next_id: WorkspaceId,
    #[serde(default)]
    custom_agents: Vec<CustomAgent>,
    #[serde(default = "default_next_custom_id")]
    next_custom_id: u64,
}

fn default_next_id() -> WorkspaceId {
    1
}

fn default_next_custom_id() -> u64 {
    1
}

struct Parsed {
    header: Option<Header>,
    workspaces: Vec<WorkspaceState>,
    error: Option<String>,
}

/// Path of the workspace config, honouring the XDG configuration directory.
pub fn config_path() -> PathBuf {
    config_path_from(std::env::var_os("XDG_CONFIG_HOME"), std::env::var_os("HOME"))
}

/// Pure mapping so the XDG rules can be tested without mutating process env.
fn config_path_from(xdg: Option<OsString>, home: Option<OsString>) -> PathBuf {
    let base = xdg
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            home.filter(|value| !value.is_empty())
                .map(|home| PathBuf::from(home).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join(CONFIG_DIR).join(CONFIG_FILE)
}

/// Root used for the first workspace. `auro dev` runs with the app crate as
/// cwd, so relying on `current_dir()` here silently pins every agent to the
/// app's own directory; the launcher can name the real project root instead.
fn initial_root(override_root: Option<OsString>, cwd: PathBuf) -> PathBuf {
    override_root
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or(cwd)
}

fn initial_store() -> WorkspaceStore {
    let root = initial_root(
        std::env::var_os("AURI_WORKSPACE_ROOT"),
        std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    );
    let mut store = WorkspaceStore::new();
    store.add(INITIAL_NAME.into(), WorkspaceIcon::Star, root);
    store
}

fn root_is_usable(root: &Path) -> bool {
    fs::metadata(root).is_ok_and(|metadata| metadata.is_dir())
}

/// Parse one workspace line, recording damage if it is not a complete entry.
fn push_workspace(parsed: &mut Parsed, line: &str, line_number: usize) -> bool {
    match serde_json::from_str::<WorkspaceState>(line) {
        Ok(mut workspace) => {
            workspace.invalid_root = !root_is_usable(&workspace.root);
            parsed.workspaces.push(workspace);
            true
        }
        Err(error) => {
            parsed.error = Some(format!("line {line_number}: {error}"));
            false
        }
    }
}

/// Parse every line independently so complete entries survive a damaged file.
fn parse_entries(text: &str) -> Parsed {
    let mut parsed = Parsed {
        header: None,
        workspaces: Vec::new(),
        error: None,
    };
    let mut line_number = 0;

    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        line_number += 1;

        if line_number == 1 {
            match serde_json::from_str::<Header>(line) {
                Ok(header) => {
                    parsed.header = Some(header);
                    continue;
                }
                Err(_) => {
                    if push_workspace(&mut parsed, line, line_number) {
                        parsed.error = Some(format!("line {line_number}: missing config header"));
                    }
                    continue;
                }
            }
        }

        push_workspace(&mut parsed, line, line_number);
    }

    if parsed.header.is_none() {
        parsed
            .error
            .get_or_insert_with(|| "missing config header".to_owned());
    }
    parsed
}

/// Rebuild a store, keeping the allocator ahead of every id found in the file.
fn build_store(parsed: &Parsed) -> Result<WorkspaceStore, String> {
    let highest_id = parsed
        .workspaces
        .iter()
        .map(|workspace| workspace.id.saturating_add(1))
        .max()
        .unwrap_or(1);
    let next_id = parsed
        .header
        .as_ref()
        .map_or(1, |header| header.next_id)
        .max(highest_id);
    let active = parsed
        .header
        .as_ref()
        .and_then(|header| header.active)
        .filter(|id| parsed.workspaces.iter().any(|workspace| workspace.id == *id));
    let (custom_agents, next_custom_id) = parsed
        .header
        .as_ref()
        .map(|header| (header.custom_agents.clone(), header.next_custom_id))
        .unwrap_or_default();
    let custom_floor = next_id.max(
        parsed
            .workspaces
            .iter()
            .map(|workspace| workspace.canvas.next_id())
            .max()
            .unwrap_or(1),
    );
    let next_custom_id = custom_agents
        .iter()
        .map(|agent| agent.id.saturating_add(1))
        .max()
        .unwrap_or(1)
        .max(custom_floor)
        .max(next_custom_id);

    let value = json!({
        "active": active,
        "next_id": next_id,
        "custom_agents": custom_agents,
        "next_custom_id": next_custom_id,
        "workspaces": &parsed.workspaces,
    });
    serde_json::from_value(value).map_err(|error| format!("invalid workspace entries: {error}"))
}

fn read_strict(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let text = std::str::from_utf8(&bytes).map_err(|error| format!("invalid UTF-8: {error}"))?;
    let parsed = parse_entries(text);
    if let Some(error) = parsed.error {
        return Err(error);
    }
    build_store(&parsed).map(|_| ())
}

/// Read the config into `store` without ever writing to the file.
pub fn load(store: &mut WorkspaceStore) -> LoadOutcome {
    load_at(store, &config_path())
}

fn load_at(store: &mut WorkspaceStore, path: &Path) -> LoadOutcome {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            *store = initial_store();
            return LoadOutcome::Fresh;
        }
        Err(error) => {
            *store = initial_store();
            return LoadOutcome::Corrupt(format!("cannot read {}: {error}", path.display()));
        }
    };
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(error) => {
            *store = initial_store();
            return LoadOutcome::Corrupt(format!("invalid UTF-8: {error}"));
        }
    };

    let parsed = parse_entries(text);
    if let Some(error) = &parsed.error {
        if !parsed.workspaces.is_empty() {
            return match build_store(&parsed) {
                Ok(loaded) => {
                    *store = loaded;
                    LoadOutcome::Recovered(error.clone())
                }
                Err(reason) => {
                    *store = initial_store();
                    LoadOutcome::Corrupt(reason)
                }
            };
        }
        *store = initial_store();
        return LoadOutcome::Corrupt(error.clone());
    }

    match build_store(&parsed) {
        Ok(loaded) => {
            *store = loaded;
            LoadOutcome::Loaded
        }
        Err(reason) => {
            *store = initial_store();
            LoadOutcome::Corrupt(reason)
        }
    }
}

fn write_store(path: &Path, store: &WorkspaceStore) -> anyhow::Result<()> {
    let value = serde_json::to_value(store).context("serialize workspace store")?;
    let header = json!({
        "active": value["active"],
        "next_id": value["next_id"],
        "custom_agents": value["custom_agents"],
        "next_custom_id": value["next_custom_id"],
    });
    let workspaces = value["workspaces"]
        .as_array()
        .context("serialized store has no workspaces")?;

    let mut content = serde_json::to_string(&header)?;
    content.push('\n');
    for workspace in workspaces {
        content.push_str(&serde_json::to_string(workspace)?);
        content.push('\n');
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    Ok(())
}

/// Persist `store`, unless the existing file is unreadable.
pub fn save(store: &WorkspaceStore) -> anyhow::Result<()> {
    save_at(store, &config_path())
}

fn save_at(store: &WorkspaceStore, path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        if let Err(reason) = read_strict(path) {
            return Err(SaveError::UnreadableConfig(reason).into());
        }
    }
    write_store(path, store)
}

/// "Recomeçar": keep the damaged bytes as `workspaces.json.bad-<timestamp>`,
/// then install and persist a fresh initial store.
pub fn restart(store: &mut WorkspaceStore) -> anyhow::Result<()> {
    restart_at(store, &config_path())
}

fn restart_at(store: &mut WorkspaceStore, path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("system clock is before the Unix epoch")?
            .as_millis();
        let mut backup = path.as_os_str().to_os_string();
        backup.push(format!(".bad-{timestamp}"));
        fs::copy(&path, PathBuf::from(backup)).context("back up unreadable config")?;
    }
    *store = initial_store();
    write_store(&path, store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use canvas_types::{AgentIcon, AgentKind, CardBounds, WorkspaceIcon, WorkspaceStore};
    use std::fs;
    use std::path::{Path, PathBuf};

    fn config_file(dir: &Path) -> PathBuf {
        dir.join("auro-canvas").join("workspaces.json")
    }

    #[test]
    fn config_path_uses_xdg_config_home() {
        let xdg = Path::new("/tmp/xdg-config");
        assert_eq!(
            config_path_from(Some(xdg.as_os_str().to_os_string()), None),
            config_file(xdg)
        );
    }

    #[test]
    fn config_path_falls_back_to_home_config() {
        let home = Path::new("/home/tester");
        assert_eq!(
            config_path_from(None, Some(home.as_os_str().to_os_string())),
            home.join(".config").join("auro-canvas").join("workspaces.json")
        );
    }

    #[test]
    fn empty_xdg_config_home_falls_back_to_home() {
        let home = Path::new("/home/tester");
        assert_eq!(
            config_path_from(Some(OsString::new()), Some(home.as_os_str().to_os_string())),
            home.join(".config").join("auro-canvas").join("workspaces.json")
        );
    }

    #[test]
    fn initial_root_prefers_explicit_override() {
        let cwd = PathBuf::from("/app/canvas");
        let project = PathBuf::from("/home/pdl/Documentos/auri");

        assert_eq!(initial_root(Some(project.as_os_str().to_os_string()), cwd.clone()), project);
        assert_eq!(initial_root(Some(OsString::new()), cwd.clone()), cwd);
        assert_eq!(initial_root(None, cwd.clone()), cwd);
    }

    #[test]
    fn fresh_config_provides_initial_workspace_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());

        let mut store = WorkspaceStore::new();
        assert_eq!(load_at(&mut store, &path), LoadOutcome::Fresh);
        assert!(!path.exists());
        assert_eq!(store.workspaces().len(), 1);
        assert_eq!(store.workspaces()[0].name, "AURI");
        assert_eq!(store.workspaces()[0].icon, WorkspaceIcon::Star);
        assert_eq!(store.workspaces()[0].root, initial_root(None, std::env::current_dir().unwrap()));
        assert_eq!(store.active(), Some(1));
        assert!(store.workspaces()[0].unloaded);
    }

    #[test]
    fn save_then_load_preserves_name_icon_and_root() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut store = WorkspaceStore::new();
        let ws = store.add("Meu projeto".into(), WorkspaceIcon::Bolt, root.clone());
        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        assert_eq!(loaded.workspaces().len(), 1);
        let got = &loaded.workspaces()[0];
        assert_eq!(got.id, ws.id);
        assert_eq!(got.name, "Meu projeto");
        assert_eq!(got.icon, WorkspaceIcon::Bolt);
        assert_eq!(got.root, root);
        assert!(!got.invalid_root, "an existing root is valid after load");
        assert!(got.unloaded, "new workspaces start unloaded");
        assert_eq!(loaded.active(), Some(ws.id));
    }

    #[test]
    fn unloaded_state_survives_save_and_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut store = WorkspaceStore::new();
        let ws = store.add("Descarregado".into(), WorkspaceIcon::Star, root);
        store.workspace_mut(ws.id).unwrap().unload();
        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let got = &loaded.workspaces()[0];
        assert!(got.unloaded);
        assert!(got.canvas.terminals().is_empty());
        assert!(got.canvas.notes().is_empty());
    }

    #[test]
    fn all_icon_variants_survive_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let icons = [
            WorkspaceIcon::Star,
            WorkspaceIcon::Folder,
            WorkspaceIcon::Bolt,
            WorkspaceIcon::Code,
            WorkspaceIcon::Globe,
        ];
        let mut store = WorkspaceStore::new();
        for (index, icon) in icons.iter().enumerate() {
            store.add(format!("Espaço {index}"), *icon, root.clone());
        }
        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let loaded_icons: Vec<_> = loaded.workspaces().iter().map(|ws| ws.icon).collect();
        assert_eq!(loaded_icons, icons);
    }

    #[test]
    fn malformed_file_is_reported_corrupt_and_left_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bad = b"isto nao e json\n";
        fs::write(&path, bad).unwrap();

        let mut store = WorkspaceStore::new();
        assert!(matches!(&load_at(&mut store, &path), LoadOutcome::Corrupt(error) if !error.is_empty()));
        assert_eq!(fs::read(&path).unwrap(), bad);
        assert_eq!(store.workspaces().len(), 1);
        assert_eq!(store.workspaces()[0].name, "AURI");
        assert_eq!(store.workspaces()[0].icon, WorkspaceIcon::Star);
    }

    #[test]
    fn complete_entries_are_recovered_from_damaged_file_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut original = WorkspaceStore::new();
        original.add("Um".into(), WorkspaceIcon::Star, root.clone());
        original.add("Dois".into(), WorkspaceIcon::Globe, root.clone());
        save_at(&original, &path).unwrap();

        let mut damaged = fs::read(&path).unwrap();
        damaged.extend_from_slice(b"{\"id\":3,\"name\":\n");
        fs::write(&path, &damaged).unwrap();

        let mut recovered = WorkspaceStore::new();
        assert!(matches!(&load_at(&mut recovered, &path), LoadOutcome::Recovered(error) if !error.is_empty()));
        assert_eq!(recovered.workspaces().len(), 2);
        assert_eq!(recovered.workspaces()[1].name, "Dois");
        assert_eq!(fs::read(&path).unwrap(), damaged);
        assert!(
            save_at(&recovered, &path).is_err(),
            "recovered config must not be overwritten"
        );
    }

    #[test]
    fn damaged_header_still_allows_workspace_entries_to_be_recovered() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut original = WorkspaceStore::new();
        original.add("Um".into(), WorkspaceIcon::Star, root.clone());
        original.add("Dois".into(), WorkspaceIcon::Folder, root.clone());
        save_at(&original, &path).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        let entries = saved.split_once('\n').unwrap().1;
        let damaged = format!("header quebrado\n{entries}");
        fs::write(&path, &damaged).unwrap();

        let mut recovered = WorkspaceStore::new();
        assert!(matches!(load_at(&mut recovered, &path), LoadOutcome::Recovered(_)));
        assert_eq!(recovered.workspaces().len(), 2);
        assert_eq!(recovered.workspaces()[1].name, "Dois");
        assert_eq!(fs::read_to_string(&path).unwrap(), damaged);
    }

    #[test]
    fn missing_header_line_keeps_first_workspace_and_signals_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut original = WorkspaceStore::new();
        original.add("Primeiro".into(), WorkspaceIcon::Star, root.clone());
        original.add("Segundo".into(), WorkspaceIcon::Code, root.clone());
        save_at(&original, &path).unwrap();

        let saved = fs::read_to_string(&path).unwrap();
        let entries = saved.split_once('\n').unwrap().1;
        fs::write(&path, entries).unwrap();

        let mut recovered = WorkspaceStore::new();
        let outcome = load_at(&mut recovered, &path);
        assert!(
            matches!(outcome, LoadOutcome::Recovered(_) | LoadOutcome::Corrupt(_)),
            "a config without its header must signal recovery, got {outcome:?}"
        );
        assert_eq!(recovered.workspaces().len(), 2);
        assert_eq!(recovered.workspaces()[0].name, "Primeiro");
        assert_eq!(recovered.workspaces()[1].name, "Segundo");
        assert_eq!(fs::read_to_string(&path).unwrap(), entries);
        assert!(
            save_at(&recovered, &path).is_err(),
            "damaged config must not be overwritten"
        );
    }

    #[test]
    fn save_refuses_to_overwrite_corrupt_config_with_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bad = b"conteudo ruim\n";
        fs::write(&path, bad).unwrap();

        let mut store = WorkspaceStore::new();
        assert!(matches!(load_at(&mut store, &path), LoadOutcome::Corrupt(_)));
        let error = save_at(&store, &path).unwrap_err();
        assert!(
            error.downcast_ref::<SaveError>().is_some(),
            "expected a typed SaveError, got {error:?}"
        );
        assert_eq!(fs::read(&path).unwrap(), bad);
    }

    #[test]
    fn restart_backs_up_bad_config_before_writing_a_fresh_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let bad = b"conteudo ruim\n";
        fs::write(&path, bad).unwrap();

        let mut store = WorkspaceStore::new();
        assert!(matches!(load_at(&mut store, &path), LoadOutcome::Corrupt(_)));
        restart_at(&mut store, &path).unwrap();

        let backups: Vec<PathBuf> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|candidate| {
                candidate
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("workspaces.json.bad-"))
            })
            .collect();
        assert_eq!(backups.len(), 1);
        assert_eq!(fs::read(&backups[0]).unwrap(), bad);

        let mut reloaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut reloaded, &path), LoadOutcome::Loaded);
        assert_eq!(reloaded.workspaces().len(), 1);
        assert_eq!(reloaded.workspaces()[0].name, "AURI");
        save_at(&reloaded, &path).unwrap();
    }

    #[test]
    fn load_marks_persisted_workspace_with_missing_root_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());

        let mut store = WorkspaceStore::new();
        store.add("Velho".into(), WorkspaceIcon::Folder, dir.path().join("foi-embora"));
        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        assert!(loaded.workspaces()[0].invalid_root);
    }
    #[test]
    fn save_then_load_preserves_terminal_bounds_and_note_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut store = WorkspaceStore::new();
        store.add("Canvas".into(), WorkspaceIcon::Star, root);
        let terminal_bounds = CardBounds {
            x: 16.0,
            y: 24.0,
            width: 640.0,
            height: 400.0,
        };
        let note_bounds = CardBounds {
            x: -32.5,
            y: 88.0,
            width: 320.0,
            height: 220.0,
        };
        let canvas = &mut store.active_mut().unwrap().canvas;
        let terminal = canvas.add();
        assert!(canvas.set_terminal_bounds(terminal.id, terminal_bounds));
        let note = canvas.add_note("linha um\nlinha dois".into(), note_bounds);

        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let canvas = &loaded.workspaces()[0].canvas;
        assert_eq!(canvas.terminals().len(), 1);
        assert_eq!(canvas.terminals()[0].bounds, Some(terminal_bounds));
        assert_eq!(canvas.notes().len(), 1);
        assert_eq!(canvas.notes()[0].id, note.id);
        assert_eq!(canvas.notes()[0].text, "linha um\nlinha dois");
        assert_eq!(canvas.notes()[0].bounds, note_bounds);
    }

    #[test]
    fn save_then_load_preserves_agents_and_shared_allocator() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut store = WorkspaceStore::new();
        store.add("Canvas".into(), WorkspaceIcon::Star, root);
        let bounds = CardBounds {
            x: 48.0,
            y: 64.0,
            width: 640.0,
            height: 400.0,
        };
        let canvas = &mut store.active_mut().unwrap().canvas;
        let terminal = canvas.add();
        let agent = canvas.add_agent(AgentKind::OpenCode, bounds);
        canvas.rename_agent(agent.id, "Meu agente".into());
        canvas.add_note("nota".into(), bounds);

        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let canvas = &loaded.workspaces()[0].canvas;
        assert_eq!(canvas.terminals()[0].id, terminal.id);
        assert_eq!(canvas.agents().len(), 1);
        assert_eq!(canvas.agents()[0].id, agent.id);
        assert_eq!(canvas.agents()[0].kind, agent.kind);
        assert_eq!(canvas.agents()[0].name, "Meu agente");
        assert_eq!(canvas.agents()[0].kind, AgentKind::OpenCode);
        assert_eq!(canvas.agents()[0].bounds, bounds);
        assert_eq!(canvas.notes().len(), 1);
        assert_eq!(canvas.next_id(), 4);
    }

    #[test]
    fn old_workspace_line_without_geometry_or_notes_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = json!({
            "id": 1,
            "name": "Antigo",
            "icon": "Star",
            "color": "Brass",
            "root": root,
            "invalid_root": false,
            "canvas": {
                "terminals": [{"id": 1, "title": "Terminal 1", "exited": false}],
                "selected": 1,
                "next_id": 2
            }
        });
        fs::write(&path, format!("{{\"active\":1,\"next_id\":2}}\n{legacy}\n")).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let canvas = &loaded.workspaces()[0].canvas;
        assert_eq!(canvas.terminals()[0].bounds, None);
        assert!(canvas.agents().is_empty());
        assert!(canvas.notes().is_empty());
        assert_eq!(canvas.next_id(), 2);
        assert!(!loaded.workspaces()[0].unloaded);

        let note = loaded.active_mut().unwrap().canvas.add_note("novo".into(), CardBounds {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        assert_eq!(note.id, 2);
        save_at(&loaded, &path).unwrap();
    }

    #[test]
    fn save_then_load_preserves_custom_agents_and_allocator() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();

        let mut store = WorkspaceStore::new();
        store.add("Canvas".into(), WorkspaceIcon::Star, root);
        let agent = store.add_custom_agent(
            "Meu agente".into(),
            "opencode --model x".into(),
            AgentIcon::Codex,
        );
        let bounds = CardBounds { x: 0.0, y: 0.0, width: 10.0, height: 10.0 };
        store.active_mut().unwrap().canvas.add_agent_named(
            agent.name.clone(),
            agent.kind(),
            bounds,
        );

        save_at(&store, &path).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        assert_eq!(loaded.custom_agents(), &[agent.clone()]);
        assert_eq!(loaded.custom_agent(agent.id).unwrap().command, "opencode --model x");
        assert_eq!(loaded.custom_agent(agent.id).unwrap().icon, AgentIcon::Codex);
        assert_eq!(loaded.workspaces()[0].canvas.agents()[0].kind, agent.kind());

        let next = loaded.add_custom_agent("Outro".into(), "pi".into(), AgentIcon::Pi);
        assert!(next.id > agent.id);
    }

    #[test]
    fn old_workspace_line_without_connections_or_admin_still_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_file(dir.path());
        let root = dir.path().join("root");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let legacy = json!({
            "id": 1,
            "name": "Antigo",
            "icon": "Star",
            "root": root,
            "canvas": {
                "terminals": [],
                "notes": [],
                "agents": [{
                    "id": 2,
                    "kind": "Claude",
                    "name": "Claude 1",
                    "bounds": {"x": 0.0, "y": 0.0, "width": 640.0, "height": 400.0}
                }],
                "selected": 2,
                "next_id": 3
            }
        });
        fs::write(&path, format!("{{\"active\":1,\"next_id\":3}}\n{legacy}\n")).unwrap();

        let mut loaded = WorkspaceStore::new();
        assert_eq!(load_at(&mut loaded, &path), LoadOutcome::Loaded);
        let canvas = &loaded.workspaces()[0].canvas;
        assert!(canvas.connections().is_empty());
        assert_eq!(canvas.agents().len(), 1);
        assert!(!canvas.agents()[0].admin);
        assert_eq!(canvas.next_id(), 3);
    }
}
