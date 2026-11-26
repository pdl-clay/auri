//! GitHub Releases update channel bootstrap.
//!
//! Auro owns the trust boundary: the pinned public key verifies the signed
//! manifest before the artifact URL is used. This module only wires the
//! documented flow into Auri's startup and lets the restart helper finish the
//! apply when an update was prepared.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use auro::update::{ApplyJournal, ChannelConfig, GithubChannel, Recovery, UpdateError};
use auro::Config;

/// The AppImage has no `Auro.toml` beside its binary, so the release settings
/// are compiled in and materialised in the private update state directory.
const EMBEDDED_CONFIG: &str = include_str!("../../Auro.toml");

/// Reconcile a journal left by the restart helper, then check for a new
/// release. Returns `true` when the app must exit so the helper can apply.
pub fn prepare(_dir: &Path) -> Result<bool, Box<dyn Error>> {
    let state_dir = update_state_dir()?;
    let config = load_config(&state_dir)?;
    reconcile(&state_dir, &config.app.version)?;
    let Some(settings) = config.updater.as_ref() else {
        return Ok(false);
    };
    let channel = GithubChannel::new(
        ChannelConfig::new(&settings.repository, &settings.public_key)
            .with_manifest_asset(&settings.manifest_asset)
            .with_interval_secs(settings.interval_secs)
            .with_check_on_start(settings.check_on_start)
            .with_prereleases(settings.include_prereleases),
        state_dir,
        &config.app.version,
    )?;
    let target = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .unwrap_or(std::env::current_exe()?);
    Ok(channel.apply_if_due(target, None)?)
}


/// Update state must be writable from an AppImage and private to this user:
/// the journal names the verification key, staged artifact and relaunch path.
fn update_state_dir() -> Result<PathBuf, Box<dyn Error>> {
    let path = std::env::var_os("AURI_UPDATE_STATE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_STATE_HOME")
                .map(|root| PathBuf::from(root).join("auri"))
        })
        .or_else(|| {
            std::env::var_os("XDG_CACHE_HOME")
                .map(|root| PathBuf::from(root).join("auri"))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .map(|home| PathBuf::from(home).join(".local/state/auri"))
        })
        .ok_or_else(|| Box::<dyn Error>::from("no writable update state directory"))?;
    std::fs::create_dir_all(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

fn load_config(state_dir: &Path) -> Result<Config, Box<dyn Error>> {
    let path = materialise_config(state_dir)?;
    Ok(Config::load_from(&path)?)
}

fn materialise_config(state_dir: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let path = state_dir.join("Auri.toml");
    let current = fs::read_to_string(&path).unwrap_or_default();
    if current != EMBEDDED_CONFIG {
        let staged = state_dir.join("Auri.toml.tmp");
        fs::write(&staged, EMBEDDED_CONFIG)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&staged, fs::Permissions::from_mode(0o600))?;
        }
        fs::rename(&staged, &path)?;
    }
    // App::run loads Config before looking at the current directory, and the
    // AppImage wrapper changes that directory to its own usr/ folder.
    unsafe { std::env::set_var("AURO_CONFIG", &path) };
    Ok(path)
}

fn reconcile(state_dir: &Path, version: &str) -> Result<(), Box<dyn Error>> {
    let path = state_dir.join("update.json");
    let journal = match ApplyJournal::load(&path) {
        Ok(journal) => journal,
        Err(UpdateError::JournalMissing { .. }) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    match journal.reconcile(version)? {
        Recovery::Complete => journal.finalize()?,
        Recovery::RetryApply | Recovery::Relaunch => {
            eprintln!("auro: update needs a restart helper; launch the app again")
        }
        Recovery::AwaitReboot => eprintln!("auro: installer requested a reboot"),
        Recovery::Failed | Recovery::RolledBack => {
            eprintln!("auro: update failed; journal retained for diagnosis")
        }
        Recovery::Prepared => eprintln!("auro: prepared journal has no apply kind"),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_config_replaces_a_missing_or_stale_file() {
        let previous = std::env::var_os("AURO_CONFIG");
        let dir = tempfile::tempdir().unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.app.version, "0.1.1");

        let path = dir.path().join("Auri.toml");
        assert_eq!(fs::read_to_string(&path).unwrap(), EMBEDDED_CONFIG);
        assert_eq!(std::env::var_os("AURO_CONFIG"), Some(path.clone().into_os_string()));

        fs::write(&path, "stale = true\n").unwrap();
        let config = load_config(dir.path()).unwrap();
        assert_eq!(config.app.version, "0.1.1");
        assert_eq!(fs::read_to_string(&path).unwrap(), EMBEDDED_CONFIG);

        unsafe {
            match previous {
                Some(value) => std::env::set_var("AURO_CONFIG", value),
                None => std::env::remove_var("AURO_CONFIG"),
            }
        }
    }
}

