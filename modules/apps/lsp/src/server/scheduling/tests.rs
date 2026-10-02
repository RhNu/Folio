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
