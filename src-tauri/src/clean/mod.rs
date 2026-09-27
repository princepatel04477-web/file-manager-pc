pub const LARGE_FILE_THRESHOLD: u64 = 100 * 1024 * 1024;

pub fn is_large_file(size: u64) -> bool {
    size >= LARGE_FILE_THRESHOLD
}

pub fn duplicate_reclaimable_bytes(size: u64, copies: usize) -> u64 {
    size.saturating_mul(copies.saturating_sub(1) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_file_threshold_is_inclusive() {
        assert!(!is_large_file(LARGE_FILE_THRESHOLD - 1));
        assert!(is_large_file(LARGE_FILE_THRESHOLD));
    }

    #[test]
    fn duplicate_savings_preserve_one_copy() {
        assert_eq!(duplicate_reclaimable_bytes(10, 1), 0);
        assert_eq!(duplicate_reclaimable_bytes(10, 3), 20);
    }
}
