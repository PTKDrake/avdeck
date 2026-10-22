//! The emulator console (telnet on the console port, 5554, 5556, ...). Used for what gRPC lacks,
//! such as `kill`, which stops the emulator and saves its quick-boot snapshot like
//! `adb emu kill`. Authenticates with `%USERPROFILE%\.emulator_console_auth_token`.

use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(5);

pub fn token_path() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .map(|home| PathBuf::from(home).join(".emulator_console_auth_token"))
}

/// Runs one console command and returns its output lines (without the final `OK`).
pub fn command(port: u16, command: &str) -> io::Result<Vec<String>> {
    let token = token_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|token| token.trim().to_owned())
        .unwrap_or_default();
    command_with_token(port, &token, command)
}

pub fn command_with_token(port: u16, token: &str, command: &str) -> io::Result<Vec<String>> {
    let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), TIMEOUT)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    let banner = read_reply(&mut reader)?;
    if banner
        .iter()
        .any(|line| line.contains("Authentication required"))
    {
        writeln!(writer, "auth {token}\r")?;
        read_reply(&mut reader)?;
    }
    writeln!(writer, "{command}\r")?;
    read_reply(&mut reader)
}

/// Lines up to a line that is exactly `OK` or starts with `KO` (an error), or the end of the
/// connection (`kill` closes it).
fn read_reply(reader: &mut impl BufRead) -> io::Result<Vec<String>> {
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(lines);
        }
        let line = line.trim_end().to_owned();
        if line == "OK" {
            return Ok(lines);
        }
        if let Some(error) = line.strip_prefix("KO") {
            return Err(io::Error::other(
                error.trim_start_matches(':').trim().to_owned(),
            ));
        }
        lines.push(line);
    }
}

/// Stops the emulator on `port`, saving the quick-boot snapshot unless it runs with
/// `-no-snapshot-save`.
pub fn kill(port: u16) -> io::Result<()> {
    command(port, "kill").map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    /// A console that wants `auth secret`, then answers `avd name` and `kill` like 37.2.12.
    fn fake_console() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut writer = stream.try_clone().unwrap();
            let mut reader = BufReader::new(stream);
            write!(
                writer,
                "Android Console: Authentication required\r\n\
                 Android Console: type 'auth <auth_token>' to authenticate\r\nOK\r\n"
            )
            .unwrap();
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            if line.trim() != "auth secret" {
                write!(writer, "KO: authentication token does not match\r\n").unwrap();
                return;
            }
            write!(
                writer,
                "Android Console: type 'help' for a list of commands\r\nOK\r\n"
            )
            .unwrap();
            line.clear();
            reader.read_line(&mut line).unwrap();
            match line.trim() {
                "avd name" => write!(writer, "Pixel_10_Pro_API_37_2\r\nOK\r\n").unwrap(),
                "kill" => write!(writer, "OK: killing emulator, bye bye\r\n").unwrap(),
                _ => write!(writer, "KO: unknown command\r\n").unwrap(),
            }
        });
        port
    }

    #[test]
    fn authenticates_and_reads_the_reply() {
        let port = fake_console();
        let lines = command_with_token(port, "secret", "avd name").unwrap();
        assert_eq!(lines, ["Pixel_10_Pro_API_37_2"]);
    }

    #[test]
    fn kill_ends_with_the_connection() {
        let port = fake_console();
        let lines = command_with_token(port, "secret", "kill").unwrap();
        assert_eq!(lines, ["OK: killing emulator, bye bye"]);
    }

    #[test]
    fn errors_come_back_as_errors() {
        let port = fake_console();
        let error = command_with_token(port, "wrong", "avd name").unwrap_err();
        assert_eq!(error.to_string(), "authentication token does not match");
        let port = fake_console();
        let error = command_with_token(port, "secret", "dance").unwrap_err();
        assert_eq!(error.to_string(), "unknown command");
    }
}
