use std::collections::HashSet;
use std::env;
use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use thiserror::Error;

use crate::listener::GsiEndpoint;

pub const GSI_CONFIG_FILE_NAME: &str = "gamestate_integration_openclip.cfg";
const CS2_APP_ID: &str = "730";
const DEFAULT_INSTALL_DIR: &str = "Counter-Strike Global Offensive";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cs2Install {
    pub steam_library: PathBuf,
    pub install_dir: PathBuf,
    pub cfg_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigInstallAction {
    Installed,
    Updated,
    AlreadyCurrent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInstallResult {
    pub path: PathBuf,
    pub action: ConfigInstallAction,
}

#[derive(Debug, Error)]
pub enum InstallError {
    #[error("Steam installation could not be located")]
    SteamNotFound,
    #[error("Counter-Strike 2 is not installed in any discovered Steam library")]
    Cs2NotFound,
    #[error("Counter-Strike 2 config directory does not exist: {0}")]
    ConfigDirectoryMissing(PathBuf),
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

#[derive(Debug, Default)]
pub struct SteamLocator;

impl SteamLocator {
    /// Discovers Steam through the Windows registry and common program directories, then follows
    /// `libraryfolders.vdf` to every configured library.
    ///
    /// # Errors
    ///
    /// Returns a useful not-found or filesystem error when Steam/CS2 cannot be resolved.
    pub fn discover() -> Result<Cs2Install, InstallError> {
        let roots = steam_root_candidates();
        if roots.is_empty() {
            return Err(InstallError::SteamNotFound);
        }
        Self::locate_from_roots(&roots)
    }

    /// Resolves CS2 from explicit Steam roots. This is deterministic for tests and diagnostics.
    ///
    /// # Errors
    ///
    /// Returns `Cs2NotFound` when no library contains app manifest 730.
    pub fn locate_from_roots(roots: &[PathBuf]) -> Result<Cs2Install, InstallError> {
        let mut libraries = Vec::new();
        let mut seen = HashSet::new();
        for root in roots.iter().filter(|root| root.is_dir()) {
            add_unique_path(&mut libraries, &mut seen, root.clone());
            let library_file = root.join("steamapps/libraryfolders.vdf");
            if library_file.is_file() {
                let contents =
                    fs::read_to_string(&library_file).map_err(|source| InstallError::Read {
                        path: library_file.clone(),
                        source,
                    })?;
                for library in parse_library_paths(&contents) {
                    add_unique_path(&mut libraries, &mut seen, library);
                }
            }
        }

        for library in libraries {
            let steamapps = library.join("steamapps");
            let manifest = steamapps.join(format!("appmanifest_{CS2_APP_ID}.acf"));
            if !manifest.is_file() {
                continue;
            }
            let manifest_contents =
                fs::read_to_string(&manifest).map_err(|source| InstallError::Read {
                    path: manifest.clone(),
                    source,
                })?;
            let install_name = parse_key_value(&manifest_contents, "installdir")
                .unwrap_or_else(|| DEFAULT_INSTALL_DIR.to_owned());
            let install_dir = steamapps.join("common").join(install_name);
            let cfg_dir = install_dir.join("game/csgo/cfg");
            if !cfg_dir.is_dir() {
                return Err(InstallError::ConfigDirectoryMissing(cfg_dir));
            }
            return Ok(Cs2Install {
                steam_library: library,
                install_dir,
                cfg_dir,
            });
        }
        Err(InstallError::Cs2NotFound)
    }
}

/// Writes only `OpenClip`'s own GSI configuration and is idempotent when content is current.
///
/// # Errors
///
/// Returns a scoped filesystem error when the existing file cannot be read or the config cannot
/// be written.
pub fn install_gsi_config(
    install: &Cs2Install,
    endpoint: GsiEndpoint,
) -> Result<ConfigInstallResult, InstallError> {
    if !install.cfg_dir.is_dir() {
        return Err(InstallError::ConfigDirectoryMissing(
            install.cfg_dir.clone(),
        ));
    }
    let path = install.cfg_dir.join(GSI_CONFIG_FILE_NAME);
    let desired = render_gsi_config(endpoint);
    let action = if path.is_file() {
        let current = fs::read_to_string(&path).map_err(|source| InstallError::Read {
            path: path.clone(),
            source,
        })?;
        if current == desired {
            ConfigInstallAction::AlreadyCurrent
        } else {
            fs::write(&path, desired).map_err(|source| InstallError::Write {
                path: path.clone(),
                source,
            })?;
            ConfigInstallAction::Updated
        }
    } else {
        fs::write(&path, desired).map_err(|source| InstallError::Write {
            path: path.clone(),
            source,
        })?;
        ConfigInstallAction::Installed
    };
    Ok(ConfigInstallResult { path, action })
}

#[must_use]
pub fn render_gsi_config(endpoint: GsiEndpoint) -> String {
    format!(
        "\"OpenClip\"\n{{\n    \"uri\"       \"http://127.0.0.1:{}/\"\n    \"timeout\"   \"5.0\"\n    \"buffer\"    \"0.1\"\n    \"throttle\"  \"0.1\"\n    \"heartbeat\" \"10.0\"\n\n    \"data\"\n    {{\n        \"provider\"           \"1\"\n        \"map\"                \"1\"\n        \"round\"              \"1\"\n        \"player_id\"          \"1\"\n        \"player_state\"       \"1\"\n        \"player_weapons\"     \"1\"\n        \"player_match_stats\" \"1\"\n    }}\n}}\n",
        endpoint.port
    )
}

fn parse_library_paths(contents: &str) -> Vec<PathBuf> {
    contents
        .lines()
        .filter_map(|line| value_after_key(line, "path"))
        .map(|value| PathBuf::from(value.replace("\\\\", "\\")))
        .collect()
}

fn parse_key_value(contents: &str, requested_key: &str) -> Option<String> {
    contents
        .lines()
        .find_map(|line| value_after_key(line, requested_key))
}

fn value_after_key(line: &str, requested_key: &str) -> Option<String> {
    let values = line
        .split('"')
        .enumerate()
        .filter_map(|(index, value)| (index % 2 == 1).then_some(value.trim()))
        .collect::<Vec<_>>();
    values
        .windows(2)
        .find_map(|pair| (pair[0] == requested_key).then(|| pair[1].to_owned()))
}

fn add_unique_path(paths: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>, path: PathBuf) {
    if seen.insert(path.clone()) {
        paths.push(path);
    }
}

fn steam_root_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = env::var_os("OPENCLIP_STEAM_DIR") {
        candidates.push(PathBuf::from(path));
    }
    candidates.extend(registry_steam_paths());
    if let Some(path) = env::var_os("PROGRAMFILES(X86)") {
        candidates.push(PathBuf::from(path).join("Steam"));
    }
    if let Some(path) = env::var_os("PROGRAMFILES") {
        candidates.push(PathBuf::from(path).join("Steam"));
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

#[cfg(windows)]
fn registry_steam_paths() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let mut paths = Vec::new();
    let current_user = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok(key) = current_user.open_subkey("Software\\Valve\\Steam")
        && let Ok::<String, _>(path) = key.get_value("SteamPath")
    {
        paths.push(PathBuf::from(path));
    }
    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    if let Ok(key) = local_machine.open_subkey("SOFTWARE\\WOW6432Node\\Valve\\Steam")
        && let Ok::<String, _>(path) = key.get_value("InstallPath")
    {
        paths.push(PathBuf::from(path));
    }
    paths
}

#[cfg(not(windows))]
fn registry_steam_paths() -> Vec<PathBuf> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_modern_library_folder_paths() {
        let vdf = r#"
        "libraryfolders"
        {
            "0" { "path" "C:\\Program Files (x86)\\Steam" }
            "1" { "path" "D:\\SteamLibrary" "apps" { "730" "1" } }
        }
        "#;
        assert_eq!(
            parse_library_paths(vdf),
            vec![
                PathBuf::from(r"C:\Program Files (x86)\Steam"),
                PathBuf::from(r"D:\SteamLibrary")
            ]
        );
    }

    #[test]
    fn config_is_loopback_only() {
        let config = render_gsi_config(GsiEndpoint { port: 21_987 });
        assert!(config.contains("http://127.0.0.1:21987/"));
        assert!(!config.contains("0.0.0.0"));
    }

    #[test]
    fn installation_is_idempotent_and_updates_only_openclip_file() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let cfg_dir = temporary.path().join("game/csgo/cfg");
        fs::create_dir_all(&cfg_dir).expect("config directory");
        let unrelated = cfg_dir.join("autoexec.cfg");
        fs::write(&unrelated, "user settings").expect("unrelated config");
        let install = Cs2Install {
            steam_library: temporary.path().to_owned(),
            install_dir: temporary.path().to_owned(),
            cfg_dir,
        };
        let first = install_gsi_config(&install, GsiEndpoint::default()).expect("first install");
        let second = install_gsi_config(&install, GsiEndpoint::default()).expect("second install");
        assert_eq!(first.action, ConfigInstallAction::Installed);
        assert_eq!(second.action, ConfigInstallAction::AlreadyCurrent);
        assert_eq!(fs::read_to_string(unrelated).unwrap(), "user settings");
    }
}
