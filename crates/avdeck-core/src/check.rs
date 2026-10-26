//! Checks that the resolved tools actually run: `adb version`, `java -version` and
//! `emulator -accel-check`, without console windows and with a time limit, and what their output
//! says.

use std::ffi::OsString;
use std::io::{self, Read};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// What a tool printed (stdout then stderr) and whether it exited successfully.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub success: bool,
    pub text: String,
}

/// Runs `program` with `args` and the resolved environment `env`; one still running after
/// `limit` is killed and counts as an error.
pub fn run(
    program: &Path,
    args: &[&str],
    env: &[(&'static str, OsString)],
    limit: Duration,
) -> io::Result<Output> {
    let mut child = Command::new(program)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()?;
    // Read both pipes while waiting: a full pipe would block the tool.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut text = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut text);
            }
            text
        })
    };
    let stdout = read(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = read(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("no answer in {} s", limit.as_secs()),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut text = String::from_utf8_lossy(&stdout.join().unwrap_or_default()).into_owned();
    text.push_str(&String::from_utf8_lossy(&stderr.join().unwrap_or_default()));
    Ok(Output {
        success: status.success(),
        text,
    })
}

/// The version `adb version` prints, without its build number: `37.0.1`.
pub fn adb_version(output: &str) -> Option<String> {
    let line = output
        .lines()
        .find_map(|line| line.trim().strip_prefix("Version "))?;
    let version = line.split('-').next()?.trim();
    (!version.is_empty()).then(|| version.to_owned())
}

/// The quoted version on the first line of `java -version`: `25.0.3`, `1.8.0_391`.
pub fn java_version(output: &str) -> Option<String> {
    let line = output.lines().find(|line| line.contains(" version \""))?;
    let (_, rest) = line.split_once('"')?;
    let (version, _) = rest.split_once('"')?;
    Some(version.to_owned())
}

/// What `emulator -accel-check` says about hardware acceleration (WHPX or AEHD on Windows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Acceleration {
    /// Status 0: virtual machines run accelerated.
    pub ready: bool,
    /// The emulator's own words, such as `WHPX(10.0.26300) is installed and usable.`
    pub message: String,
}

/// Reads the block the emulator prints between `accel:` and `accel`: a status code, then the
/// message.
pub fn acceleration(output: &str) -> Option<Acceleration> {
    let mut lines = output
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "accel:")
        .skip(1);
    let code: i32 = lines.next()?.parse().ok()?;
    let message = lines
        .take_while(|line| *line != "accel")
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(Acceleration {
        ready: code == 0,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Recorded 2026-10-26 with platform-tools 37.0.1, Temurin 25 and emulator 37.2.12.
    const ADB: &str = "Android Debug Bridge version 1.0.41\r\nVersion 37.0.1-15733141\r\n\
        Installed as D:\\Android\\SDK\\platform-tools\\adb.exe\r\nRunning on Windows 10.0.26300\r\n";
    const JAVA: &str = "openjdk version \"25.0.3\" 2026-04-21 LTS\r\n\
        OpenJDK Runtime Environment Temurin-25.0.3+9 (build 25.0.3+9-LTS)\r\n\
        OpenJDK 64-Bit Server VM Temurin-25.0.3+9 (build 25.0.3+9-LTS, mixed mode, sharing)\r\n";
    const ACCEL: &str = "accel:\r\n0\r\nWHPX(10.0.26300) is installed and usable.\r\naccel\r\n";

    #[test]
    fn reads_the_adb_version() {
        assert_eq!(adb_version(ADB).as_deref(), Some("37.0.1"));
        assert_eq!(adb_version("adb: not found"), None);
    }

    #[test]
    fn reads_the_java_version() {
        assert_eq!(java_version(JAVA).as_deref(), Some("25.0.3"));
        // Java 8 writes it the old way.
        assert_eq!(
            java_version("java version \"1.8.0_391\"\nJava(TM) SE Runtime Environment").as_deref(),
            Some("1.8.0_391")
        );
        assert_eq!(java_version("Error: could not find java.dll"), None);
    }

    #[test]
    fn reads_the_acceleration_check() {
        assert_eq!(
            acceleration(ACCEL),
            Some(Acceleration {
                ready: true,
                message: "WHPX(10.0.26300) is installed and usable.".to_owned()
            })
        );
        // Not recorded: any other status means not accelerated, whatever the message.
        let failed =
            acceleration("accel:\n9\nSomething is not enabled.\nSee the docs.\naccel\n").unwrap();
        assert!(!failed.ready);
        assert_eq!(failed.message, "Something is not enabled. See the docs.");
        assert_eq!(acceleration("unknown option -accel-check"), None);
    }
}
