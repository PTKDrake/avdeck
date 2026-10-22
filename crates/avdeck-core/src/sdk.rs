//! Finds the Android SDK, its tools, the AVD home and a JDK (PLAN.md 3.6). Each item comes from
//! the app config first, then the environment, then the usual locations; the first valid one
//! wins. Avdeck passes the result to every child process, so the tools work even when the
//! system environment has none of these variables.

use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Paths set in Avdeck's settings; `None` means "find it".
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Overrides {
    pub sdk: Option<PathBuf>,
    /// `emulator.exe`.
    pub emulator: Option<PathBuf>,
    /// `adb.exe`.
    pub adb: Option<PathBuf>,
    /// Folder holding `avdmanager.bat` and `sdkmanager.bat`.
    pub cmdline_tools: Option<PathBuf>,
    pub avd_home: Option<PathBuf>,
    /// JDK home (with `bin\java.exe`).
    pub jdk: Option<PathBuf>,
}

/// Where a resolved path came from, shown in the settings screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Settings,
    Environment(&'static str),
    Path,
    Default,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub path: PathBuf,
    pub source: Source,
    /// How it was found, written with the variable it came from: `%ANDROID_HOME%`,
    /// `%ANDROID_HOME%\emulator\emulator.exe`, `%USERPROFILE%\.android\avd`, `PATH`. Empty
    /// when set in the settings.
    pub via: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tools {
    pub sdk: Option<Found>,
    pub emulator: Option<Found>,
    pub adb: Option<Found>,
    pub cmdline_tools: Option<Found>,
    pub avd_home: Option<Found>,
    pub jdk: Option<Found>,
}

/// What resolution may look at: environment variables and the file system.
pub trait Probe {
    fn var(&self, name: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
    fn is_dir(&self, path: &Path) -> bool;
    /// Names of the subfolders of `path`.
    fn subdirs(&self, path: &Path) -> Vec<String>;
}

/// The real environment and disk.
pub struct System;

impl Probe for System {
    fn var(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name).filter(|value| !value.is_empty())
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn subdirs(&self, path: &Path) -> Vec<String> {
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
}

fn found(path: PathBuf, source: Source, via: impl Into<String>) -> Found {
    Found {
        path,
        source,
        via: via.into(),
    }
}

/// The first `name` in a `PATH` folder.
fn on_path(probe: &impl Probe, name: &str) -> Option<PathBuf> {
    std::env::split_paths(&probe.var("PATH")?)
        .map(|dir| dir.join(name))
        .find(|file| probe.is_file(file))
}

fn is_sdk(probe: &impl Probe, dir: &Path) -> bool {
    probe.is_file(&dir.join("platform-tools").join("adb.exe"))
        || probe.is_file(&dir.join("emulator").join("emulator.exe"))
}

fn is_jdk(probe: &impl Probe, dir: &Path) -> bool {
    probe.is_file(&dir.join("bin").join("java.exe"))
}

fn is_cmdline_tools(probe: &impl Probe, dir: &Path) -> bool {
    probe.is_file(&dir.join("avdmanager.bat")) && probe.is_file(&dir.join("sdkmanager.bat"))
}

pub fn resolve(overrides: &Overrides, probe: &impl Probe) -> Tools {
    let sdk = resolve_sdk(overrides, probe);
    let sdk_dir = sdk.as_ref().map(|sdk| sdk.path.clone());
    // Tools inside the SDK are written relative to how the SDK was found.
    let sdk_via = sdk
        .as_ref()
        .map(|sdk| {
            if sdk.via.is_empty() {
                "SDK".to_owned()
            } else {
                sdk.via.clone()
            }
        })
        .unwrap_or_default();
    let via_sdk = |parts: &[&str]| format!("{sdk_via}\\{}", parts.join("\\"));
    let in_sdk = |parts: &[&str]| {
        let mut path = sdk_dir.clone()?;
        path.extend(parts);
        Some(path)
    };

    let emulator = file_override(probe, &overrides.emulator).or_else(|| {
        in_sdk(&["emulator", "emulator.exe"])
            .filter(|file| probe.is_file(file))
            .map(|file| {
                found(
                    file,
                    Source::Default,
                    via_sdk(&["emulator", "emulator.exe"]),
                )
            })
    });
    let adb = file_override(probe, &overrides.adb)
        .or_else(|| {
            in_sdk(&["platform-tools", "adb.exe"])
                .filter(|file| probe.is_file(file))
                .map(|file| {
                    found(
                        file,
                        Source::Default,
                        via_sdk(&["platform-tools", "adb.exe"]),
                    )
                })
        })
        .or_else(|| on_path(probe, "adb.exe").map(|file| found(file, Source::Path, "PATH")));
    let cmdline_tools = overrides
        .cmdline_tools
        .clone()
        .filter(|dir| is_cmdline_tools(probe, dir))
        .map(|dir| found(dir, Source::Settings, ""))
        .or_else(|| {
            let root = in_sdk(&["cmdline-tools"])?;
            let latest = root.join("latest").join("bin");
            if is_cmdline_tools(probe, &latest) {
                let via = via_sdk(&["cmdline-tools", "latest", "bin"]);
                return Some(found(latest, Source::Default, via));
            }
            // Otherwise the highest versioned folder, such as `cmdline-tools\19.0\bin`.
            let mut versions = probe.subdirs(&root);
            versions.sort_by(|a, b| compare_versions(b, a));
            versions
                .into_iter()
                .find(|version| is_cmdline_tools(probe, &root.join(version).join("bin")))
                .map(|version| {
                    let via = via_sdk(&["cmdline-tools", &version, "bin"]);
                    found(root.join(&version).join("bin"), Source::Default, via)
                })
        });
    Tools {
        emulator,
        adb,
        cmdline_tools,
        avd_home: resolve_avd_home(overrides, probe),
        jdk: resolve_jdk(overrides, probe),
        sdk,
    }
}

fn file_override(probe: &impl Probe, path: &Option<PathBuf>) -> Option<Found> {
    path.clone()
        .filter(|file| probe.is_file(file))
        .map(|file| found(file, Source::Settings, ""))
}

fn resolve_sdk(overrides: &Overrides, probe: &impl Probe) -> Option<Found> {
    if let Some(dir) = overrides.sdk.clone().filter(|dir| is_sdk(probe, dir)) {
        return Some(found(dir, Source::Settings, ""));
    }
    for name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(dir) = probe.var(name).map(PathBuf::from)
            && is_sdk(probe, &dir)
        {
            return Some(found(dir, Source::Environment(name), format!("%{name}%")));
        }
    }
    // `<sdk>\platform-tools\adb.exe` or `<sdk>\emulator\emulator.exe` on PATH.
    for name in ["adb.exe", "emulator.exe"] {
        if let Some(dir) = on_path(probe, name)
            .and_then(|file| Some(file.parent()?.parent()?.to_owned()))
            .filter(|dir| is_sdk(probe, dir))
        {
            return Some(found(dir, Source::Path, "PATH"));
        }
    }
    let default = PathBuf::from(probe.var("LOCALAPPDATA")?)
        .join("Android")
        .join("Sdk");
    is_sdk(probe, &default).then(|| found(default, Source::Default, "%LOCALAPPDATA%\\Android\\Sdk"))
}

fn resolve_avd_home(overrides: &Overrides, probe: &impl Probe) -> Option<Found> {
    if let Some(dir) = overrides.avd_home.clone().filter(|dir| probe.is_dir(dir)) {
        return Some(found(dir, Source::Settings, ""));
    }
    if let Some(dir) = probe
        .var("ANDROID_AVD_HOME")
        .map(PathBuf::from)
        .filter(|dir| probe.is_dir(dir))
    {
        return Some(found(
            dir,
            Source::Environment("ANDROID_AVD_HOME"),
            "%ANDROID_AVD_HOME%",
        ));
    }
    if let Some(dir) = probe
        .var("ANDROID_USER_HOME")
        .map(|home| PathBuf::from(home).join("avd"))
        .filter(|dir| probe.is_dir(dir))
    {
        return Some(found(
            dir,
            Source::Environment("ANDROID_USER_HOME"),
            "%ANDROID_USER_HOME%\\avd",
        ));
    }
    // The emulator's own default, even before the first AVD creates it.
    let default = PathBuf::from(probe.var("USERPROFILE")?)
        .join(".android")
        .join("avd");
    Some(found(
        default,
        Source::Default,
        "%USERPROFILE%\\.android\\avd",
    ))
}

fn resolve_jdk(overrides: &Overrides, probe: &impl Probe) -> Option<Found> {
    if let Some(dir) = overrides.jdk.clone().filter(|dir| is_jdk(probe, dir)) {
        return Some(found(dir, Source::Settings, ""));
    }
    if let Some(dir) = probe
        .var("JAVA_HOME")
        .map(PathBuf::from)
        .filter(|dir| is_jdk(probe, dir))
    {
        return Some(found(dir, Source::Environment("JAVA_HOME"), "%JAVA_HOME%"));
    }
    // Android Studio's bundled runtime.
    for root in ["ProgramFiles", "LOCALAPPDATA"] {
        let Some(base) = probe.var(root).map(PathBuf::from) else {
            continue;
        };
        let parts = if root == "LOCALAPPDATA" {
            ["Programs", "Android Studio", "jbr"]
        } else {
            ["Android", "Android Studio", "jbr"]
        };
        let studio: PathBuf = std::iter::once(base)
            .chain(parts.map(PathBuf::from))
            .collect();
        if is_jdk(probe, &studio) {
            let via = format!("%{root}%\\{}", parts.join("\\"));
            return Some(found(studio, Source::Default, via));
        }
    }
    on_path(probe, "java.exe")
        .and_then(|file| Some(file.parent()?.parent()?.to_owned()))
        .filter(|dir| is_jdk(probe, dir))
        .map(|dir| found(dir, Source::Path, "PATH"))
}

/// Compares dotted versions numerically (`19.0` > `9.0`); non-numeric parts compare as text.
fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parts = |text: &str| -> Vec<u64> {
        text.split(['.', '-'])
            .map(|part| part.parse().unwrap_or(0))
            .collect()
    };
    parts(a).cmp(&parts(b)).then_with(|| a.cmp(b))
}

impl Tools {
    /// Variables every child process gets, so `avdmanager`, `sdkmanager` and the emulator use the
    /// same SDK, AVD home and JDK as Avdeck.
    pub fn child_env(&self) -> Vec<(&'static str, OsString)> {
        let mut env = Vec::new();
        if let Some(sdk) = &self.sdk {
            env.push(("ANDROID_HOME", sdk.path.clone().into_os_string()));
            env.push(("ANDROID_SDK_ROOT", sdk.path.clone().into_os_string()));
        }
        if let Some(home) = &self.avd_home {
            env.push(("ANDROID_AVD_HOME", home.path.clone().into_os_string()));
        }
        if let Some(jdk) = &self.jdk {
            env.push(("JAVA_HOME", jdk.path.clone().into_os_string()));
        }
        env
    }
}

/// `Pkg.Revision` of the emulator next to `emulator.exe`, e.g. `37.2.12`.
pub fn emulator_version(emulator: &Path) -> Option<String> {
    let properties = std::fs::read_to_string(emulator.parent()?.join("source.properties")).ok()?;
    crate::ini::parse(&properties).remove("Pkg.Revision")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::{HashMap, HashSet};

    /// A fake machine: variables, files, and folders (with their parents implied).
    #[derive(Default)]
    struct Fake {
        vars: HashMap<String, String>,
        files: HashSet<PathBuf>,
    }

    impl Fake {
        fn var(mut self, name: &str, value: &str) -> Self {
            self.vars.insert(name.to_owned(), value.to_owned());
            self
        }

        fn file(mut self, path: &str) -> Self {
            self.files.insert(PathBuf::from(path));
            self
        }
    }

    impl Probe for Fake {
        fn var(&self, name: &str) -> Option<OsString> {
            self.vars.get(name).map(Into::into)
        }

        fn is_file(&self, path: &Path) -> bool {
            self.files.contains(path)
        }

        fn is_dir(&self, path: &Path) -> bool {
            self.files
                .iter()
                .any(|file| file.starts_with(path) && file != path)
        }

        fn subdirs(&self, path: &Path) -> Vec<String> {
            let mut names: Vec<String> = self
                .files
                .iter()
                .filter_map(|file| file.strip_prefix(path).ok())
                .filter(|rest| rest.components().count() > 1)
                .filter_map(|rest| rest.components().next())
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names.dedup();
            names
        }
    }

    /// This machine as measured on 2026-10-22.
    fn this_machine() -> Fake {
        Fake::default()
            .var("ANDROID_HOME", r"D:\Android\SDK")
            .var("ANDROID_AVD_HOME", r"D:\Android\avd")
            .var("JAVA_HOME", r"C:\Program Files\Eclipse Adoptium\jdk-25")
            .var("USERPROFILE", r"C:\Users\kien1")
            .file(r"D:\Android\SDK\platform-tools\adb.exe")
            .file(r"D:\Android\SDK\emulator\emulator.exe")
            .file(r"D:\Android\SDK\cmdline-tools\latest\bin\avdmanager.bat")
            .file(r"D:\Android\SDK\cmdline-tools\latest\bin\sdkmanager.bat")
            .file(r"D:\Android\avd\Pixel_Tablet.ini")
            .file(r"C:\Program Files\Eclipse Adoptium\jdk-25\bin\java.exe")
    }

    #[test]
    fn finds_everything_from_the_environment() {
        let tools = resolve(&Overrides::default(), &this_machine());
        let sdk = tools.sdk.unwrap();
        assert_eq!(sdk.path, PathBuf::from(r"D:\Android\SDK"));
        assert_eq!(sdk.source, Source::Environment("ANDROID_HOME"));
        assert_eq!(
            tools.emulator.unwrap().path,
            PathBuf::from(r"D:\Android\SDK\emulator\emulator.exe")
        );
        assert_eq!(
            tools.cmdline_tools.unwrap().path,
            PathBuf::from(r"D:\Android\SDK\cmdline-tools\latest\bin")
        );
        assert_eq!(
            tools.avd_home.unwrap().source,
            Source::Environment("ANDROID_AVD_HOME")
        );
        assert_eq!(tools.jdk.unwrap().source, Source::Environment("JAVA_HOME"));
    }

    #[test]
    fn says_how_each_path_was_found() {
        let tools = resolve(&Overrides::default(), &this_machine());
        assert_eq!(tools.sdk.unwrap().via, "%ANDROID_HOME%");
        assert_eq!(
            tools.emulator.unwrap().via,
            r"%ANDROID_HOME%\emulator\emulator.exe"
        );
        assert_eq!(
            tools.cmdline_tools.unwrap().via,
            r"%ANDROID_HOME%\cmdline-tools\latest\bin"
        );
        assert_eq!(tools.avd_home.unwrap().via, "%ANDROID_AVD_HOME%");
        assert_eq!(tools.jdk.unwrap().via, "%JAVA_HOME%");
        // Set in the settings: nothing to explain.
        let probe = this_machine().file(r"E:\sdk\platform-tools\adb.exe");
        let overrides = Overrides {
            sdk: Some(r"E:\sdk".into()),
            ..Default::default()
        };
        let tools = resolve(&overrides, &probe);
        assert_eq!(tools.sdk.unwrap().via, "");
        assert_eq!(tools.adb.unwrap().via, r"SDK\platform-tools\adb.exe");
    }

    #[test]
    fn settings_win_over_the_environment() {
        let probe = this_machine()
            .file(r"E:\sdk\platform-tools\adb.exe")
            .file(r"E:\avd\x.ini");
        let overrides = Overrides {
            sdk: Some(r"E:\sdk".into()),
            avd_home: Some(r"E:\avd".into()),
            ..Default::default()
        };
        let tools = resolve(&overrides, &probe);
        assert_eq!(tools.sdk.unwrap().source, Source::Settings);
        assert_eq!(
            tools.adb.unwrap().path,
            PathBuf::from(r"E:\sdk\platform-tools\adb.exe")
        );
        // No emulator in that SDK.
        assert_eq!(tools.emulator, None);
        assert_eq!(tools.avd_home.unwrap().source, Source::Settings);
    }

    #[test]
    fn an_invalid_setting_falls_back() {
        let overrides = Overrides {
            sdk: Some(r"E:\nothing".into()),
            ..Default::default()
        };
        let tools = resolve(&overrides, &this_machine());
        assert_eq!(
            tools.sdk.unwrap().source,
            Source::Environment("ANDROID_HOME")
        );
    }

    #[test]
    fn without_variables_uses_path_studio_and_defaults() {
        let probe = Fake::default()
            .var("PATH", r"C:\bin;D:\Android\SDK\platform-tools")
            .var("USERPROFILE", r"C:\Users\kien1")
            .var("ProgramFiles", r"C:\Program Files")
            .file(r"D:\Android\SDK\platform-tools\adb.exe")
            .file(r"D:\Android\SDK\cmdline-tools\9.0\bin\avdmanager.bat")
            .file(r"D:\Android\SDK\cmdline-tools\9.0\bin\sdkmanager.bat")
            .file(r"D:\Android\SDK\cmdline-tools\19.0\bin\avdmanager.bat")
            .file(r"D:\Android\SDK\cmdline-tools\19.0\bin\sdkmanager.bat")
            .file(r"C:\Program Files\Android\Android Studio\jbr\bin\java.exe");
        let tools = resolve(&Overrides::default(), &probe);
        let sdk = tools.sdk.unwrap();
        assert_eq!(
            (sdk.path, sdk.source),
            (PathBuf::from(r"D:\Android\SDK"), Source::Path)
        );
        assert_eq!(
            tools.cmdline_tools.unwrap().path,
            PathBuf::from(r"D:\Android\SDK\cmdline-tools\19.0\bin")
        );
        assert_eq!(
            tools.jdk.unwrap().path,
            PathBuf::from(r"C:\Program Files\Android\Android Studio\jbr")
        );
        let home = tools.avd_home.unwrap();
        assert_eq!(
            (home.path, home.source),
            (
                PathBuf::from(r"C:\Users\kien1\.android\avd"),
                Source::Default
            )
        );
        let again = resolve(&Overrides::default(), &probe);
        assert_eq!(again.sdk.unwrap().via, "PATH");
        assert_eq!(
            again.cmdline_tools.unwrap().via,
            r"PATH\cmdline-tools\19.0\bin"
        );
        assert_eq!(
            again.jdk.unwrap().via,
            r"%ProgramFiles%\Android\Android Studio\jbr"
        );
        assert_eq!(again.avd_home.unwrap().via, r"%USERPROFILE%\.android\avd");
    }

    #[test]
    fn child_processes_get_the_resolved_paths() {
        let env = resolve(&Overrides::default(), &this_machine()).child_env();
        let names: Vec<&str> = env.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            names,
            [
                "ANDROID_HOME",
                "ANDROID_SDK_ROOT",
                "ANDROID_AVD_HOME",
                "JAVA_HOME"
            ]
        );
    }

    #[test]
    fn compares_versions_numerically() {
        assert!(compare_versions("19.0", "9.0").is_gt());
        assert!(compare_versions("12.0", "12.0-rc1").is_lt());
    }
}
