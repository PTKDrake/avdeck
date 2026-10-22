//! Avdeck's settings, `%APPDATA%\avdeck\config.toml`. Missing keys take their defaults, so the
//! file only holds what the user changed.

use crate::launch;
use crate::sdk::Overrides;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};

/// What closing an emulator tab, or the whole app, does to the emulator.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CloseAction {
    #[default]
    Ask,
    /// Stop it (saving the quick-boot snapshot unless disabled).
    Stop,
    /// Leave it running headless; Avdeck attaches again later.
    KeepRunning,
}

/// Light or dark look.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    /// Follow the Windows setting.
    #[default]
    System,
    Light,
    Dark,
}

/// Where the window was, in points of the desktop.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowPosition {
    pub x: f32,
    pub y: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// SDK and tool paths set in the settings screen.
    pub tools: Overrides,
    /// Default options for starting an emulator.
    pub launch: launch::Options,
    pub close_tab: CloseAction,
    pub close_app: CloseAction,
    pub appearance: Appearance,
    pub window: Option<WindowPosition>,
    /// AVD ids open as tabs when Avdeck closed, reopened (attached) at start.
    pub tabs: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tools: Overrides::default(),
            // Quick-boot snapshots record `-gpu host` and refuse to load without it, and
            // `start-emulator.ps1` always used it.
            launch: launch::Options {
                gpu: Some("host".to_owned()),
                ..Default::default()
            },
            close_tab: CloseAction::default(),
            close_app: CloseAction::default(),
            appearance: Appearance::default(),
            window: None,
            tabs: Vec::new(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("avdeck").join("config.toml"))
    }

    /// The saved settings; defaults when there is no file or it cannot be read.
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| Self::load_from(&path).ok())
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> io::Result<Self> {
        toml::from_str(&std::fs::read_to_string(path)?).map_err(io::Error::other)
    }

    pub fn save(&self) -> io::Result<()> {
        self.save_to(&Self::path().ok_or_else(|| io::Error::other("APPDATA is not set"))?)
    }

    pub fn save_to(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        // Write then rename, so a crash never leaves half a file.
        let temporary = path.with_extension("toml.tmp");
        std::fs::write(&temporary, text)?;
        std::fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_file() {
        let path = std::env::temp_dir()
            .join(format!("avdeck-config-{}", std::process::id()))
            .join("config.toml");
        let config = Config {
            tools: Overrides {
                sdk: Some(r"D:\Android\SDK".into()),
                ..Default::default()
            },
            launch: launch::Options {
                gpu: Some("host".to_owned()),
                ..Default::default()
            },
            close_app: CloseAction::KeepRunning,
            appearance: Appearance::Dark,
            window: Some(WindowPosition { x: 640.0, y: 80.5 }),
            tabs: vec!["Pixel_10_Pro_API_37_2".to_owned()],
            ..Default::default()
        };
        config.save_to(&path).unwrap();
        assert_eq!(Config::load_from(&path).unwrap(), config);
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn missing_keys_take_defaults() {
        let config: Config =
            toml::from_str("close_tab = \"stop\"\n[tools]\njdk = 'C:\\jdk'\n").unwrap();
        assert_eq!(config.close_tab, CloseAction::Stop);
        assert_eq!(config.close_app, CloseAction::Ask);
        assert_eq!(config.appearance, Appearance::System);
        assert_eq!(config.tools.jdk, Some(PathBuf::from(r"C:\jdk")));
        assert!(config.tabs.is_empty());
        assert_eq!(config.launch.gpu.as_deref(), Some("host"));
        // Written by an older Avdeck: a launch table without the GPU mode.
        let config: Config = toml::from_str("[launch]\ncold_boot = false\n").unwrap();
        assert_eq!(config.launch.gpu.as_deref(), Some("host"));
    }
}
