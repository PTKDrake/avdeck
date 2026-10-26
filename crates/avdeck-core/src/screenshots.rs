//! Where screenshots are saved: the folder set in Avdeck's settings, else the emulator's own
//! "Screenshot save location" (Extended controls > Settings), else `Pictures\Avdeck`.

use crate::sdk::{Found, Source};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Where the emulator keeps its settings: Qt's `QSettings` in the registry, under
/// `HKEY_CURRENT_USER`.
const EMULATOR_KEY: &str = r"Software\Android Open Source Project\Emulator\set";
const EMULATOR_VALUE: &str = "savePath";

/// The folder for screenshots, from Avdeck's `setting`, the emulator's `emulator` setting and
/// `user_profile` (`%USERPROFILE%`). `None` only when none of them gives an absolute path.
pub fn resolve(
    setting: Option<&Path>,
    emulator: Option<&str>,
    user_profile: Option<OsString>,
) -> Option<Found> {
    if let Some(path) = setting.filter(|path| path.is_absolute()) {
        return Some(Found {
            path: path.to_owned(),
            source: Source::Settings,
            via: String::new(),
        });
    }
    if let Some(path) = emulator
        .map(|text| PathBuf::from(text.trim()))
        .filter(|path| path.is_absolute())
    {
        return Some(Found {
            path,
            source: Source::Default,
            via: "The emulator's setting".to_owned(),
        });
    }
    let path = PathBuf::from(user_profile?).join("Pictures").join("Avdeck");
    path.is_absolute().then(|| Found {
        path,
        source: Source::Default,
        via: r"%USERPROFILE%\Pictures\Avdeck".to_owned(),
    })
}

/// [`resolve`] with the emulator's setting from the registry and the real `%USERPROFILE%`.
pub fn folder(setting: Option<&Path>) -> Option<Found> {
    resolve(
        setting,
        emulator_setting().as_deref(),
        std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()),
    )
}

/// The emulator's "Screenshot save location", if it was ever set.
fn emulator_setting() -> Option<String> {
    use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, RRF_RT_REG_SZ, RegGetValueW};
    let wide = |text: &str| text.encode_utf16().chain([0]).collect::<Vec<u16>>();
    let (key, value) = (wide(EMULATOR_KEY), wide(EMULATOR_VALUE));
    let mut buffer = [0u16; 1024];
    let mut size = std::mem::size_of_val(&buffer) as u32;
    // SAFETY: the strings are NUL-terminated and `size` is the buffer's size in bytes.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    // `size` counts the terminating NUL.
    let length = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buffer[..length]))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROFILE: &str = r"C:\Users\kien1";

    #[test]
    fn the_setting_wins() {
        let found = resolve(
            Some(Path::new(r"D:\Shots")),
            Some(r"C:\Users\kien1\Pictures\Emulator"),
            Some(PROFILE.into()),
        )
        .unwrap();
        assert_eq!(found.path, PathBuf::from(r"D:\Shots"));
        assert_eq!(found.source, Source::Settings);
    }

    #[test]
    fn then_the_emulator_setting() {
        // As the emulator 37.2.12 wrote it on this machine.
        let found = resolve(
            None,
            Some(r"C:\Users\kien1\Pictures\Emulator"),
            Some(PROFILE.into()),
        )
        .unwrap();
        assert_eq!(
            found.path,
            PathBuf::from(r"C:\Users\kien1\Pictures\Emulator")
        );
        assert_eq!(found.via, "The emulator's setting");
    }

    #[test]
    fn then_pictures() {
        let found = resolve(None, None, Some(PROFILE.into())).unwrap();
        assert_eq!(found.path, PathBuf::from(r"C:\Users\kien1\Pictures\Avdeck"));
        // Empty or relative settings are skipped.
        let found = resolve(Some(Path::new("Shots")), Some(""), Some(PROFILE.into())).unwrap();
        assert_eq!(found.path, PathBuf::from(r"C:\Users\kien1\Pictures\Avdeck"));
        assert_eq!(resolve(None, None, None), None);
    }
}
