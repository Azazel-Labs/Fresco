#[path = "support/common.rs"]
mod common;

#[test]
fn parallel_tests_with_the_same_suffix_have_distinct_paths() {
    let barrier = std::sync::Barrier::new(16);
    let paths = std::thread::scope(|scope| {
        let workers: [_; 16] = std::array::from_fn(|_| {
            scope.spawn(|| {
                barrier.wait();
                (0..128)
                    .map(|_| common::unique_temp_path("shared_suffix"))
                    .collect::<Vec<_>>()
            })
        });
        workers
            .into_iter()
            .flat_map(|worker| worker.join().expect("path worker panicked"))
            .collect::<Vec<_>>()
    });
    let unique = paths.iter().collect::<std::collections::HashSet<_>>();
    assert_eq!(
        unique.len(),
        paths.len(),
        "concurrent tests must never share an input path"
    );
}
