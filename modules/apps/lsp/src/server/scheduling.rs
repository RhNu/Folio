//! Fixed workers and a bounded queue keep request bursts from exhausting threads and memory.
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
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

/// Only replaceable background requests carry a key; request identities remain distinct.
pub(super) struct QueryMeta {
    pub id: String,
    pub generation: u64,
    pub interactive: bool,
    pub coalesce_key: Option<String>,
}

struct Queued<T> {
    meta: QueryMeta,
    value: T,
}

/// Selection is pure queue policy, independent of threads and response publication.
struct QueryQueue<T> {
    waiting: VecDeque<Queued<T>>,
    interactive_streak: usize,
    capacity: usize,
}

impl<T> QueryQueue<T> {
    fn new(capacity: usize) -> Self {
        Self {
            waiting: VecDeque::new(),
            interactive_streak: 0,
            capacity,
        }
    }

    /// Replacement frees capacity before admission; rejection leaves existing requests intact.
    fn push(&mut self, meta: QueryMeta, value: T) -> Result<Vec<T>, T> {
        let replaced = if let Some(key) = &meta.coalesce_key {
            self.remove_where(|old| {
                old.generation == meta.generation && old.coalesce_key.as_ref() == Some(key)
            })
        } else {
            Vec::new()
        };
        if self.waiting.len() == self.capacity {
            return Err(value);
        }
        self.waiting.push_back(Queued { meta, value });
        Ok(replaced)
    }

    /// Four interactive selections at most may pass a waiting background request.
    fn pop(&mut self) -> Option<T> {
        let interactive = self.waiting.iter().position(|item| item.meta.interactive);
        let background = self.waiting.iter().position(|item| !item.meta.interactive);
        let index = match (interactive, background) {
            (Some(interactive), Some(background)) => {
                if self.interactive_streak < 4 {
                    interactive
                } else {
                    background
                }
            }
            (Some(index), None) | (None, Some(index)) => index,
            (None, None) => return None,
        };
        let item = self.waiting.remove(index).expect("selected queued request");
        self.interactive_streak = if item.meta.interactive {
            (self.interactive_streak + 1).min(4)
        } else {
            0
        };
        Some(item.value)
    }

    fn remove_where(&mut self, predicate: impl Fn(&QueryMeta) -> bool) -> Vec<T> {
        let mut removed = Vec::new();
        let mut retained = VecDeque::with_capacity(self.waiting.len());
        for item in self.waiting.drain(..) {
            if predicate(&item.meta) {
                removed.push(item.value);
            } else {
                retained.push_back(item);
            }
        }
        self.waiting = retained;
        removed
    }
}

struct QueryTask {
    run: Task,
    cancel: Task,
}

struct PoolState {
    queue: QueryQueue<QueryTask>,
    closed: bool,
}

pub(super) struct QueryPool {
    state: Arc<(Mutex<PoolState>, Condvar)>,
}

#[cfg(test)]
mod tests;

impl QueryPool {
    pub fn new() -> Self {
        let state = Arc::new((
            Mutex::new(PoolState {
                queue: QueryQueue::new(64),
                closed: false,
            }),
            Condvar::new(),
        ));
        for index in 0..2 {
            let state = Arc::clone(&state);
            std::thread::Builder::new()
                .name(format!("folio-query-{index}"))
                .spawn(move || {
                    loop {
                        let task = {
                            let (lock, available) = &*state;
                            let mut state = lock.lock().expect("query queue");
                            loop {
                                if state.closed {
                                    return;
                                }
                                if let Some(task) = state.queue.pop() {
                                    break task;
                                }
                                state = available.wait(state).expect("query queue");
                            }
                        };
                        (task.run)();
                    }
                })
                .expect("start query worker");
        }
        Self { state }
    }

    /// No wait for capacity. Removed work only runs its lightweight response/cleanup callback.
    pub fn submit(
        &self,
        meta: QueryMeta,
        run: impl FnOnce() + Send + 'static,
        cancel: impl FnOnce() + Send + 'static,
    ) -> bool {
        let removed = {
            let mut state = self.state.0.lock().expect("query queue");
            if state.closed {
                return false;
            }
            let task = QueryTask {
                run: Box::new(run),
                cancel: Box::new(cancel),
            };
            match state.queue.push(meta, task) {
                Ok(removed) => removed,
                Err(_) => return false,
            }
        };
        self.state.1.notify_one();
        Self::cancel_removed(removed, "superseded");
        true
    }

    pub fn cancel(&self, id: &str) {
        self.remove_where(|meta| meta.id == id, "client cancellation");
    }

    pub fn retain_generation(&self, generation: u64) {
        self.remove_where(|meta| meta.generation != generation, "obsolete generation");
    }

    fn remove_where(&self, predicate: impl Fn(&QueryMeta) -> bool, reason: &'static str) {
        let removed = self
            .state
            .0
            .lock()
            .expect("query queue")
            .queue
            .remove_where(predicate);
        Self::cancel_removed(removed, reason);
    }

    fn cancel_removed(tasks: Vec<QueryTask>, reason: &'static str) {
        if !tasks.is_empty() {
            tracing::debug!(
                count = tasks.len(),
                reason,
                "cancelled queued editor requests"
            );
        }
        for task in tasks {
            (task.cancel)();
        }
    }
}

impl Drop for QueryPool {
    fn drop(&mut self) {
        let removed = {
            let mut state = self.state.0.lock().expect("query queue");
            state.closed = true;
            state.queue.remove_where(|_| true)
        };
        // Workers exit after any running request; never join while a response may await stdout.
        self.state.1.notify_all();
        Self::cancel_removed(removed, "server stopped");
    }
}
