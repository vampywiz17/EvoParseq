//! Linux file location/permissions; JSON schema and transactions are shared.
use anyhow::{Context, Result};
use std::{path::PathBuf, sync::Arc};
pub fn open() -> Result<Arc<crate::connection_settings::Manager>> {
    use std::os::unix::fs::PermissionsExt;
    let mut args = std::env::args_os().skip(1);
    let mut explicit = None;
    while let Some(arg) = args.next() {
        if arg == "--data-dir" {
            explicit = Some(PathBuf::from(
                args.next().context("--data-dir requires a directory")?,
            ));
        }
    }
    let directory = if let Some(path) = explicit {
        path
    } else {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .context("No user configuration directory")?;
        base.join("EvoParseq")
    };
    std::fs::create_dir_all(&directory).context("Cannot create settings directory")?;
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))?;
    Ok(crate::connection_settings::Manager::open(
        directory.join("settings.json"),
    ))
}
#[cfg(test)]
mod tests {
    #[test]
    fn linux_settings_match_windows_schema_preserve_unwired_options_and_use_private_atomic_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir =
            std::env::temp_dir().join(format!("evoparseq-settings-linux-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("settings.json");
        let input = serde_json::json!({"bud":true,"preferHevc":true,"experimentalZeroCopy":true,"nativeCore":{"email":"fixture@example.invalid","encryptedSession":"opaque-platform-blob","lastPeer":"fixture-host"},"futurePreference":{"enabled":true}});
        std::fs::write(&path, serde_json::to_vec(&input).unwrap()).unwrap();
        let manager = crate::connection_settings::Manager::open(path.clone());
        assert!(manager.view().0.bud);
        assert!(manager.view().0.prefer_hevc);
        manager
            .update_local(|settings| {
                settings.bud = false;
                Ok(())
            })
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            value["nativeCore"]["encryptedSession"],
            "opaque-platform-blob"
        );
        assert_eq!(value["futurePreference"], input["futurePreference"]);
        assert_eq!(value["experimentalZeroCopy"], true);
        assert_eq!(value["bud"], false);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::write(&path, b"not-json").unwrap();
        let broken = crate::connection_settings::Manager::open(path.clone());
        assert!(broken.update_local(|_| Ok(())).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not-json");
        std::fs::remove_file(path).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }
}
