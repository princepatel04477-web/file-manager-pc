//! Duplicate detection: cheap size grouping, a 64 KB preview hash, then a full
//! BLAKE3 pass in parallel. The grouping and "which copy is the original" rules are
//! pure so they are covered by unit tests; the hashing pipeline is exercised against
//! real temporary files.

use crate::clean::rules::{self, DUPLICATE_PREVIEW_BYTES};
use blake3::Hasher;
use rayon::prelude::*;
use serde::Serialize;
use specta::Type;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// One file the index says could have a twin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub path: PathBuf,
    pub name: String,
    pub size: u64,
    pub modified_unix: Option<i64>,
}

/// Bucket indices that share an exact size. Only buckets with two or more entries
/// are worth hashing.
pub fn group_by_size(candidates: &[Candidate]) -> Vec<Vec<usize>> {
    let mut buckets: Vec<(u64, Vec<usize>)> = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if !rules::is_duplicate_candidate(candidate.size, false, false) {
            continue;
        }
        match buckets.iter_mut().find(|(size, _)| *size == candidate.size) {
            Some((_, members)) => members.push(index),
            None => buckets.push((candidate.size, vec![index])),
        }
    }
    let mut groups: Vec<Vec<usize>> = buckets
        .into_iter()
        .filter(|(_, members)| members.len() > 1)
        .map(|(_, members)| members)
        .collect();
    // Biggest buckets first: they are the ones worth the user's attention.
    groups.sort_by(|left, right| {
        let left_bytes = candidates[left[0]].size.saturating_mul(left.len() as u64);
        let right_bytes = candidates[right[0]].size.saturating_mul(right.len() as u64);
        right_bytes.cmp(&left_bytes)
    });
    groups
}

/// The copy Sift protects: the oldest file, breaking ties by the shortest and then
/// alphabetically first path. "Original" means "least likely to be the stray copy".
pub fn original_index(group: &[&Candidate]) -> usize {
    let mut best = 0_usize;
    for (index, candidate) in group.iter().enumerate().skip(1) {
        let current = group[best];
        let by_age = candidate
            .modified_unix
            .zip(current.modified_unix)
            .map(|(mine, theirs)| mine.cmp(&theirs));
        let better = match by_age {
            Some(ordering) => ordering.is_lt(),
            None => path_depth(&candidate.path) < path_depth(&current.path),
        };
        // With no timestamps to compare and the same depth, the alphabetically
        // first path wins so the choice is stable between runs.
        let tie_break = by_age.is_none()
            && path_depth(&candidate.path) == path_depth(&current.path)
            && candidate.path.to_string_lossy() < current.path.to_string_lossy();
        if better || tie_break {
            best = index;
        }
    }
    best
}

/// Everything except the original — the default selection on the review screen.
pub fn suggested_removals(group: &[&Candidate]) -> Vec<usize> {
    let keep = original_index(group);
    (0..group.len()).filter(|index| *index != keep).collect()
}

fn path_depth(path: &Path) -> usize {
    path.components().count()
}

/// BLAKE3 over the first [`DUPLICATE_PREVIEW_BYTES`] — enough to reject most
/// same-size impostors without reading whole files.
pub fn hash_preview(file: &mut File) -> Option<String> {
    let mut buffer = vec![0_u8; DUPLICATE_PREVIEW_BYTES];
    let mut read_total = 0_usize;
    while read_total < buffer.len() {
        let read = file.read(&mut buffer[read_total..]).ok()?;
        if read == 0 {
            break;
        }
        read_total += read;
    }
    let mut hasher = Hasher::new();
    hasher.update(&buffer[..read_total]);
    Some(hasher.finalize().to_hex().to_string())
}

/// BLAKE3 over the whole file.
pub fn hash_full(file: &mut File) -> Option<String> {
    let mut hasher = Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Some(hasher.finalize().to_hex().to_string())
}

fn open(path: &Path) -> Option<File> {
    File::open(crate::ops::io_path(path)).ok()
}

/// Split a bucket by preview hash, then confirm each remaining set with a full hash.
/// Hashing runs in parallel across the bucket.
pub fn confirm_group(indices: &[usize], candidates: &[Candidate]) -> Vec<(String, Vec<usize>)> {
    let preview: Vec<(usize, Option<String>)> = indices
        .par_iter()
        .map(|index| {
            let candidate = &candidates[*index];
            let digest = open(&candidate.path).and_then(|mut file| hash_preview(&mut file));
            (*index, digest)
        })
        .collect();

    let mut by_preview: Vec<(String, Vec<usize>)> = Vec::new();
    for (index, digest) in preview {
        let Some(digest) = digest else { continue };
        match by_preview.iter_mut().find(|(key, _)| *key == digest) {
            Some((_, members)) => members.push(index),
            None => by_preview.push((digest, vec![index])),
        }
    }

    let mut confirmed: Vec<(String, Vec<usize>)> = Vec::new();
    for (_, members) in by_preview.into_iter().filter(|(_, members)| members.len() > 1) {
        let full: Vec<(usize, Option<String>)> = members
            .par_iter()
            .map(|index| {
                let candidate = &candidates[*index];
                let digest = open(&candidate.path).and_then(|mut file| hash_full(&mut file));
                (*index, digest)
            })
            .collect();
        let mut by_full: Vec<(String, Vec<usize>)> = Vec::new();
        for (index, digest) in full {
            let Some(digest) = digest else { continue };
            match by_full.iter_mut().find(|(key, _)| *key == digest) {
                Some((_, members)) => members.push(index),
                None => by_full.push((digest, vec![index])),
            }
        }
        confirmed.extend(by_full.into_iter().filter(|(_, members)| members.len() > 1));
    }
    confirmed
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub modified_unix: Option<i64>,
    /// True for the copy Sift protects by default.
    pub original: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateSet {
    pub fingerprint: String,
    pub size: u64,
    pub files: Vec<DuplicateFile>,
    pub reclaimable_bytes: u64,
}

#[derive(Clone, Debug, Default, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateReport {
    pub sets: Vec<DuplicateSet>,
    pub reclaimable_bytes: u64,
    pub candidates: u64,
    pub hashed: u64,
    pub skipped: u64,
}

/// Full pipeline: size buckets → preview hash → parallel full hash → review payload.
/// `on_progress` receives an approximate byte count after each bucket so the caller can
/// drive a progress bar; tests pass a no-op.
pub fn find_duplicates(
    candidates: Vec<Candidate>,
    cancel: &dyn Fn() -> bool,
    on_progress: &mut dyn FnMut(u64),
) -> DuplicateReport {
    let mut report = DuplicateReport { candidates: candidates.len() as u64, ..DuplicateReport::default() };
    for indices in group_by_size(&candidates) {
        if cancel() {
            break;
        }
        let bucket_bytes = candidates[indices[0]].size.saturating_mul(indices.len() as u64);
        let confirmed = confirm_group(&indices, &candidates);
        report.hashed = report.hashed.saturating_add(indices.len() as u64);
        on_progress(bucket_bytes);
        for (fingerprint, members) in confirmed {
            let size = candidates[members[0]].size;
            let group: Vec<&Candidate> = members.iter().map(|index| &candidates[*index]).collect();
            let keep = original_index(&group);
            let files: Vec<DuplicateFile> = group
                .iter()
                .enumerate()
                .map(|(index, candidate)| DuplicateFile {
                    path: crate::ops::display_path(&candidate.path),
                    name: candidate.name.clone(),
                    size: candidate.size,
                    modified_unix: candidate.modified_unix,
                    original: index == keep,
                })
                .collect();
            let reclaimable_bytes = rules::duplicate_reclaimable_bytes(size, files.len());
            report.reclaimable_bytes = report.reclaimable_bytes.saturating_add(reclaimable_bytes);
            report.sets.push(DuplicateSet { fingerprint, size, files, reclaimable_bytes });
        }
    }
    report.sets.sort_by_key(|set| std::cmp::Reverse(set.reclaimable_bytes));
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn candidate(path: &str, size: u64, modified_unix: Option<i64>) -> Candidate {
        Candidate {
            path: PathBuf::from(path),
            name: path.split(['/', '\\']).next_back().unwrap_or(path).to_owned(),
            size,
            modified_unix,
        }
    }

    fn temp_root(tag: &str) -> PathBuf {
        let unique = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
        let path = std::env::temp_dir().join(format!("sift-dupes-{tag}-{unique}"));
        std::fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn groups_only_same_size_files_and_needs_at_least_two() {
        let candidates = vec![
            candidate("/u/a.bin", 5_000, Some(10)),
            candidate("/u/b.bin", 5_000, Some(20)),
            candidate("/u/c.bin", 9_000, Some(30)),
        ];
        let groups = group_by_size(&candidates);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0], vec![0, 1]);
    }

    #[test]
    fn small_cloud_and_folder_entries_never_enter_a_bucket() {
        let candidates = vec![
            candidate("/u/tiny.bin", 512, Some(10)),
            candidate("/u/tiny-copy.bin", 512, Some(11)),
            candidate("/u/big.bin", 4_096, Some(12)),
        ];
        assert!(group_by_size(&candidates).is_empty(), "under 1 KB is ignored even in pairs");
        // The same shape above the threshold does group.
        let bigger = vec![
            candidate("/u/a.bin", 4_096, Some(12)),
            candidate("/u/b.bin", 4_096, Some(13)),
        ];
        assert_eq!(group_by_size(&bigger).len(), 1);
    }

    #[test]
    fn biggest_buckets_are_reviewed_first() {
        let candidates = vec![
            candidate("/u/small-a.bin", 2_000, Some(1)),
            candidate("/u/small-b.bin", 2_000, Some(2)),
            candidate("/u/big-a.bin", 900_000, Some(3)),
            candidate("/u/big-b.bin", 900_000, Some(4)),
        ];
        let groups = group_by_size(&candidates);
        assert_eq!(groups.len(), 2);
        assert_eq!(candidates[groups[0][0]].size, 900_000, "the expensive set comes first");
    }

    #[test]
    fn the_oldest_copy_is_the_protected_original() {
        let group_owned = [candidate("/u/copies/newer.bin", 5_000, Some(300)),
            candidate("/u/original.bin", 5_000, Some(100)),
            candidate("/u/copies/middle.bin", 5_000, Some(200))];
        let group: Vec<&Candidate> = group_owned.iter().collect();
        assert_eq!(original_index(&group), 1);
        assert_eq!(suggested_removals(&group), vec![0, 2]);
    }

    #[test]
    fn ties_are_broken_by_the_shallowest_then_alphabetically_first_path() {
        let same_age = [candidate("/u/z/keep-me.bin", 5_000, Some(100)),
            candidate("/u/a/other.bin", 5_000, Some(100))];
        let group: Vec<&Candidate> = same_age.iter().collect();
        assert_eq!(original_index(&group), 0, "same depth, alphabetically first wins");

        let different_depth = [candidate("/u/deep/nested/folder/file.bin", 5_000, None),
            candidate("/u/file.bin", 5_000, None)];
        let group: Vec<&Candidate> = different_depth.iter().collect();
        assert_eq!(original_index(&group), 1, "the shallowest copy is the original");
    }

    #[test]
    fn exactly_one_copy_is_protected_in_every_set() {
        for count in 2..6 {
            let owned: Vec<Candidate> = (0..count)
                .map(|index| candidate(&format!("/u/copy{index}.bin"), 7_777, Some(index as i64)))
                .collect();
            let group: Vec<&Candidate> = owned.iter().collect();
            let removals = suggested_removals(&group);
            assert_eq!(removals.len(), count - 1);
            assert!(!removals.contains(&original_index(&group)));
        }
    }

    #[test]
    fn identical_files_survive_both_hash_passes() {
        let root = temp_root("identical");
        let payload: Vec<u8> = (0..200_000_u32).map(|value| (value % 251) as u8).collect();
        let first = root.join("first.bin");
        let second = root.join("nested").join("second.bin");
        std::fs::create_dir_all(second.parent().expect("parent")).expect("dir");
        std::fs::write(&first, &payload).expect("write");
        std::fs::write(&second, &payload).expect("write");

        let candidates = vec![
            candidate(&first.to_string_lossy(), payload.len() as u64, Some(100)),
            candidate(&second.to_string_lossy(), payload.len() as u64, Some(200)),
        ];
        let report = find_duplicates(candidates, &|| false, &mut |_| {});
        assert_eq!(report.sets.len(), 1, "one matching set");
        assert_eq!(report.sets[0].files.len(), 2);
        assert_eq!(report.reclaimable_bytes, payload.len() as u64);
        let originals: Vec<bool> = report.sets[0].files.iter().map(|file| file.original).collect();
        assert_eq!(originals.iter().filter(|value| **value).count(), 1, "exactly one protected copy");
        assert!(report.sets[0].files.iter().any(|file| file.original && file.path.ends_with("first.bin")), "the older file is kept");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_size_different_content_is_not_a_duplicate() {
        let root = temp_root("different");
        let first = root.join("a.bin");
        let second = root.join("b.bin");
        std::fs::write(&first, vec![1_u8; 80_000]).expect("write");
        std::fs::write(&second, vec![2_u8; 80_000]).expect("write");

        let candidates = vec![
            candidate(&first.to_string_lossy(), 80_000, Some(1)),
            candidate(&second.to_string_lossy(), 80_000, Some(2)),
        ];
        let report = find_duplicates(candidates, &|| false, &mut |_| {});
        assert!(report.sets.is_empty(), "the preview hash separates them");
        assert_eq!(report.hashed, 2, "both candidates were hashed");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn files_that_only_match_in_the_first_64kb_are_still_separated() {
        let root = temp_root("tail");
        let shared = vec![7_u8; DUPLICATE_PREVIEW_BYTES];
        let mut first_payload = shared.clone();
        let mut second_payload = shared;
        first_payload.push(1);
        second_payload.push(2);
        let first = root.join("a.bin");
        let second = root.join("b.bin");
        std::fs::write(&first, &first_payload).expect("write");
        std::fs::write(&second, &second_payload).expect("write");

        let candidates = vec![
            candidate(&first.to_string_lossy(), first_payload.len() as u64, Some(1)),
            candidate(&second.to_string_lossy(), second_payload.len() as u64, Some(2)),
        ];
        // The preview hashes match; only the full pass can tell them apart.
        let mut first_file = open(&first).expect("open");
        let mut second_file = open(&second).expect("open");
        assert_eq!(hash_preview(&mut first_file), hash_preview(&mut second_file));
        let mut first_file = open(&first).expect("open");
        let mut second_file = open(&second).expect("open");
        assert_ne!(hash_full(&mut first_file), hash_full(&mut second_file));

        let report = find_duplicates(candidates, &|| false, &mut |_| {});
        assert!(report.sets.is_empty(), "the full hash rejects the near-miss");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unreadable_files_are_dropped_instead_of_failing_the_scan() {
        let candidates = vec![
            candidate("/definitely/not/here/a.bin", 5_000, Some(1)),
            candidate("/definitely/not/here/b.bin", 5_000, Some(2)),
        ];
        let report = find_duplicates(candidates, &|| false, &mut |_| {});
        assert!(report.sets.is_empty());
        assert_eq!(report.candidates, 2);
    }

    #[test]
    fn cancellation_stops_before_the_next_bucket() {
        let candidates = vec![
            candidate("/u/a.bin", 5_000, Some(1)),
            candidate("/u/b.bin", 5_000, Some(2)),
        ];
        let report = find_duplicates(candidates, &|| true, &mut |_| {});
        assert!(report.sets.is_empty());
        assert_eq!(report.hashed, 0, "nothing was hashed after cancellation");
    }
}
