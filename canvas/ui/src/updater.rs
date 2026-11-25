//! GitHub Releases update channel bootstrap.
//!
//! Auro owns the trust boundary: the pinned public key verifies the signed
//! manifest before the artifact URL is used. This module only wires the
//! documented flow into Auri's startup and lets the restart helper finish the
//! apply when an update was prepared.

use std::error::Error;
use std::path::{Path, PathBuf};

use auro::update::{ApplyJournal, ChannelConfig, GithubChannel, Recovery, UpdateError};
use auro::Config;

/// Reconcile a journal left by the restart helper, then check for a new
/// release. Returns `true` when the app must exit so the helper can apply.
pub fn prepare(dir: &Path) -> Result<bool, Box<dyn Error>> {
    let config = Config::load(dir)?;
    let state_dir = update_state_dir()?;
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
