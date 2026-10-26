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

/// The window when Avdeck last closed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Outer bounds, left, top, right and bottom, in physical pixels of the virtual desktop
    /// (monitors at any DPI share it).
    pub bounds: [i32; 4],
    /// Client size of the Devices and Settings pages, in points.
    pub page_size: Option<[f32; 2]>,
    /// Longer side of the last device screen shown, in points.
    pub screen_side: Option<f32>,
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
    /// Replaces `window` (a position in points, wrong across monitors of different DPI), which
    /// older versions wrote and this one ignores.
    pub window_state: Option<WindowState>,
    /// AVD ids open as tabs when Avdeck closed, reopened (attached) at start.
    pub tabs: Vec<String>,
    /// The first-run setup was finished or skipped.
    pub setup_done: bool,
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
            window_state: None,
            tabs: Vec::new(),
            setup_done: false,
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
            window_state: Some(WindowState {
                bounds: [-1700, 600, -900, 1500],
                page_size: Some([960.0, 640.5]),
                screen_side: None,
            }),
            tabs: vec!["Pixel_10_Pro_API_37_2".to_owned()],
            setup_done: true,
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
        assert!(!config.setup_done);
        assert_eq!(config.launch.gpu.as_deref(), Some("host"));
        // Written by an older Avdeck: a launch table without the GPU mode.
        let config: Config = toml::from_str("[launch]\ncold_boot = false\n").unwrap();
        assert_eq!(config.launch.gpu.as_deref(), Some("host"));
        // An older Avdeck saved the window position in points: dropped.
        let config: Config = toml::from_str("[window]\nx = -1171.0\ny = 609.0\n").unwrap();
        assert_eq!(config.window_state, None);
    }
}
