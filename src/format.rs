//! How sizes and times read in the explorer.

use std::time::SystemTime;

use chrono::DateTime;
use chrono::Local;

pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

pub fn items(count: u64) -> String {
    if count == 1 { "1 item".into() } else { format!("{} items", grouped(count)) }
}

/// A number with its thousands set apart, as in 39,001.
pub fn grouped(n: impl Into<u64>) -> String {
    let digits = n.into().to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

pub fn time(time: SystemTime) -> String {
    DateTime::<Local>::from(time).format("%Y-%m-%d %H:%M").to_string()
}

pub fn count(n: usize, one: &str, many: &str) -> String {
    format!("{} {}", grouped(n as u64), if n == 1 { one } else { many })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(size(0), "0 B");
        assert_eq!(size(1023), "1023 B");
        assert_eq!(size(1536), "1.5 KB");
        assert_eq!(size(44 * 1024 * 1024), "44 MB");
        assert_eq!(grouped(7u64), "7");
        assert_eq!(grouped(39_001u64), "39,001");
        assert_eq!(grouped(1_234_567u64), "1,234,567");
        assert_eq!(count(20_000, "match", "matches"), "20,000 matches");
    }
}
