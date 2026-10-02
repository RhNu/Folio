//! Cancellable single-flight initialization for immutable semantic results.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

use crate::AnalysisCancelled;

pub(crate) struct SingleFlight<T> {
    value: OnceLock<T>,
    running: Mutex<bool>,
    ready: Condvar,
}

impl<T> Default for SingleFlight<T> {
    fn default() -> Self {
        Self {
            value: OnceLock::new(),
            running: Mutex::new(false),
            ready: Condvar::new(),
        }
    }
}

impl<T> SingleFlight<T> {
    pub(crate) fn get(&self) -> Option<&T> {
        self.value.get()
    }

    /// Only one caller computes; cancellation releases ownership for a later retry.
    pub(crate) fn warm(
        &self,
        cancelled: &dyn Fn() -> bool,
        compute: impl FnOnce() -> Result<T, AnalysisCancelled>,
    ) -> Result<&T, AnalysisCancelled> {
        loop {
            if cancelled() {
                return Err(AnalysisCancelled);
            }
            if let Some(value) = self.get() {
                return Ok(value);
            }
            let mut running = self
                .running
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(value) = self.value.get() {
                return Ok(value);
            }
            if !*running {
                *running = true;
                break;
            }
            // Waiters must observe their own cancellation even if the owner keeps running.
            let _ = self
                .ready
                .wait_timeout(running, Duration::from_millis(10))
                .unwrap_or_else(|error| error.into_inner());
        }
        let _ownership = Running(self);
        tracing::debug!("claimed semantic initialization");
        let value =
            compute().inspect_err(|_| tracing::debug!("semantic initialization cancelled"))?;
        if cancelled() {
            tracing::debug!("discarded cancelled semantic initialization");
            return Err(AnalysisCancelled);
        }
        let _ = self.value.set(value);
        Ok(self
            .value
            .get()
            .expect("single-flight result was published"))
    }
}

/// Wake waiters after success, cancellation, or unwinding from a failed computation.
struct Running<'a, T>(&'a SingleFlight<T>);

impl<T> Drop for Running<'_, T> {
    fn drop(&mut self) {
        *self
            .0
            .running
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = false;
        self.0.ready.notify_all();
    }
}

#[cfg(test)]
mod tests;
