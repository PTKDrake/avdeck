//! Running the SDK's command-line tools (`avdmanager.bat`, `sdkmanager.bat`) with the resolved
//! environment, without console windows.

use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command(tool: &Path, args: &[&str], env: &[(&'static str, OsString)]) -> Command {
    // A .bat runs through cmd; `/c` with the quoted path and arguments.
    let mut command = Command::new("cmd.exe");
    command
        .arg("/c")
        .arg(tool)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    command
}

/// Runs `tool` to completion, feeding `input` on stdin, and returns stdout and stderr together.
/// Fails with that output when the tool exits with an error.
pub fn run(
    tool: &Path,
    args: &[&str],
    env: &[(&'static str, OsString)],
    input: &str,
) -> io::Result<String> {
    let mut child = command(tool, args, env).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let output = child.wait_with_output()?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    if output.status.success() {
        Ok(text)
    } else {
        Err(io::Error::other(text.trim().to_owned()))
    }
}

/// Like `run`, calling `progress` with the output read so far as it arrives (sdkmanager draws
/// its progress bar with `\r`).
pub fn run_with_progress(
    tool: &Path,
    args: &[&str],
    env: &[(&'static str, OsString)],
    input: &str,
    mut progress: impl FnMut(&str),
) -> io::Result<String> {
    let mut child = command(tool, args, env).spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let mut stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(stderr) = &mut stderr {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    });
    let mut text = String::new();
    if let Some(mut stdout) = child.stdout.take() {
        let mut buffer = [0u8; 4096];
        loop {
            let read = stdout.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            text.push_str(&String::from_utf8_lossy(&buffer[..read]));
            progress(&text);
        }
    }
    let status = child.wait()?;
    text.push_str(&errors.join().unwrap_or_default());
    if status.success() {
        Ok(text)
    } else {
        Err(io::Error::other(text.trim().to_owned()))
    }
}
