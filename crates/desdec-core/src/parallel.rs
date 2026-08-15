//! Spreading work across the machine's cores.
//!
//! Two rules hold everywhere this is used:
//!
//! - **The result never depends on the split.** Every parallel operation here
//!   combines its partial results associatively and in a fixed order, so the
//!   analysis of a file is identical on one core and on sixty-four. An analysis
//!   that changed with the hardware would be worthless as evidence.
//! - **Threads are only worth their cost above a threshold.** Spawning is
//!   measured in microseconds; small inputs stay on the calling thread.
//!
//! Only the standard library is used: scoped threads borrow the input instead
//! of requiring it to be shared or copied, which keeps `unsafe_code = forbid`.

use std::{sync::OnceLock, thread};

/// Smallest slice worth handing to its own thread.
///
/// Chosen from measurement rather than taste: below a few megabytes the
/// spawning and the cache traffic cost more than the split saves, while a
/// large file still reaches every core — at 147 MB this leaves 35 candidate
/// slices for 8 cores.
pub const MINIMUM_CHUNK: usize = 4 * 1024 * 1024;

/// How many threads may run at once, from the machine, capped so a very large
/// core count does not split the input into slices too small to pay for
/// themselves.
///
/// Work below one chunk answers without asking the operating system anything:
/// this is called once per section, and querying the core count each time cost
/// more than the entropy of a small section takes to compute.
#[must_use]
pub fn worker_count(work: usize) -> usize {
    if work <= MINIMUM_CHUNK {
        return 1;
    }
    available().min(work.div_ceil(MINIMUM_CHUNK)).max(1)
}

/// Cores usable by this process, asked once and remembered.
fn available() -> usize {
    static AVAILABLE: OnceLock<usize> = OnceLock::new();
    *AVAILABLE.get_or_init(|| thread::available_parallelism().map_or(1, std::num::NonZero::get))
}

/// Splits `items` into `parts` contiguous chunks, maps each in parallel, and
/// returns the results **in input order**.
///
/// Order is what makes the outcome independent of scheduling: whichever chunk
/// finishes first, the results are assembled the way the input was laid out.
pub fn map_chunks<T, R, F>(items: &[T], parts: usize, map: F) -> Vec<R>
where
    T: Sync,
    R: Send,
    F: Fn(&[T]) -> R + Sync,
{
    if parts <= 1 || items.is_empty() {
        return vec![map(items)];
    }
    let chunk = items.len().div_ceil(parts);
    thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|slice| scope.spawn(|| map(slice)))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|payload| std::panic::resume_unwind(payload))
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn results_come_back_in_input_order() {
        let items: Vec<u32> = (0..1000).collect();
        let sums = map_chunks(&items, 8, |chunk| chunk.iter().sum::<u32>());

        assert_eq!(sums.len(), 8);
        assert_eq!(sums.iter().sum::<u32>(), items.iter().sum::<u32>());
        // Chunks are contiguous and ordered, so the first holds the smallest
        // values and the last the largest.
        assert!(sums[0] < sums[7]);
    }

    /// The whole point: the same input must give the same answer whatever the
    /// machine, so a report can be compared against another run.
    #[test]
    fn the_split_never_changes_the_combined_result() {
        let items: Vec<u32> = (0..10_000).map(|value| value % 251).collect();
        let reference: u32 = items.iter().sum();

        for parts in [1, 2, 3, 7, 8, 64, 10_001] {
            let total: u32 = map_chunks(&items, parts, |chunk| chunk.iter().sum::<u32>())
                .into_iter()
                .sum();
            assert_eq!(total, reference, "{parts} parts changed the result");
        }
    }

    #[test]
    fn an_empty_input_still_answers_once() {
        let empty: [u32; 0] = [];
        let sums = map_chunks(&empty, 8, <[u32]>::len);
        assert_eq!(sums, vec![0]);
    }

    #[test]
    fn small_work_stays_on_one_thread() {
        assert_eq!(worker_count(0), 1);
        assert_eq!(worker_count(1024), 1);
        assert_eq!(worker_count(MINIMUM_CHUNK), 1);
        assert!(worker_count(MINIMUM_CHUNK * 64) >= 1);
    }

    /// A panic inside a chunk must surface, not be swallowed into a wrong
    /// result that would then be reported as a finding.
    #[test]
    #[should_panic(expected = "chunk failed")]
    fn a_panic_in_a_chunk_is_not_silently_lost() {
        let items: Vec<u32> = (0..100).collect();
        let _ = map_chunks(&items, 4, |chunk| {
            assert!(!chunk.contains(&50), "chunk failed");
            0
        });
    }
}
