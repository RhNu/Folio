//! Fixed workers and a bounded queue keep request bursts from exhausting threads and memory.
use std::sync::{
    Arc, Mutex,
    mpsc::{self, SyncSender, TrySendError},
};
use std::time::{Duration, Instant};

/// At most one load runs; later edits replace the pending generation while retaining disk refreshes.
#[derive(Default)]
pub(super) struct ReloadSchedule {
    deadline: Option<Instant>,
    refresh_disk: bool,
    running: bool,
}

impl ReloadSchedule {
    pub fn request(&mut self, refresh_disk: bool, now: Instant) {
        self.refresh_disk |= refresh_disk;
        self.deadline = Some(now + Duration::from_millis(40));
    }

    pub fn take_due(&mut self, now: Instant) -> Option<bool> {
        if self.running || self.deadline.is_none_or(|deadline| deadline > now) {
            return None;
        }
        self.running = true;
        self.deadline = None;
        Some(self.refresh_disk)
    }

    pub fn finish(&mut self, published: bool) {
        self.running = false;
        if published {
            self.refresh_disk = false;
        }
    }
}

type Task = Box<dyn FnOnce() + Send>;

pub(super) struct QueryPool {
    queue: SyncSender<Task>,
}

#[cfg(test)]
mod tests;

impl QueryPool {
    pub fn new() -> Self {
        let (queue, jobs) = mpsc::sync_channel::<Task>(64);
        let jobs = Arc::new(Mutex::new(jobs));
        for index in 0..2 {
            let jobs = Arc::clone(&jobs);
            std::thread::Builder::new()
                .name(format!("folio-query-{index}"))
                .spawn(move || {
                    loop {
                        let task = { jobs.lock().expect("query queue").recv() };
                        let Ok(task) = task else {
                            break;
                        };
                        task();
                    }
                })
                .expect("start query worker");
        }
        Self { queue }
    }

    /// Submission never blocks the protocol thread; callers report overload as cancellation.
    pub fn submit(&self, task: impl FnOnce() + Send + 'static) -> bool {
        match self.queue.try_send(Box::new(task)) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => false,
        }
    }
}
