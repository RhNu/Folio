//! A bounded session cache resolves the selected candidate without rebuilding its list.
use super::*;
use std::collections::VecDeque;

#[derive(Clone, Default)]
pub(in crate::server) struct CompletionCache(Arc<Mutex<State>>);

#[derive(Default)]
struct State {
    next: u64,
    lists: VecDeque<(u64, u64, Arc<Vec<folio_ide::CompletionItem>>)>,
}

impl CompletionCache {
    pub fn clear(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.lists.clear();
        }
    }

    pub fn insert(&self, generation: u64, items: Arc<Vec<folio_ide::CompletionItem>>) -> u64 {
        let mut state = self.0.lock().expect("completion cache");
        state.next += 1;
        let id = state.next;
        state.lists.push_back((id, generation, items));
        while state.lists.len() > 8 {
            state.lists.pop_front();
        }
        id
    }

    pub fn get(&self, id: u64, generation: u64, index: usize) -> Option<folio_ide::CompletionItem> {
        let state = self.0.lock().ok()?;
        state
            .lists
            .iter()
            .find(|(candidate, revision, _)| *candidate == id && *revision == generation)?
            .2
            .get(index)
            .cloned()
    }
}

#[cfg(test)]
mod tests;
