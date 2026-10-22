//! Runs one emulator console command, for checking the console client against a real emulator.
//!
//! ```text
//! cargo run -p avdeck-core --example console -- 5554 "avd name"
//! ```

fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args
        .next()
        .and_then(|port| port.parse().ok())
        .unwrap_or(5554);
    let command = args.next().unwrap_or_else(|| "avd name".to_owned());
    match avdeck_core::console::command(port, &command) {
        Ok(lines) => println!("OK {lines:?}"),
        Err(error) => println!("ERROR {error}"),
    }
}
