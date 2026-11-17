//! Data-parallel maps for the bake. The browser only decodes baked assets, so it runs them on one thread.

/// Collects `work(0..len)` in order, spreading the calls over every core. Results are identical to a serial map.
#[cfg(not(target_arch = "wasm32"))]
pub fn map<T: Send>(len: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };
    // Small chunks, handed out as workers free up, balance sites whose cost varies widely.
    const CHUNK: usize = 32;
    let next = AtomicUsize::new(0);
    let workers = thread::available_parallelism().map_or(1, usize::from).min(len.div_ceil(CHUNK)).max(1);
    let mut parts: Vec<(usize, Vec<T>)> = thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let start = next.fetch_add(CHUNK, Ordering::Relaxed);
                        if start >= len {
                            break mine;
                        }
                        mine.push((start, (start..(start + CHUNK).min(len)).map(&work).collect::<Vec<_>>()));
                    }
                })
            })
            .collect();
        handles.into_iter().flat_map(|handle| handle.join().expect("bake worker panicked")).collect()
    });
    parts.sort_unstable_by_key(|part| part.0);
    parts.into_iter().flat_map(|(_, values)| values).collect()
}

#[cfg(target_arch = "wasm32")]
pub fn map<T: Send>(len: usize, work: impl Fn(usize) -> T + Sync) -> Vec<T> {
    (0..len).map(work).collect()
}

#[cfg(test)]
mod tests {
    use crate::irradiance::parallel::map;

    #[test]
    fn results_come_back_in_order_for_any_length() {
        for len in [0, 1, 31, 32, 33, 1000] {
            assert_eq!(map(len, |index| index * 3), (0..len).map(|index| index * 3).collect::<Vec<_>>());
        }
    }
}
