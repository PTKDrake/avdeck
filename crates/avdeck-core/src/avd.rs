//! AVD definitions in an AVD home: `<id>.ini` points at `<id>.avd\config.ini`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

fn read_ini(path: &Path) -> Option<HashMap<String, String>> {
    Some(crate::ini::parse(&fs::read_to_string(path).ok()?))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avd {
    /// Name passed to `-avd`, e.g. `Pixel_10_Pro_API_37_2`.
    pub id: String,
    /// `avd.ini.displayname`, or the id.
    pub name: String,
    pub dir: PathBuf,
    /// `hw.device.manufacturer` + `hw.device.name`, e.g. `Google pixel_10_pro`.
    pub device: Option<String>,
    /// `hw.lcd.width` x `hw.lcd.height`, natural orientation.
    pub panel: Option<(u32, u32)>,
    pub density: Option<u32>,
    /// API level from `target` (`android-37.2` -> `37.2`).
    pub api: Option<String>,
    pub abi: Option<String>,
    /// `tag.id`, e.g. `google_apis_playstore`.
    pub tag: Option<String>,
    pub ram_mb: Option<u32>,
    /// `disk.dataPartition.size`, e.g. `30G`.
    pub data_size: Option<String>,
    pub play_store: bool,
    /// Has a hinge sensor: the toolbar gets fold/unfold.
    pub foldable: bool,
}

impl Avd {
    /// Reads `<home>\<id>.ini` and the `config.ini` it points at.
    pub fn read(home: &Path, id: &str) -> Option<Self> {
        let pointer = read_ini(&home.join(format!("{id}.ini")))?;
        // The .ini points at the .avd folder; fall back to the default layout if it was moved.
        let dir = pointer
            .get("path")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .unwrap_or_else(|| home.join(format!("{id}.avd")));
        let config = read_ini(&dir.join("config.ini"))?;
        let number = |key: &str| config.get(key).and_then(|value| value.parse().ok());
        let text = |key: &str| config.get(key).filter(|value| !value.is_empty()).cloned();
        let yes = |key: &str| {
            config
                .get(key)
                .is_some_and(|value| value == "yes" || value == "true")
        };
        let device = match (text("hw.device.manufacturer"), text("hw.device.name")) {
            (Some(maker), Some(name)) => Some(format!("{maker} {name}")),
            (maker, name) => name.or(maker),
        };
        let target = text("target").or_else(|| pointer.get("target").cloned());
        Some(Self {
            id: id.to_owned(),
            name: text("avd.ini.displayname").unwrap_or_else(|| id.to_owned()),
            device,
            panel: number("hw.lcd.width").zip(number("hw.lcd.height")),
            density: number("hw.lcd.density"),
            api: target.as_deref().and_then(api_level),
            abi: text("abi.type"),
            tag: text("tag.id"),
            ram_mb: number("hw.ramSize"),
            data_size: text("disk.dataPartition.size"),
            play_store: yes("PlayStore.enabled"),
            foldable: yes("hw.sensor.hinge"),
            dir,
        })
    }
}

/// `android-37.2` -> `37.2`; `android-VanillaIceCream` stays as the codename.
pub fn api_level(target: &str) -> Option<String> {
    let rest = target.strip_prefix("android-").unwrap_or(target);
    Some(rest.to_owned()).filter(|rest| !rest.is_empty())
}

/// Every AVD in `home`, sorted by display name.
pub fn list(home: &Path) -> Vec<Avd> {
    let Ok(entries) = fs::read_dir(home) else {
        return Vec::new();
    };
    let mut avds: Vec<Avd> = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let id = name.strip_suffix(".ini")?;
            Avd::read(home, id)
        })
        .collect();
    avds.sort_by_key(|avd| avd.name.to_lowercase());
    avds
}

/// Every key of the AVD's `config.ini`.
pub fn values(avd: &Avd) -> HashMap<String, String> {
    read_ini(&avd.dir.join("config.ini")).unwrap_or_default()
}

/// `config.ini` with `values` set: existing keys are replaced in place, new ones appended, every
/// other line (comments included) kept as it was.
pub fn set_values(config: &str, values: &[(&str, String)]) -> String {
    let mut pending: Vec<&(&str, String)> = values.iter().collect();
    let mut lines: Vec<String> = config
        .lines()
        .map(|line| {
            let key = line.split_once('=').map(|(key, _)| key.trim());
            match pending.iter().position(|(name, _)| Some(*name) == key) {
                Some(index) => {
                    let (name, value) = pending.remove(index);
                    format!("{name}={value}")
                }
                None => line.to_owned(),
            }
        })
        .collect();
    lines.extend(
        pending
            .into_iter()
            .map(|(name, value)| format!("{name}={value}")),
    );
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Writes `values` into the AVD's `config.ini`.
pub fn write_values(avd: &Avd, values: &[(&str, String)]) -> std::io::Result<()> {
    let path = avd.dir.join("config.ini");
    let config = fs::read_to_string(&path)?;
    fs::write(path, set_values(&config, values))
}

/// An AVD id from a display name: letters, digits, `.`, `_` and `-`; the rest becomes `_`.
pub fn id_from_name(name: &str) -> String {
    let id: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    id.trim_matches('_').to_owned()
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Removes the AVD's folder and its pointer file.
pub fn delete(home: &Path, avd: &Avd) -> std::io::Result<()> {
    if avd.dir.is_dir() {
        fs::remove_dir_all(&avd.dir)?;
    }
    let pointer = home.join(format!("{}.ini", avd.id));
    if pointer.exists() {
        fs::remove_file(pointer)?;
    }
    Ok(())
}

/// Files a copy leaves out: locks and runtime files of a running emulator, quick-boot snapshots
/// and qcow2 overlays (they point at the original's images by absolute path). The copy keeps
/// the configuration and the user data image, and cold boots the first time.
fn skip_in_copy(name: &str) -> bool {
    name.ends_with(".lock")
        || name.ends_with(".qcow2")
        || matches!(
            name,
            "snapshots"
                | "hardware-qemu.ini"
                | "emu-launch-params.txt"
                | "read-snapshot.txt"
                | "quickbootChoice.ini"
                | "tmpAdbCmds"
                | "bootcompleted.ini"
        )
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        if skip_in_copy(&name.to_string_lossy()) {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Copies `avd` to a new AVD `id` named `name` in `home`.
pub fn duplicate(home: &Path, avd: &Avd, id: &str, name: &str) -> std::io::Result<Avd> {
    let dir = home.join(format!("{id}.avd"));
    if dir.exists() || home.join(format!("{id}.ini")).exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("an AVD named {id} already exists"),
        ));
    }
    copy_dir(&avd.dir, &dir)?;
    let pointer = fs::read_to_string(home.join(format!("{}.ini", avd.id))).unwrap_or_default();
    let path = dir.display().to_string();
    fs::write(
        home.join(format!("{id}.ini")),
        set_values(
            &pointer,
            &[("path", path), ("path.rel", format!("avd\\{id}.avd"))],
        ),
    )?;
    let config = fs::read_to_string(dir.join("config.ini"))?;
    fs::write(
        dir.join("config.ini"),
        set_values(
            &config,
            &[
                ("avd.ini.displayname", name.to_owned()),
                ("AvdId", id.to_owned()),
            ],
        ),
    )?;
    Avd::read(home, id).ok_or_else(|| std::io::Error::other("the copy cannot be read"))
}

/// A hardware profile `avdmanager create avd -d` accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceProfile {
    pub id: String,
    /// `Pixel 10 Pro`, `Wear OS Large Round`, ...
    pub name: String,
    pub oem: String,
    /// `android-wear`, `android-tv`, ...; none for phones and tablets.
    pub tag: Option<String>,
}

/// The groups the device list is shown in, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeviceGroup {
    Pixel,
    PhonesAndTablets,
    WearOs,
    Tv,
    Automotive,
    Desktop,
    XrAndGlasses,
    Older,
}

impl DeviceGroup {
    pub fn title(self) -> &'static str {
        match self {
            Self::Pixel => "Pixel",
            Self::PhonesAndTablets => "Phones and tablets",
            Self::WearOs => "Wear OS",
            Self::Tv => "TV",
            Self::Automotive => "Automotive",
            Self::Desktop => "Desktop",
            Self::XrAndGlasses => "XR and glasses",
            Self::Older => "Older devices",
        }
    }
}

impl DeviceProfile {
    pub fn group(&self) -> DeviceGroup {
        let tag = self.tag.as_deref().unwrap_or_default();
        if tag.starts_with("android-wear") {
            DeviceGroup::WearOs
        } else if tag.starts_with("android-tv") {
            DeviceGroup::Tv
        } else if tag.starts_with("android-automotive") {
            DeviceGroup::Automotive
        } else if tag.starts_with("android-desktop") {
            DeviceGroup::Desktop
        } else if tag.starts_with("android-xr") || tag.contains("glasses") {
            DeviceGroup::XrAndGlasses
        } else if self.id == "pixel" || self.id.starts_with("pixel_") {
            DeviceGroup::Pixel
        } else if self.id.starts_with("Nexus") || self.id.contains("in ") || self.oem.is_empty() {
            DeviceGroup::Older
        } else {
            DeviceGroup::PhonesAndTablets
        }
    }

    /// The Pixel generation (`pixel_10_pro` -> 10), for newest-first ordering.
    fn generation(&self) -> u32 {
        self.id
            .strip_prefix("pixel_")
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
                digits.parse().ok()
            })
            .unwrap_or(0)
    }
}

/// Profiles from `avdmanager list device` (blocks of `id: N or "id"`, `Name:`, `OEM :`, `Tag :`).
pub fn device_profiles(output: &str) -> Vec<DeviceProfile> {
    let mut profiles: Vec<DeviceProfile> = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("id:") {
            let Some(id) = rest.split('"').nth(1) else {
                continue;
            };
            profiles.push(DeviceProfile {
                id: id.to_owned(),
                name: id.to_owned(),
                oem: String::new(),
                tag: None,
            });
        } else if let Some(profile) = profiles.last_mut()
            && let Some((key, value)) = line.split_once(':')
        {
            let value = value.trim().to_owned();
            match key.trim() {
                "Name" => profile.name = value,
                "OEM" => profile.oem = value,
                "Tag" => profile.tag = Some(value).filter(|tag| !tag.is_empty()),
                _ => {}
            }
        }
    }
    profiles
}

/// Profiles by group: Pixels newest first, the rest by name.
pub fn grouped(profiles: &[DeviceProfile]) -> Vec<(DeviceGroup, Vec<DeviceProfile>)> {
    let mut groups: Vec<(DeviceGroup, Vec<DeviceProfile>)> = Vec::new();
    let mut sorted = profiles.to_vec();
    sorted.sort_by(|a, b| {
        a.group()
            .cmp(&b.group())
            .then_with(|| b.generation().cmp(&a.generation()))
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    for profile in sorted {
        match groups.last_mut() {
            Some((group, members)) if *group == profile.group() => members.push(profile),
            _ => groups.push((profile.group(), vec![profile])),
        }
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `config.ini` of the Pixel 10 Pro AVD on this machine, trimmed.
    const PIXEL_10_PRO: &str = "PlayStore.enabled=true
abi.type=x86_64
avd.ini.displayname=Pixel 10 Pro API 37 2
disk.dataPartition.size=30G
hw.device.manufacturer=Google
hw.device.name=pixel_10_pro
hw.lcd.density=480
hw.lcd.height=2856
hw.lcd.width=1280
hw.ramSize=4096
tag.id=google_apis_playstore
target=android-37.2
";

    fn home(name: &str) -> PathBuf {
        let home = std::env::temp_dir().join(format!("avdeck-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn reads_an_avd_through_its_pointer_file() {
        let home = home("avd-read");
        let moved = home.join("elsewhere");
        fs::create_dir_all(&moved).unwrap();
        fs::write(
            home.join("Pixel.ini"),
            format!("avd.ini.encoding=UTF-8\npath={}\n", moved.display()),
        )
        .unwrap();
        fs::write(moved.join("config.ini"), PIXEL_10_PRO).unwrap();

        let avd = Avd::read(&home, "Pixel").unwrap();
        assert_eq!(avd.name, "Pixel 10 Pro API 37 2");
        assert_eq!(avd.device.as_deref(), Some("Google pixel_10_pro"));
        assert_eq!(avd.panel, Some((1280, 2856)));
        assert_eq!(avd.density, Some(480));
        assert_eq!(avd.api.as_deref(), Some("37.2"));
        assert_eq!(avd.ram_mb, Some(4096));
        assert_eq!(avd.data_size.as_deref(), Some("30G"));
        assert!(avd.play_store);
        assert!(!avd.foldable);
        assert_eq!(avd.dir, moved);
        assert_eq!(Avd::read(&home, "Tablet"), None);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn lists_avds_by_name_and_skips_broken_ones() {
        let home = home("avd-list");
        for (id, name) in [("b", "Beta"), ("a", "alpha")] {
            fs::create_dir_all(home.join(format!("{id}.avd"))).unwrap();
            fs::write(home.join(format!("{id}.ini")), "avd.ini.encoding=UTF-8\n").unwrap();
            fs::write(
                home.join(format!("{id}.avd")).join("config.ini"),
                format!("avd.ini.displayname={name}\n"),
            )
            .unwrap();
        }
        // A pointer file without an AVD folder.
        fs::write(home.join("gone.ini"), "path=Z:\\gone\n").unwrap();
        let names: Vec<String> = list(&home).into_iter().map(|avd| avd.name).collect();
        assert_eq!(names, ["alpha", "Beta"]);
        fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn api_levels() {
        assert_eq!(api_level("android-37.2").as_deref(), Some("37.2"));
        assert_eq!(api_level("android-34").as_deref(), Some("34"));
        assert_eq!(api_level("android-").as_deref(), None);
    }

    #[test]
    fn sets_values_in_place_and_appends_new_ones() {
        let config = "# comment\nhw.ramSize=2048\nhw.lcd.width=1280\n";
        let updated = set_values(
            config,
            &[
                ("hw.ramSize", "4096".to_owned()),
                ("hw.cpu.ncore", "4".to_owned()),
            ],
        );
        assert_eq!(
            updated,
            "# comment\nhw.ramSize=4096\nhw.lcd.width=1280\nhw.cpu.ncore=4\n"
        );
    }

    #[test]
    fn ids_from_names() {
        assert_eq!(
            id_from_name("Pixel 10 Pro (API 37)"),
            "Pixel_10_Pro__API_37"
        );
        assert!(valid_id("Pixel_10_Pro_API_37_2"));
        assert!(!valid_id("Pixel 10"));
        assert!(!valid_id(""));
    }

    #[test]
    fn duplicates_without_runtime_files() {
        let home = home("avd-copy");
        let dir = home.join("P.avd");
        fs::create_dir_all(dir.join("snapshots").join("default_boot")).unwrap();
        fs::create_dir_all(dir.join("data")).unwrap();
        fs::write(dir.join("config.ini"), PIXEL_10_PRO).unwrap();
        fs::write(dir.join("userdata-qemu.img"), "data").unwrap();
        fs::write(dir.join("userdata-qemu.img.qcow2"), "overlay").unwrap();
        fs::write(dir.join("multiinstance.lock"), "").unwrap();
        fs::write(dir.join("data").join("misc"), "x").unwrap();
        fs::write(
            home.join("P.ini"),
            format!(
                "avd.ini.encoding=UTF-8\npath={}\ntarget=android-37.2\n",
                dir.display()
            ),
        )
        .unwrap();
        let source = Avd::read(&home, "P").unwrap();

        let copy = duplicate(&home, &source, "Q", "Pixel copy").unwrap();
        assert_eq!(copy.name, "Pixel copy");
        assert_eq!(copy.dir, home.join("Q.avd"));
        assert_eq!(copy.panel, Some((1280, 2856)));
        assert!(copy.dir.join("userdata-qemu.img").exists());
        assert!(copy.dir.join("data").join("misc").exists());
        assert!(!copy.dir.join("userdata-qemu.img.qcow2").exists());
        assert!(!copy.dir.join("multiinstance.lock").exists());
        assert!(!copy.dir.join("snapshots").exists());
        assert!(duplicate(&home, &source, "Q", "again").is_err());

        delete(&home, &copy).unwrap();
        assert!(!home.join("Q.avd").exists() && !home.join("Q.ini").exists());
        fs::remove_dir_all(&home).unwrap();
    }

    /// Trimmed `avdmanager list device` from cmdline-tools 19.
    const DEVICES: &str = r#"Available devices definitions:
id: 0 or "ai_glasses_displayless"
    Name: Audio Glasses
    OEM : Google
    Tag : ai-glasses
---------
id: 1 or "automotive_1024p_landscape"
    Name: Automotive (1024p landscape)
    OEM : Google
    Tag : android-automotive-playstore
---------
id: 19 or "Nexus 5"
    Name: Nexus 5
    OEM : Google
---------
id: 29 or "pixel_10"
    Name: Pixel 10
    OEM : Google
---------
id: 30 or "pixel_10_pro"
    Name: Pixel 10 Pro
    OEM : Google
---------
id: 53 or "pixel_9"
    Name: Pixel 9
    OEM : Google
---------
id: 60 or "pixel_tablet"
    Name: Pixel Tablet
    OEM : Google
---------
id: 64 or "small_phone"
    Name: Small Phone
    OEM : Generic
---------
id: 69 or "wearos_large_round"
    Name: Wear OS Large Round
    OEM : Google
    Tag : android-wear
---------
id: 76 or "2.7in QVGA"
    Name: 2.7" QVGA
    OEM : Generic
"#;

    #[test]
    fn groups_device_profiles_with_pixels_newest_first() {
        let profiles = device_profiles(DEVICES);
        assert_eq!(profiles.len(), 10);
        assert_eq!(profiles[2].id, "Nexus 5");
        assert_eq!(profiles[3].name, "Pixel 10");
        assert_eq!(
            profiles[1].tag.as_deref(),
            Some("android-automotive-playstore")
        );
        let summary: Vec<(DeviceGroup, Vec<String>)> = grouped(&profiles)
            .into_iter()
            .map(|(group, members)| (group, members.into_iter().map(|p| p.id).collect()))
            .collect();
        let expected: Vec<(DeviceGroup, Vec<&str>)> = vec![
            (
                DeviceGroup::Pixel,
                vec!["pixel_10", "pixel_10_pro", "pixel_9", "pixel_tablet"],
            ),
            (DeviceGroup::PhonesAndTablets, vec!["small_phone"]),
            (DeviceGroup::WearOs, vec!["wearos_large_round"]),
            (DeviceGroup::Automotive, vec!["automotive_1024p_landscape"]),
            (DeviceGroup::XrAndGlasses, vec!["ai_glasses_displayless"]),
            (DeviceGroup::Older, vec!["2.7in QVGA", "Nexus 5"]),
        ];
        let expected: Vec<(DeviceGroup, Vec<String>)> = expected
            .into_iter()
            .map(|(group, ids)| (group, ids.into_iter().map(str::to_owned).collect()))
            .collect();
        assert_eq!(summary, expected);
    }
}
