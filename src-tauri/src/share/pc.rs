//! Pure protocol helpers for nearby PC sharing.
//!
//! Network orchestration lives at the Tauri boundary; keeping the pairing and
//! range calculations here makes the security-sensitive decisions testable.
use std::ops::Range;

pub const SERVICE_TYPE: &str = "_sift._tcp.local.";
pub const PC_SESSION_LIFETIME_SECONDS: u64 = 10 * 60;
pub const TRANSFER_CHUNK_SIZE: usize = 256 * 1024;

/// Generate a zero-padded pairing code from six uniformly selected decimal digits.
pub fn pairing_code(random: [u8; 4]) -> String {
    let value = u32::from_le_bytes(random) % 1_000_000;
    format!("{value:06}")
}

pub fn valid_pairing_code(candidate: &str, expected: &str) -> bool {
    candidate.len() == 6
        && candidate.bytes().all(|byte| byte.is_ascii_digit())
        && candidate.as_bytes().iter().zip(expected.as_bytes()).fold(
            candidate.len() ^ expected.len(),
            |difference, (left, right)| difference | usize::from(left ^ right),
        ) == 0
}

/// Parse a single HTTP byte range. Invalid/multiple ranges are deliberately rejected.
pub fn parse_range(header: &str, file_length: u64) -> Option<Range<u64>> {
    let value = header.strip_prefix("bytes=")?;
    if value.contains(',') || file_length == 0 {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        let suffix = end.parse::<u64>().ok()?;
        if suffix == 0 { return None; }
        let length = suffix.min(file_length);
        return Some(file_length - length..file_length);
    }
    let start = start.parse::<u64>().ok()?;
    if start >= file_length { return None; }
    let end = if end.is_empty() { file_length - 1 } else { end.parse::<u64>().ok()?.min(file_length - 1) };
    if end < start { return None; }
    Some(start..end + 1)
}

pub fn safe_device_name(value: &str) -> String {
    let cleaned = value
        .chars()
        .filter(|character| !character.is_control())
        .take(48)
        .collect::<String>()
        .trim()
        .to_owned();
    if cleaned.is_empty() { "Nearby PC".to_owned() } else { cleaned }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairing_code_is_six_decimal_digits() {
        let code = pairing_code([0xff; 4]);
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
    }

    #[test]
    fn pairing_code_rejects_wrong_length_and_non_digits() {
        assert!(valid_pairing_code("004219", "004219"));
        assert!(!valid_pairing_code("4219", "004219"));
        assert!(!valid_pairing_code("00a219", "00a219"));
        assert!(!valid_pairing_code("004218", "004219"));
    }

    #[test]
    fn parses_normal_open_ended_and_suffix_ranges() {
        assert_eq!(parse_range("bytes=2-5", 10), Some(2..6));
        assert_eq!(parse_range("bytes=7-", 10), Some(7..10));
        assert_eq!(parse_range("bytes=-3", 10), Some(7..10));
        assert_eq!(parse_range("bytes=-30", 10), Some(0..10));
    }

    #[test]
    fn rejects_invalid_or_multiple_ranges() {
        assert_eq!(parse_range("bytes=10-", 10), None);
        assert_eq!(parse_range("bytes=4-2", 10), None);
        assert_eq!(parse_range("bytes=1-2,4-5", 10), None);
        assert_eq!(parse_range("bytes=-0", 10), None);
    }

    #[test]
    fn device_name_is_bounded_and_nonempty() {
        assert_eq!(safe_device_name(" PC\n"), "PC");
        assert_eq!(safe_device_name("\n\t"), "Nearby PC");
        assert_eq!(safe_device_name(&"x".repeat(100)).len(), 48);
    }
}
