use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

use super::*;

#[test]
fn concurrent_readers_compute_once_and_share_the_result() {
    let cache = Arc::new(SingleFlight::default());
    let computations = Arc::new(AtomicUsize::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner_cache = Arc::clone(&cache);
        let owner_computations = Arc::clone(&computations);
        let owner = scope.spawn(move || {
            owner_cache
                .warm(&|| false, || {
                    owner_computations.fetch_add(1, Ordering::SeqCst);
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(41)
                })
                .copied()
        });
        started_rx.recv().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let checks = AtomicUsize::new(0);
        let waiter_cache = Arc::clone(&cache);
        let waiter_computations = Arc::clone(&computations);
        let waiter = scope.spawn(move || {
            waiter_cache
                .warm(
                    &|| {
                        if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                            entered_tx.send(()).unwrap();
                        }
                        false
                    },
                    || {
                        waiter_computations.fetch_add(1, Ordering::SeqCst);
                        Ok(99)
                    },
                )
                .copied()
        });
        entered_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), Ok(41));
        assert_eq!(waiter.join().unwrap(), Ok(41));
    });
    assert_eq!(computations.load(Ordering::SeqCst), 1);
}

#[test]
fn waiter_cancellation_does_not_cancel_the_owner() {
    let cache = Arc::new(SingleFlight::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner_cache = Arc::clone(&cache);
        let owner = scope.spawn(move || {
            owner_cache
                .warm(&|| false, || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(7)
                })
                .copied()
        });
        started_rx.recv().unwrap();
        let checks = AtomicUsize::new(0);
        assert_eq!(
            cache
                .warm(&|| checks.fetch_add(1, Ordering::SeqCst) > 0, || {
                    panic!("a waiting reader must not compute while the owner runs")
                })
                .copied(),
            Err(AnalysisCancelled)
        );
        release_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), Ok(7));
    });
    assert_eq!(cache.get(), Some(&7));
}

#[test]
fn waiting_reader_takes_over_after_owner_cancellation() {
    let cache = Arc::new(SingleFlight::default());
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    std::thread::scope(|scope| {
        let owner_cache = Arc::clone(&cache);
        let owner = scope.spawn(move || {
            owner_cache
                .warm(&|| false, || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Err::<u32, _>(AnalysisCancelled)
                })
                .copied()
        });
        started_rx.recv().unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let checks = AtomicUsize::new(0);
        let waiter_cache = Arc::clone(&cache);
        let waiter = scope.spawn(move || {
            waiter_cache
                .warm(
                    &|| {
                        if checks.fetch_add(1, Ordering::SeqCst) == 1 {
                            entered_tx.send(()).unwrap();
                        }
                        false
                    },
                    || Ok(5),
                )
                .copied()
        });
        entered_rx.recv().unwrap();
        release_tx.send(()).unwrap();
        assert_eq!(owner.join().unwrap(), Err(AnalysisCancelled));
        assert_eq!(waiter.join().unwrap(), Ok(5));
    });
}

#[test]
fn cancelled_or_panicking_owners_allow_retry() {
    let cache = SingleFlight::default();
    assert_eq!(
        cache.warm(&|| false, || Err::<u32, _>(AnalysisCancelled)),
        Err(AnalysisCancelled)
    );
    assert!(cache.get().is_none());
    assert!(
        std::panic::catch_unwind(|| cache.warm(&|| false, || panic!("failed computation")))
            .is_err()
    );
    assert_eq!(cache.warm(&|| false, || Ok(12)), Ok(&12));
}

#[test]
fn cancellation_before_publication_discards_completed_work() {
    let cache = SingleFlight::default();
    let checks = AtomicUsize::new(0);
    assert_eq!(
        cache.warm(&|| checks.fetch_add(1, Ordering::SeqCst) > 0, || Ok(3)),
        Err(AnalysisCancelled)
    );
    assert!(cache.get().is_none());
    assert_eq!(cache.warm(&|| false, || Ok(4)), Ok(&4));
}
