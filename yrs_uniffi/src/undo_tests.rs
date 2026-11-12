//! The undo manager through ykt's API (the same calls the Kotlin and Swift bindings make), with the
//! web editor's setup first: tracked origins = the editor's, so remote edits are never undone. Then
//! redo, the capture timeout and `stopCapturing`, clearing, tracked and untracked origins, the scope,
//! stack item events and their meta (a selection to restore), what may not be done inside a
//! transaction or a callback (it throws, it doesn't hang), releasing the manager, and threads.

use crate::attrs::{YAttributes, YValue};
use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YSubscription, YTextEvent, YTextObserver};
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2, encode_state_vector};
use crate::text::YText;
use crate::tools::Error;
use crate::transaction::YTransaction;
use crate::undo::*;
use crate::updates::diff_update_v2;
use std::collections::HashMap;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

// Helpers ------------------------------------------------------------------------------------------

fn doc(client_id: u64) -> YDoc {
    YDoc::new(Some(YDocOptions {
        client_id: Some(client_id),
        guid: None,
        collection_id: None,
        gc: None,
        auto_load: None,
        should_load: None,
        bytes_offset: None,
    }))
}

fn manager(doc: &YDoc, text: &YText, capture_timeout_millis: u64, origins: &[&str]) -> Arc<YUndoManager> {
    let options = YUndoOptions {
        capture_timeout_millis,
        tracked_origins: origins.iter().map(|o| o.to_string()).collect(),
    };
    Arc::new(YUndoManager::new(doc, text, Some(options)).unwrap())
}

/// One transaction with an origin.
fn edit(doc: &YDoc, origin: &str, f: impl FnOnce(&Arc<YTransaction>)) {
    let txn = Arc::new(doc.transaction(Some(origin.into())).unwrap());
    f(&txn);
    txn.commit().unwrap();
}

fn insert(doc: &YDoc, text: &YText, origin: &str, index: u32, chunk: &str) {
    edit(doc, origin, |txn| text.insert(index, chunk, None, Some(txn.clone())).unwrap());
}

fn content(text: &YText) -> String {
    text.to_string(None).unwrap()
}

/// What `remote` has that `local` lacks, applied to `local` as a remote update.
fn receive(local: &YDoc, remote: &YDoc, origin: Option<&str>) {
    let missing = diff_update_v2(encode_state_as_update_v2(remote, None).unwrap(), encode_state_vector(local).unwrap()).unwrap();
    apply_update_v2(local, &missing, origin.map(|o| o.as_bytes().to_vec())).unwrap();
}

fn meta(key: &str, value: f64) -> YAttributes {
    HashMap::from([(key.to_string(), YValue::Number(value))])
}

struct ItemFn<F>(F);
impl<F: Fn(YUndoEvent) -> Option<YAttributes> + Send + Sync> YUndoItemObserver for ItemFn<F> {
    fn on_item(&self, event: YUndoEvent) -> Option<YAttributes> {
        (self.0)(event)
    }
}

struct PopFn<F>(F);
impl<F: Fn(YUndoEvent) + Send + Sync> YUndoPopObserver for PopFn<F> {
    fn on_pop(&self, event: YUndoEvent) {
        (self.0)(event)
    }
}

struct TextFn<F>(F);
impl<F: Fn(YTextEvent) + Send + Sync> YTextObserver for TextFn<F> {
    fn on_change(&self, event: YTextEvent) {
        (self.0)(event)
    }
}

// The editor's case: only its own changes are undone -----------------------------------------------

#[test]
fn undo_takes_back_the_editors_changes_and_keeps_the_remote_ones() {
    let local = doc(1);
    let text = local.get_text("t");
    let undo = manager(&local, &text, 0, &["editor"]);
    let remote = doc(2);

    insert(&local, &text, "editor", 0, "hello");
    receive(&remote, &local, None);
    remote.get_text("t").insert(5, " world", None, None).unwrap();
    remote.get_text("t").insert(2, "~", None, None).unwrap(); // inside the editor's "hello"
    receive(&local, &remote, Some("remote"));
    insert(&local, &text, "editor", 0, "> ");
    assert_eq!(content(&text), "> he~llo world");

    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "he~llo world");
    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "~ world", "only the editor's characters are gone");
    assert!(!undo.can_undo().unwrap());
    assert!(!undo.undo().unwrap(), "nothing left: no change");
    assert_eq!(content(&text), "~ world");
}

#[test]
fn remote_changes_after_the_undo_merge_with_it_on_every_replica() {
    let local = doc(1);
    let text = local.get_text("t");
    let undo = manager(&local, &text, 0, &["editor"]);
    let remote = doc(2);
    insert(&local, &text, "editor", 0, "abc");
    receive(&remote, &local, None);

    assert!(undo.undo().unwrap()); // local: ""
    remote.get_text("t").insert(3, "!", None, None).unwrap(); // remote, concurrently: "abc!"
    receive(&local, &remote, Some("remote"));
    receive(&remote, &local, Some("remote"));

    assert_eq!(content(&text), "!");
    assert_eq!(content(&remote.get_text("t")), "!");
}

// Redo, capture timeout, clear -----------------------------------------------------------------------

#[test]
fn redo_brings_back_what_undo_took() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    insert(&doc, &text, "editor", 0, "ab");
    insert(&doc, &text, "editor", 2, "cd");

    assert!(!undo.can_redo().unwrap());
    assert!(undo.undo().unwrap());
    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "");
    assert!(undo.can_redo().unwrap());
    assert!(undo.redo().unwrap());
    assert_eq!(content(&text), "ab");
    assert!(undo.redo().unwrap());
    assert_eq!(content(&text), "abcd");
    assert!(!undo.can_redo().unwrap());
    assert!(!undo.redo().unwrap());
    assert!(undo.can_undo().unwrap());
}

#[test]
fn a_new_change_drops_the_redo_stack() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    insert(&doc, &text, "editor", 0, "ab");
    undo.undo().unwrap();
    assert!(undo.can_redo().unwrap());

    insert(&doc, &text, "editor", 0, "x");

    assert!(!undo.can_redo().unwrap());
    assert_eq!(content(&text), "x");
}

#[test]
fn changes_within_the_capture_timeout_merge_and_stop_capturing_splits_them() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 3_600_000, &["editor"]);
    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");
    undo.stop_capturing().unwrap();
    insert(&doc, &text, "editor", 2, "c");
    insert(&doc, &text, "editor", 3, "d");

    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "ab", "c and d were one item");
    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "", "a and b were one item");
    assert!(!undo.can_undo().unwrap());
}

#[test]
fn a_zero_capture_timeout_makes_every_transaction_an_item() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    for (i, c) in ["a", "b", "c"].iter().enumerate() {
        insert(&doc, &text, "editor", i as u32, c);
    }

    undo.undo().unwrap();
    assert_eq!(content(&text), "ab");
    undo.undo().unwrap();
    assert_eq!(content(&text), "a");
}

#[test]
fn clear_empties_both_stacks() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");
    undo.undo().unwrap();

    undo.clear().unwrap();

    assert!(!undo.can_undo().unwrap());
    assert!(!undo.can_redo().unwrap());
    assert!(!undo.undo().unwrap());
    assert_eq!(content(&text), "a", "clearing changes nothing in the text");
}

// Origins ------------------------------------------------------------------------------------------------

#[test]
fn untracked_origins_are_ignored() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);

    insert(&doc, &text, "other", 0, "a");
    text.insert(1, "b", None, None).unwrap(); // no origin: not tracked once origins are given
    assert!(!undo.can_undo().unwrap());

    undo.add_tracked_origin("other".into()).unwrap();
    insert(&doc, &text, "other", 2, "c");
    undo.remove_tracked_origin("editor".into()).unwrap();
    insert(&doc, &text, "editor", 3, "d");

    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "abd", "only the tracked 'other' change");
    assert!(!undo.can_undo().unwrap());
}

#[test]
fn without_tracked_origins_changes_without_an_origin_are_tracked() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = Arc::new(YUndoManager::new(&doc, &text, None).unwrap());

    text.insert(0, "a", None, None).unwrap();
    undo.stop_capturing().unwrap();
    insert(&doc, &text, "editor", 1, "b");
    text.insert(2, "c", None, None).unwrap();

    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "ab");
    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "b", "the 'editor' change wasn't tracked");
}

#[test]
fn undo_transactions_carry_the_managers_origin() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let origins = Arc::new(Mutex::new(vec![]));
    let heard = origins.clone();
    let _sub = text.observe(Arc::new(TextFn(move |e: YTextEvent| heard.lock().unwrap().push(e.origin)))).unwrap();

    insert(&doc, &text, "editor", 0, "a");
    undo.undo().unwrap();
    undo.redo().unwrap();

    let own = Some(undo.origin());
    assert_eq!(*origins.lock().unwrap(), vec![Some(b"editor".to_vec()), own.clone(), own]);
    assert_ne!(undo.origin(), b"editor".to_vec());
}

// Scope ------------------------------------------------------------------------------------------------

#[test]
fn only_the_scope_is_tracked_and_it_can_grow() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let other = doc.get_text("other");
    let undo = manager(&doc, &text, 0, &["editor"]);

    insert(&doc, &other, "editor", 0, "x");
    assert!(!undo.can_undo().unwrap(), "another text isn't in the scope");

    undo.expand_scope(&other).unwrap();
    insert(&doc, &other, "editor", 1, "y");
    insert(&doc, &text, "editor", 0, "a");
    undo.undo().unwrap();
    undo.undo().unwrap();

    assert_eq!(content(&other), "x");
    assert_eq!(content(&text), "");
}

#[test]
fn a_text_of_another_document_or_a_preliminary_one_is_refused() {
    let a = doc(1);
    let b = doc(2);
    assert!(YUndoManager::new(&a, &b.get_text("t"), None).is_err());
    assert!(matches!(YUndoManager::new(&a, &YText::new_with_text(None), None), Err(Error::InvalidPrelimOp)));
    let undo = YUndoManager::new(&a, &a.get_text("t"), None).unwrap();
    assert!(undo.expand_scope(&b.get_text("t")).is_err());
}

// Stack item events and their meta --------------------------------------------------------------------

#[test]
fn meta_stored_when_an_item_is_added_comes_back_when_it_is_popped() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let added = Arc::new(Mutex::new(vec![]));
    let popped = Arc::new(Mutex::new(vec![]));
    // The selection to restore: a counter stands in for it.
    let selection = Arc::new(Mutex::new(0.0));
    let (heard, sel) = (added.clone(), selection.clone());
    let _added = undo.clone().observe_item_added(Arc::new(ItemFn(move |e: YUndoEvent| {
        heard.lock().unwrap().push(e);
        Some(meta("selection", *sel.lock().unwrap()))
    })));
    let heard = popped.clone();
    let _popped = undo.clone().observe_item_popped(Arc::new(PopFn(move |e: YUndoEvent| heard.lock().unwrap().push(e))));

    *selection.lock().unwrap() = 1.0;
    insert(&doc, &text, "editor", 0, "a");
    *selection.lock().unwrap() = 2.0;
    insert(&doc, &text, "editor", 1, "b");
    *selection.lock().unwrap() = 9.0;
    undo.undo().unwrap();
    undo.redo().unwrap();

    let own = Some(undo.origin());
    let added = added.lock().unwrap().clone();
    assert_eq!(added.len(), 4, "two changes, the undo (for redo), the redo (for undo)");
    assert_eq!((added[0].kind, added[0].origin.clone()), (YUndoKind::Undo, Some(b"editor".to_vec())));
    assert!(added[0].meta.is_empty(), "a new item has no meta yet");
    assert_eq!((added[2].kind, added[2].origin.clone()), (YUndoKind::Redo, own.clone()));
    assert_eq!((added[3].kind, added[3].origin.clone()), (YUndoKind::Undo, own.clone()));
    let popped = popped.lock().unwrap().clone();
    assert_eq!(
        popped,
        vec![
            YUndoEvent { kind: YUndoKind::Undo, origin: own.clone(), meta: meta("selection", 2.0) },
            YUndoEvent { kind: YUndoKind::Redo, origin: own, meta: meta("selection", 9.0) },
        ]
    );
}

#[test]
fn an_extended_item_is_an_update_event_and_its_meta_can_change() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 3_600_000, &["editor"]);
    let updated = Arc::new(Mutex::new(vec![]));
    let popped = Arc::new(Mutex::new(vec![]));
    let _added = undo.clone().observe_item_added(Arc::new(ItemFn(|_| Some(meta("first", 1.0)))));
    let heard = updated.clone();
    let _updated = undo.clone().observe_item_updated(Arc::new(ItemFn(move |e: YUndoEvent| {
        heard.lock().unwrap().push(e.meta.clone());
        let mut m = e.meta;
        m.insert("last".into(), YValue::Number(2.0));
        Some(m)
    })));
    let heard = popped.clone();
    let _popped = undo.clone().observe_item_popped(Arc::new(PopFn(move |e: YUndoEvent| heard.lock().unwrap().push(e.meta))));

    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");
    undo.undo().unwrap();

    assert_eq!(*updated.lock().unwrap(), vec![meta("first", 1.0)]);
    let mut both = meta("first", 1.0);
    both.insert("last".into(), YValue::Number(2.0));
    assert_eq!(*popped.lock().unwrap(), vec![both]);
}

#[test]
fn returning_no_meta_keeps_what_the_item_has() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 3_600_000, &["editor"]);
    let popped = Arc::new(Mutex::new(vec![]));
    let _added = undo.clone().observe_item_added(Arc::new(ItemFn(|_| Some(meta("kept", 1.0)))));
    let _updated = undo.clone().observe_item_updated(Arc::new(ItemFn(|_| None)));
    let heard = popped.clone();
    let _popped = undo.clone().observe_item_popped(Arc::new(PopFn(move |e: YUndoEvent| heard.lock().unwrap().push(e.meta))));

    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");
    undo.undo().unwrap();

    assert_eq!(*popped.lock().unwrap(), vec![meta("kept", 1.0)]);
}

#[test]
fn subscriptions_stop_on_unsubscribe_and_release() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let calls = Arc::new(Mutex::new(0));
    let c = calls.clone();
    let sub = undo.clone().observe_item_added(Arc::new(ItemFn(move |_| {
        *c.lock().unwrap() += 1;
        None
    }))).unwrap();
    let c = calls.clone();
    let released = undo.clone().observe_item_added(Arc::new(ItemFn(move |_| {
        *c.lock().unwrap() += 10;
        None
    }))).unwrap();

    insert(&doc, &text, "editor", 0, "a"); // both: 11
    assert!(sub.unsubscribe());
    assert!(!sub.unsubscribe());
    insert(&doc, &text, "editor", 1, "b"); // 10 more
    drop(released);
    insert(&doc, &text, "editor", 2, "c"); // none

    assert_eq!(*calls.lock().unwrap(), 21);
}

// What may not be done inside a transaction or a callback -------------------------------------------

#[test]
fn undo_redo_and_clear_inside_a_transaction_throw() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    insert(&doc, &text, "editor", 0, "a");
    let txn = doc.transaction(Some("editor".into())).unwrap();

    assert!(matches!(undo.undo(), Err(Error::AnotherTx)));
    assert!(matches!(undo.redo(), Err(Error::AnotherTx)));
    assert!(matches!(undo.clear(), Err(Error::AnotherTx)));
    assert!(undo.can_undo().unwrap(), "reading the stacks is fine");
    assert!(YUndoManager::new(&doc, &text, None).is_err(), "so is making a manager");

    txn.commit().unwrap();
    assert!(undo.undo().unwrap());
    assert_eq!(content(&text), "");
}

#[test]
fn undo_inside_a_text_observer_throws() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let results = Arc::new(Mutex::new(vec![]));
    let (u, r) = (undo.clone(), results.clone());
    let _sub = text.observe(Arc::new(TextFn(move |_| {
        r.lock().unwrap().push(matches!(u.undo(), Err(Error::AnotherTx)));
    }))).unwrap();

    insert(&doc, &text, "editor", 0, "a");

    assert_eq!(*results.lock().unwrap(), vec![true]);
    assert_eq!(content(&text), "a");
    drop(_sub);
    assert!(undo.undo().unwrap(), "fine outside the callback");
}

#[test]
fn the_manager_cannot_be_used_inside_its_own_callbacks() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let results = Arc::new(Mutex::new(vec![]));
    let (u, r) = (Arc::downgrade(&undo), results.clone());
    let _added = undo.clone().observe_item_added(Arc::new(ItemFn(move |_| {
        let u = u.upgrade().unwrap();
        let mut r = r.lock().unwrap();
        r.push(("added: can_undo", u.can_undo().is_err()));
        r.push(("added: undo", u.undo().is_err()));
        r.push(("added: stop_capturing", u.stop_capturing().is_err()));
        None
    })));
    let (u, r) = (Arc::downgrade(&undo), results.clone());
    let _popped = undo.clone().observe_item_popped(Arc::new(PopFn(move |_| {
        let u = u.upgrade().unwrap();
        let mut r = r.lock().unwrap();
        r.push(("popped: redo", u.redo().is_err()));
        r.push(("popped: can_redo", u.can_redo().is_err()));
    })));

    insert(&doc, &text, "editor", 0, "a");
    results.lock().unwrap().clear();
    undo.undo().unwrap();

    let results = results.lock().unwrap().clone();
    assert!(results.iter().all(|(_, threw)| *threw), "{results:?}");
    assert!(results.iter().any(|(what, _)| what.starts_with("popped")));
    assert_eq!(content(&text), "");
    assert!(undo.can_redo().unwrap(), "and the manager is fine after");
}

/// Unsubscribes itself on its first call, inside the manager's work.
#[test]
fn unsubscribing_inside_a_callback_stops_it() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let calls = Arc::new(Mutex::new(0));
    let holder: Arc<Mutex<Option<Arc<YSubscription>>>> = Arc::new(Mutex::new(None));
    let (c, h) = (calls.clone(), holder.clone());
    let sub = undo.clone().observe_item_added(Arc::new(ItemFn(move |_| {
        *c.lock().unwrap() += 1;
        if let Some(sub) = h.lock().unwrap().take() {
            assert!(sub.unsubscribe());
        }
        None
    }))).unwrap();
    *holder.lock().unwrap() = Some(sub);

    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");

    assert_eq!(*calls.lock().unwrap(), 1);
    assert!(undo.undo().unwrap());
}

// Releasing the manager ------------------------------------------------------------------------------

#[test]
fn a_released_manager_stops_tracking() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    let sub = undo.clone().observe_item_added(Arc::new(ItemFn(|_| None))).unwrap();
    insert(&doc, &text, "editor", 0, "a");

    drop(undo);
    insert(&doc, &text, "editor", 1, "b");

    assert_eq!(content(&text), "ab");
    assert!(sub.unsubscribe(), "a subscription may outlive its manager");
}

#[test]
fn releasing_the_manager_while_the_document_is_locked_is_safe() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let undo = manager(&doc, &text, 0, &["editor"]);
    insert(&doc, &text, "editor", 0, "a");

    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    text.insert(1, "b", None, Some(txn.clone())).unwrap();
    drop(undo); // can't leave the document now: it stays, harmlessly
    txn.commit().unwrap();
    insert(&doc, &text, "editor", 2, "c");

    assert_eq!(content(&text), "abc");
}

#[test]
fn releasing_the_manager_inside_a_callback_is_safe() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let holder = Arc::new(Mutex::new(Some(manager(&doc, &text, 0, &["editor"]))));
    let h = holder.clone();
    let _sub = text.observe(Arc::new(TextFn(move |_| {
        h.lock().unwrap().take(); // the last reference
    }))).unwrap();

    insert(&doc, &text, "editor", 0, "a");
    insert(&doc, &text, "editor", 1, "b");

    assert!(holder.lock().unwrap().is_none());
    assert_eq!(content(&text), "ab");
}

// Threads ----------------------------------------------------------------------------------------------

/// ykt's contract: one call at a time, from any thread. Eight threads share a document and its undo
/// manager behind a lock, edit, then undo everything and redo it.
#[test]
fn a_document_and_its_manager_shared_behind_a_lock() {
    let doc = Arc::new(doc(1));
    let text = Arc::new(doc.get_text("t"));
    let undo = manager(&doc, &text, 0, &["editor"]);
    let lock = Arc::new(Mutex::new(()));
    let threads = 8;
    let per_thread = 20;
    let start = Arc::new(Barrier::new(threads));

    let handles: Vec<_> = (0..threads)
        .map(|i| {
            let (doc, text, undo, lock, start) = (doc.clone(), text.clone(), undo.clone(), lock.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                for n in 0..per_thread {
                    let _guard = lock.lock().unwrap();
                    insert(&doc, &text, "editor", 0, "x");
                    if (i + n) % 5 == 0 {
                        undo.stop_capturing().unwrap();
                        assert!(undo.can_undo().unwrap());
                    }
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let total = threads * per_thread;
    assert_eq!(text.length(None).unwrap() as usize, total);
    let undos = thread::spawn({
        let (undo, lock) = (undo.clone(), lock.clone());
        move || {
            let _guard = lock.lock().unwrap();
            let mut n = 0;
            while undo.undo().unwrap() {
                n += 1;
            }
            n
        }
    })
    .join()
    .unwrap();
    assert_eq!(undos, total, "a zero capture timeout: one item per transaction");
    assert_eq!(content(&text), "");
    while undo.redo().unwrap() {}
    assert_eq!(text.length(None).unwrap() as usize, total);
}
