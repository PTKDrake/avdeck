//! System images: the installed ones (read from the SDK folder, fast) and what `sdkmanager`
//! offers (parsed from `sdkmanager --list`).

use std::path::Path;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemImage {
    /// sdkmanager package path, e.g. `system-images;android-37.2;google_apis_playstore_ps16k;x86_64`.
    pub package: String,
    /// `Google Play Intel x86_64 Atom System Image`, when known.
    pub description: String,
    pub installed: bool,
}

impl SystemImage {
    /// The API level part of the package (`android-37.2` -> `37.2`).
    pub fn api(&self) -> &str {
        let platform = self.package.split(';').nth(1).unwrap_or_default();
        platform.strip_prefix("android-").unwrap_or(platform)
    }

    pub fn tag(&self) -> &str {
        self.package.split(';').nth(2).unwrap_or_default()
    }

    /// The tag in words: `google_apis_playstore_ps16k` -> `Google Play, 16 KB pages`.
    pub fn tag_title(&self) -> String {
        let tag = self.tag();
        let (base, pages) = match tag.strip_suffix("_ps16k") {
            Some(base) => (base, ", 16 KB pages"),
            None => (tag, ""),
        };
        let title = match base {
            "google_apis_playstore" => "Google Play",
            "google_apis" => "Google APIs",
            "default" => "AOSP",
            "google_atd" => "Google ATD",
            "aosp_atd" => "AOSP ATD",
            "wear" => "Wear OS",
            other if other.starts_with("android-wear") => "Wear OS",
            "android-tv" => "Android TV",
            "google-tv" => "Google TV",
            "android-desktop" => "Desktop",
            other if other.starts_with("android-automotive") => "Automotive",
            other => other,
        };
        format!("{title}{pages}")
    }

    /// One line for lists: `API 37.2 · Google Play, 16 KB pages · x86_64`.
    pub fn title(&self) -> String {
        format!(
            "API {}  ·  {}  ·  {}",
            self.api(),
            self.tag_title(),
            self.abi()
        )
    }

    pub fn abi(&self) -> &str {
        self.package.split(';').nth(3).unwrap_or_default()
    }

    /// `image.sysdir.1` for this image, relative to the SDK.
    pub fn sysdir(&self) -> String {
        let mut dir = self.package.replace(';', "\\");
        dir.push('\\');
        dir
    }
}

/// Images under `<sdk>\system-images\<platform>\<tag>\<abi>` that have a `source.properties`.
pub fn installed(sdk: &Path) -> Vec<SystemImage> {
    let root = sdk.join("system-images");
    let mut images = Vec::new();
    for platform in subdirs(&root) {
        for tag in subdirs(&root.join(&platform)) {
            for abi in subdirs(&root.join(&platform).join(&tag)) {
                let dir = root.join(&platform).join(&tag).join(&abi);
                let Ok(properties) = std::fs::read_to_string(dir.join("source.properties")) else {
                    continue;
                };
                let properties = crate::ini::parse(&properties);
                images.push(SystemImage {
                    package: format!("system-images;{platform};{tag};{abi}"),
                    description: properties
                        .get("Pkg.Desc")
                        .cloned()
                        .unwrap_or_else(|| format!("{tag} {abi}")),
                    installed: true,
                });
            }
        }
    }
    sort(&mut images);
    images
}

fn subdirs(path: &Path) -> Vec<String> {
    std::fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Newest API first, then by package.
fn sort(images: &mut [SystemImage]) {
    let key = |image: &SystemImage| -> Vec<u64> {
        image
            .api()
            .split('.')
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    images.sort_by(|a, b| key(b).cmp(&key(a)).then_with(|| a.package.cmp(&b.package)));
}

/// System images this machine can run (x86 and x86_64) in the available table of
/// `sdkmanager --list`, minus `installed`. Two formats exist:
/// `  system-images;android-36;google_apis;x86_64 | 7 | Google APIs ... Image` (older
/// cmdline-tools) and `  system-images/android-36/google_apis/x86_64   7.0.0   Google APIs ...`
/// (cmdline-tools 23, under "Available packages:").
pub fn available(sdkmanager_list: &str, installed: &[SystemImage]) -> Vec<SystemImage> {
    let mut in_available = false;
    let mut images: Vec<SystemImage> = Vec::new();
    for line in sdkmanager_list.lines() {
        let trimmed = line.trim();
        // Section titles are not indented and end with a colon.
        if !line.starts_with(' ') && trimmed.ends_with(':') {
            in_available = trimmed.eq_ignore_ascii_case("Available Packages:");
            continue;
        }
        if !in_available {
            continue;
        }
        let columns: Vec<&str> = if trimmed.contains('|') {
            trimmed.split('|').map(str::trim).collect()
        } else {
            trimmed
                .split("  ")
                .map(str::trim)
                .filter(|column| !column.is_empty())
                .collect()
        };
        let Some(path) = columns.first() else {
            continue;
        };
        let package = path.replace('/', ";");
        if !package.starts_with("system-images;") {
            continue;
        }
        let image = SystemImage {
            description: columns.get(2).copied().unwrap_or_default().to_owned(),
            package,
            installed: false,
        };
        if !matches!(image.abi(), "x86_64" | "x86")
            || installed.iter().any(|done| done.package == image.package)
            || images.iter().any(|known| known.package == image.package)
        {
            continue;
        }
        images.push(image);
    }
    sort(&mut images);
    images
}

/// Install progress from an `sdkmanager` output chunk, e.g. `[=====    ] 42% Downloading ...`.
pub fn progress(output: &str) -> Option<(u8, String)> {
    let line = output
        .rsplit(['\r', '\n'])
        .find(|line| line.contains('%'))?;
    let percent_at = line.find('%')?;
    let digits: String = line[..percent_at]
        .chars()
        .rev()
        .take_while(char::is_ascii_digit)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let percent = digits.parse().ok()?;
    Some((percent, line[percent_at + 1..].trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trimmed `sdkmanager --list` output (cmdline-tools 19).
    const LIST: &str = "\
Installed packages:
  Path                                                          | Version | Description                                | Location
  -------                                                       | ------- | -------                                    | -------
  system-images;android-37.2;google_apis_playstore_ps16k;x86_64 | 1       | Google Play Intel x86_64 Atom System Image | system-images\\android-37.2\\google_apis_playstore_ps16k\\x86_64

Available Packages:
  Path                                                    | Version | Description
  -------                                                 | ------- | -------
  platforms;android-36                                    | 2       | Android SDK Platform 36
  system-images;android-36;google_apis;x86_64             | 7       | Google APIs Intel x86_64 Atom System Image
  system-images;android-37.2;google_apis_playstore_ps16k;x86_64 | 1 | Google Play Intel x86_64 Atom System Image
  system-images;android-9;default;x86                     | 4       | Intel x86 Atom System Image

Available Updates:
  ID                        | Installed | Available
  system-images;android-9;default;x86 | 3 | 4
";

    fn installed_pixel() -> Vec<SystemImage> {
        vec![SystemImage {
            package: "system-images;android-37.2;google_apis_playstore_ps16k;x86_64".to_owned(),
            description: String::new(),
            installed: true,
        }]
    }

    /// Trimmed `sdkmanager --list` from cmdline-tools 23 (Android CLI).
    const LIST_23: &str = "\
WARNING: The SDK Manager CLI tool (sdkmanager) is deprecated. Android CLI will be used instead.

Installed packages:
  emulator                                                    37.2.12   Android Emulator
  system-images/android-37.2/google_apis_playstore_ps16k/x86_64   6.0.0   16 KB Page Size Google Play Intel x86_64 Atom System Image
Available packages:
  sources/android-37.2                                        1.0.0     Sources for Android 37.2
  system-images/android-10/default/armeabi-v7a                5.0.0     ARM EABI v7a System Image
  system-images/android-10/default/x86                        5.0.0     Intel x86 Atom System Image
  system-images/android-36/google_apis/x86_64                 7.0.0     Google APIs Intel x86_64 Atom System Image
  system-images/android-37.2/google_apis_playstore_ps16k/x86_64   6.0.0   16 KB Page Size Google Play Intel x86_64 Atom System Image
";

    #[test]
    fn reads_the_cmdline_tools_23_format() {
        let images = available(LIST_23, &installed_pixel());
        let packages: Vec<&str> = images.iter().map(|image| image.package.as_str()).collect();
        // ARM images and the installed one are left out; newest first.
        assert_eq!(
            packages,
            [
                "system-images;android-36;google_apis;x86_64",
                "system-images;android-10;default;x86"
            ]
        );
        assert_eq!(
            images[0].description,
            "Google APIs Intel x86_64 Atom System Image"
        );
    }

    #[test]
    fn lists_available_images_newest_first_without_installed_ones() {
        let images = available(LIST, &installed_pixel());
        let packages: Vec<&str> = images.iter().map(|image| image.package.as_str()).collect();
        assert_eq!(
            packages,
            [
                "system-images;android-36;google_apis;x86_64",
                "system-images;android-9;default;x86"
            ]
        );
        assert_eq!(
            images[0].description,
            "Google APIs Intel x86_64 Atom System Image"
        );
        assert!(!images[0].installed);
    }

    #[test]
    fn package_parts_and_sysdir() {
        let image = &installed_pixel()[0];
        assert_eq!(image.api(), "37.2");
        assert_eq!(image.tag(), "google_apis_playstore_ps16k");
        assert_eq!(image.abi(), "x86_64");
        assert_eq!(image.tag_title(), "Google Play, 16 KB pages");
        let wear = SystemImage {
            package: "system-images;android-37.0;android-wear-signed;x86_64".to_owned(),
            description: String::new(),
            installed: false,
        };
        assert_eq!(wear.tag_title(), "Wear OS");
        assert_eq!(
            image.title(),
            "API 37.2  ·  Google Play, 16 KB pages  ·  x86_64"
        );
        assert_eq!(
            image.sysdir(),
            "system-images\\android-37.2\\google_apis_playstore_ps16k\\x86_64\\"
        );
    }

    #[test]
    fn reads_install_progress() {
        let chunk = "[=====                                  ] 13% Downloading x86_64-36_r07.zip...\r\
                     [==========                             ] 27% Downloading x86_64-36_r07.zip...";
        assert_eq!(
            progress(chunk),
            Some((27, "Downloading x86_64-36_r07.zip...".to_owned()))
        );
        assert_eq!(progress("Loading package information..."), None);
    }

    #[test]
    fn finds_installed_images_on_disk() {
        let sdk = std::env::temp_dir().join(format!("avdeck-images-{}", std::process::id()));
        let dir = sdk.join("system-images/android-36/google_apis/x86_64");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("source.properties"),
            "Pkg.Desc=Google APIs Image\n",
        )
        .unwrap();
        std::fs::create_dir_all(sdk.join("system-images/android-35/broken/x86_64")).unwrap();
        let images = installed(&sdk);
        assert_eq!(images.len(), 1);
        assert_eq!(
            images[0].package,
            "system-images;android-36;google_apis;x86_64"
        );
        assert_eq!(images[0].description, "Google APIs Image");
        std::fs::remove_dir_all(&sdk).unwrap();
    }
}
