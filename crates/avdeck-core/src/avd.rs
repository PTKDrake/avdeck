//! Reads AVD definitions: where they live and each device's native display size.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn read_ini(path: &Path) -> Option<HashMap<String, String>> {
    Some(crate::ini::parse(&fs::read_to_string(path).ok()?))
}

/// Directories that may hold `<name>.ini`, in the order the emulator looks at them.
pub fn avd_homes(explicit: Option<&Path>) -> Vec<PathBuf> {
    let mut homes = Vec::new();
    if let Some(path) = explicit {
        homes.push(path.to_owned());
    }
    if let Some(path) = std::env::var_os("ANDROID_AVD_HOME") {
        homes.push(path.into());
    }
    if let Some(path) = std::env::var_os("ANDROID_USER_HOME") {
        homes.push(PathBuf::from(path).join("avd"));
    }
    if let Some(path) = std::env::var_os("USERPROFILE") {
        homes.push(PathBuf::from(path).join(".android").join("avd"));
    }
    homes
}

/// `hw.lcd.width` x `hw.lcd.height` from the AVD's `config.ini`.
pub fn panel_size(homes: &[PathBuf], name: &str) -> Option<(i32, i32)> {
    homes.iter().find_map(|home| {
        let pointer = read_ini(&home.join(format!("{name}.ini")))?;
        // The .ini points at the .avd folder; fall back to the default layout if it was moved.
        let directory = pointer
            .get("path")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .unwrap_or_else(|| home.join(format!("{name}.avd")));
        let config = read_ini(&directory.join("config.ini"))?;
        let width = config.get("hw.lcd.width")?.parse().ok()?;
        let height = config.get("hw.lcd.height")?.parse().ok()?;
        Some((width, height))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_panel_size_through_the_pointer_file() {
        let home = std::env::temp_dir().join(format!("avdeck-avd-test-{}", std::process::id()));
        let moved = home.join("elsewhere");
        fs::create_dir_all(&moved).unwrap();
        fs::write(
            home.join("Pixel.ini"),
            format!("avd.ini.encoding=UTF-8\npath={}\n", moved.display()),
        )
        .unwrap();
        fs::write(
            moved.join("config.ini"),
            "# comment\nhw.lcd.width = 1280\nhw.lcd.height=2856\n",
        )
        .unwrap();

        let homes = [PathBuf::from("Z:\\missing"), home.clone()];
        assert_eq!(panel_size(&homes, "Pixel"), Some((1280, 2856)));
        assert_eq!(panel_size(&homes, "Tablet"), None);
        fs::remove_dir_all(&home).unwrap();
    }
}
