use super::*;

fn candidates() -> Arc<Vec<folio_ide::CompletionItem>> {
    Arc::new(vec![folio_ide::CompletionItem {
        label: "Example".into(),
        detail: "Int".into(),
        kind: 6,
        symbol: None,
        replacement: TextRange { start: 0, end: 0 },
        insert_text: "Example".into(),
        documentation: None,
        receiver: None,
    }])
}

#[test]
fn resolve_uses_the_original_list_and_rejects_expired_generations() {
    let cache = CompletionCache::default();
    let id = cache.insert(3, candidates());
    assert_eq!(cache.get(id, 3, 0).unwrap().label, "Example");
    assert!(cache.get(id, 4, 0).is_none());
    assert!(cache.get(id, 3, 1).is_none());
    cache.clear();
    assert!(cache.get(id, 3, 0).is_none());
}

#[test]
fn old_lists_are_evicted_without_reusing_resolve_identifiers() {
    let cache = CompletionCache::default();
    let first = cache.insert(1, candidates());
    let mut last = first;
    for _ in 0..32 {
        last = cache.insert(1, candidates());
    }
    assert!(cache.get(first, 1, 0).is_none());
    assert!(cache.get(last, 1, 0).is_some());
    cache.clear();
    assert_ne!(cache.insert(1, candidates()), first);
}
