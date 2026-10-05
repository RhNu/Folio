use super::*;

#[test]
fn edits_coalesce_and_wait_for_the_running_generation() {
    let mut schedule = ReloadSchedule::default();
    let now = Instant::now();
    schedule.request(false, now);
    schedule.request(false, now);
    assert_eq!(schedule.take_due(now), None);
    assert_eq!(schedule.take_due(now + Duration::from_secs(1)), Some(false));
    schedule.request(false, now);
    assert_eq!(schedule.take_due(now + Duration::from_secs(2)), None);
    schedule.finish(false);
    assert_eq!(schedule.take_due(now + Duration::from_secs(2)), Some(false));
    schedule.finish(true);
    assert_eq!(schedule.take_due(now + Duration::from_secs(3)), None);
}

#[test]
fn cancelled_disk_refresh_survives_a_newer_buffer_only_request() {
    let mut schedule = ReloadSchedule::default();
    let now = Instant::now();
    schedule.request(true, now);
    assert_eq!(schedule.take_due(now + Duration::from_secs(1)), Some(true));
    schedule.request(false, now);
    schedule.finish(false);
    assert_eq!(schedule.take_due(now + Duration::from_secs(2)), Some(true));
    schedule.finish(true);
    schedule.request(false, now);
    assert_eq!(schedule.take_due(now + Duration::from_secs(3)), Some(false));
}

fn request(id: &str, generation: u64, interactive: bool, key: Option<&str>) -> QueryMeta {
    QueryMeta {
        id: id.into(),
        generation,
        interactive,
        coalesce_key: key.map(str::to_owned),
    }
}

#[test]
fn interactive_requests_pass_background_without_starving_it() {
    let mut queue = QueryQueue::new(8);
    assert!(queue.push(request("b1", 1, false, None), "b1").is_ok());
    assert!(queue.push(request("b2", 1, false, None), "b2").is_ok());
    for id in ["i1", "i2", "i3", "i4", "i5"] {
        assert!(queue.push(request(id, 1, true, None), id).is_ok());
    }
    let mut responses = Vec::new();
    while let Some(id) = queue.pop() {
        responses.push(id);
    }
    assert_eq!(responses, ["i1", "i2", "i3", "i4", "b1", "i5", "b2"]);
}

#[test]
fn replacement_admits_latest_work_at_capacity_and_preserves_other_requests() {
    let mut queue = QueryQueue::new(2);
    assert!(
        queue
            .push(request("old", 1, false, Some("tokens:a")), "old")
            .is_ok()
    );
    assert!(
        queue
            .push(request("other", 1, false, Some("tokens:b")), "other")
            .is_ok()
    );
    assert_eq!(
        queue.push(request("new", 1, false, Some("tokens:a")), "new"),
        Ok(vec!["old"])
    );
    assert_eq!(
        queue.push(request("extra", 1, true, None), "extra"),
        Err("extra")
    );
    assert_eq!(queue.pop(), Some("other"));
    assert_eq!(queue.pop(), Some("new"));
    assert_eq!(queue.pop(), None);
}

#[test]
fn explicit_cancellation_and_generation_changes_only_remove_waiting_work() {
    let mut queue = QueryQueue::new(5);
    for (id, generation) in [("running", 1), ("cancel", 2), ("old", 1), ("current", 2)] {
        assert!(queue.push(request(id, generation, true, None), id).is_ok());
    }
    assert_eq!(queue.pop(), Some("running"));
    assert!(queue.remove_where(|meta| meta.id == "running").is_empty());
    assert_eq!(queue.remove_where(|meta| meta.id == "cancel"), ["cancel"]);
    assert_eq!(queue.remove_where(|meta| meta.generation != 2), ["old"]);
    assert_eq!(queue.pop(), Some("current"));
    assert_eq!(queue.pop(), None);
}

#[test]
fn independent_requests_and_different_generations_do_not_coalesce() {
    let mut queue = QueryQueue::new(4);
    for (id, generation, key) in [
        ("first", 1, None),
        ("second", 1, None),
        ("old", 1, Some("hints:a")),
        ("new", 2, Some("hints:a")),
    ] {
        assert_eq!(
            queue.push(request(id, generation, false, key), id),
            Ok(vec![])
        );
    }
    assert_eq!(
        queue.remove_where(|_| true),
        ["first", "second", "old", "new"]
    );
    assert_eq!(queue.pop(), None);
}

fn recorded_task(id: &'static str, events: &Arc<Mutex<Vec<(&'static str, bool)>>>) -> QueryTask {
    let executed = Arc::clone(events);
    let cancelled = Arc::clone(events);
    QueryTask {
        run: Box::new(move || executed.lock().unwrap().push((id, true))),
        cancel: Box::new(move || cancelled.lock().unwrap().push((id, false))),
    }
}

#[test]
fn accepted_work_executes_or_cancels_exactly_once_and_rejection_runs_neither() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let mut queue = QueryQueue::new(2);
    assert!(
        queue
            .push(
                request("old", 1, false, Some("tokens:a")),
                recorded_task("old", &events)
            )
            .is_ok()
    );
    let removed = queue
        .push(
            request("new", 1, false, Some("tokens:a")),
            recorded_task("new", &events),
        )
        .ok()
        .unwrap();
    QueryPool::cancel_removed(removed, "test replacement");
    assert!(
        queue
            .push(
                request("waiting", 1, false, None),
                recorded_task("waiting", &events)
            )
            .is_ok()
    );
    assert!(
        queue
            .push(
                request("rejected", 1, true, None),
                recorded_task("rejected", &events)
            )
            .is_err()
    );
    (queue.pop().unwrap().run)();
    QueryPool::cancel_removed(queue.remove_where(|_| true), "test shutdown");
    assert_eq!(
        *events.lock().unwrap(),
        [("old", false), ("new", true), ("waiting", false)]
    );
}
