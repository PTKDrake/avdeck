//! The `key=value` files the SDK and emulator write (`config.ini`, discovery files).

use std::collections::HashMap;

/// Keys and values trimmed; `#` comments and lines without `=` are skipped.
pub fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect()
}
