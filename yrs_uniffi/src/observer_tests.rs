//! Observers through ykt's API (the same calls the Kotlin and Swift bindings make): text deltas and
//! origins, document updates and cleanups, unsubscribing (twice, by release, inside a callback,
//! after the document is gone), what a callback may not do (read, write, subscribe: it must fail,
//! not deadlock), and threads (callbacks on the committing thread; a document shared behind a
//! lock; two replicas on two threads syncing through their update events).

use crate::attrs::{YAttributes, YValue};
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::observers::*;
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2};
use crate::text::YText;
use std::collections::HashMap;
use std::sync::mpsc::channel;
use std::sync::{Arc, Barrier, Mutex};
use std::thread::{self, ThreadId};

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

fn insert(text: &str) -> YDelta {
    YDelta::YInsert(YValue::String(text.to_string()), None)
}

fn bold() -> YAttributes {
    HashMap::from([("bold".to_string(), YValue::Bool(true))])
}

/// Text events as they came: delta, origin, and the thread the callback ran on.
#[derive(Default)]
struct TextRecorder {
    events: Mutex<Vec<(Vec<YDelta>, Option<Vec<u8>>, ThreadId)>>,
}

impl YTextObserver for TextRecorder {
    fn on_change(&self, event: YTextEvent) {
        self.events.lock().unwrap().push((event.delta, event.origin, thread::current().id()));
    }
}

impl TextRecorder {
    fn deltas(&self) -> Vec<Vec<YDelta>> {
        self.events.lock().unwrap().iter().map(|e| e.0.clone()).collect()
    }
    fn origins(&self) -> Vec<Option<Vec<u8>>> {
        self.events.lock().unwrap().iter().map(|e| e.1.clone()).collect()
    }
    fn count(&self) -> usize {
        self.events.lock().unwrap().len()
    }
}

#[derive(Default)]
struct UpdateRecorder {
    events: Mutex<Vec<YUpdateEvent>>,
}

impl YUpdateObserver for UpdateRecorder {
    fn on_update(&self, event: YUpdateEvent) {
        self.events.lock().unwrap().push(event);
    }
}

#[derive(Default)]
struct CleanupRecorder {
    events: Mutex<Vec<YTransactionCleanupEvent>>,
}

impl YTransactionCleanupObserver for CleanupRecorder {
    fn on_cleanup(&self, event: YTransactionCleanupEvent) {
        self.events.lock().unwrap().push(event);
    }
}

// Text: what the delta says ------------------------------------------------------------------------

#[test]
fn insert_is_one_event_with_an_insert() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.insert(0, "hello", None, None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![insert("hello")]]);
    assert_eq!(rec.origins(), vec![None]);
}

#[test]
fn insert_in_the_middle_retains_up_to_it() {
    let doc = doc(1);
    let text = doc.get_text("t");
    text.insert(0, "hello", None, None).unwrap();
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.insert(5, " world", None, None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![YDelta::YRetain(5, None), insert(" world")]]);
}

#[test]
fn delete_retains_then_deletes() {
    let doc = doc(1);
    let text = doc.get_text("t");
    text.insert(0, "hello", None, None).unwrap();
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.delete(1, 3, None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![YDelta::YRetain(1, None), YDelta::YDelete(3)]]);
    assert_eq!(text.to_string(None).unwrap(), "ho");
}

#[test]
fn format_is_a_retain_with_attributes_and_insert_carries_them() {
    let doc = doc(1);
    let text = doc.get_text("t");
    text.insert(0, "hello", None, None).unwrap();
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.format(0, 5, r#"{"bold":true}"#.to_string(), None).unwrap();
    text.insert(5, "!", Some(r#"{"bold":true}"#.to_string()), None).unwrap();

    assert_eq!(
        rec.deltas(),
        vec![
            vec![YDelta::YRetain(5, Some(bold()))],
            vec![YDelta::YRetain(5, None), YDelta::YInsert(YValue::String("!".into()), Some(bold()))],
        ]
    );
}

#[test]
fn removing_an_attribute_sends_null() {
    let doc = doc(1);
    let text = doc.get_text("t");
    text.insert(0, "hello", Some(r#"{"bold":true}"#.to_string()), None).unwrap();
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.format(0, 5, r#"{"bold":null}"#.to_string(), None).unwrap();

    let null = HashMap::from([("bold".to_string(), YValue::Null)]);
    assert_eq!(rec.deltas(), vec![vec![YDelta::YRetain(5, Some(null))]]);
}

#[test]
fn offsets_are_utf16_units() {
    let doc = doc(1);
    let text = doc.get_text("t");
    text.insert(0, "😀a", None, None).unwrap(); // 😀 is two UTF-16 units
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    text.insert(3, "b", None, None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![YDelta::YRetain(3, None), insert("b")]]);
}

#[test]
fn an_embedded_value_comes_back_as_it_was_inserted() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();
    let mention = YValue::AttrMap(HashMap::from([("mention".to_string(), YValue::String("note://1".into()))]));

    text.apply_delta(vec![insert("a"), YDelta::YInsert(mention.clone(), None)], None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![insert("a"), YDelta::YInsert(mention, None)]]);
}

// Text: transactions and origins -------------------------------------------------------------------

#[test]
fn a_transaction_is_one_event_with_its_origin() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    text.insert(0, "ab", None, Some(txn.clone())).unwrap();
    text.insert(2, "cd", None, Some(txn.clone())).unwrap();
    text.delete(0, 1, Some(txn.clone())).unwrap();
    assert_eq!(rec.count(), 0, "nothing before the commit");
    txn.commit().unwrap();

    assert_eq!(rec.deltas(), vec![vec![insert("bcd")]]);
    assert_eq!(rec.origins(), vec![Some(b"editor".to_vec())]);
}

#[test]
fn a_transaction_without_changes_or_with_changes_elsewhere_sends_nothing() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let other = doc.get_text("other");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    doc.transaction(Some("empty".into())).unwrap().commit().unwrap();
    other.insert(0, "elsewhere", None, None).unwrap();

    assert_eq!(rec.count(), 0);
}

#[test]
fn a_remote_update_is_an_event_with_the_origin_it_was_applied_with() {
    let a = doc(1);
    a.get_text("t").insert(0, "hello", None, None).unwrap();
    let b = doc(2);
    let text = b.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    let update = encode_state_as_update_v2(&a, None).unwrap();
    apply_update_v2(&b, &update, Some(b"server".to_vec())).unwrap();
    // The same update again changes nothing: no event.
    apply_update_v2(&b, &update, Some(b"server".to_vec())).unwrap();

    assert_eq!(rec.deltas(), vec![vec![insert("hello")]]);
    assert_eq!(rec.origins(), vec![Some(b"server".to_vec())]);
}

#[test]
fn concurrent_edits_merge_into_one_delta_for_the_observer() {
    let a = doc(1);
    let b = doc(2);
    a.get_text("t").insert(0, "base", None, None).unwrap();
    apply_update_v2(&b, &encode_state_as_update_v2(&a, None).unwrap(), None).unwrap();
    // Both edit the same text at once.
    a.get_text("t").insert(0, "A", None, None).unwrap();
    b.get_text("t").insert(4, "B", None, None).unwrap();
    let text = b.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();

    apply_update_v2(&b, &encode_state_as_update_v2(&a, None).unwrap(), None).unwrap();

    assert_eq!(rec.deltas(), vec![vec![insert("A")]]);
    assert_eq!(text.to_string(None).unwrap(), "AbaseB");
}

// Text: subscriptions ------------------------------------------------------------------------------

#[test]
fn two_observers_both_hear_and_one_can_leave() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let first = Arc::new(TextRecorder::default());
    let second = Arc::new(TextRecorder::default());
    let first_sub = text.observe(first.clone()).unwrap();
    let _second_sub = text.observe(second.clone()).unwrap();

    text.insert(0, "a", None, None).unwrap();
    assert!(first_sub.unsubscribe());
    text.insert(1, "b", None, None).unwrap();

    assert_eq!(first.count(), 1);
    assert_eq!(second.count(), 2);
}

#[test]
fn unsubscribing_twice_says_so_and_stops_for_good() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    let sub = text.observe(rec.clone()).unwrap();
    assert!(sub.is_active());

    assert!(sub.unsubscribe());
    assert!(!sub.unsubscribe());
    assert!(!sub.is_active());
    text.insert(0, "a", None, None).unwrap();

    assert_eq!(rec.count(), 0);
}

#[test]
fn releasing_the_subscription_unsubscribes() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let rec = Arc::new(TextRecorder::default());
    {
        let _sub = text.observe(rec.clone()).unwrap();
        text.insert(0, "a", None, None).unwrap();
    }
    text.insert(1, "b", None, None).unwrap();

    assert_eq!(rec.count(), 1);
}

#[test]
fn the_subscription_may_outlive_the_document() {
    let rec = Arc::new(TextRecorder::default());
    let sub = {
        let doc = doc(1);
        let text = doc.get_text("t");
        let sub = text.observe(rec.clone()).unwrap();
        text.insert(0, "a", None, None).unwrap();
        sub
    }; // the document and the text handle are gone here; the subscription keeps the store
    assert!(sub.unsubscribe());
    drop(sub);

    assert_eq!(rec.count(), 1);
}

#[test]
fn a_preliminary_text_cannot_be_observed() {
    let text = YText::new_with_text(Some("prelim".into()));
    assert!(text.observe(Arc::new(TextRecorder::default())).is_err());
}

// Text: what a callback may not do (it must fail, not deadlock or crash) ---------------------------

struct Misbehaving {
    text: YText,
    results: Mutex<Vec<(&'static str, bool)>>,
}

impl YTextObserver for Misbehaving {
    fn on_change(&self, _event: YTextEvent) {
        let read = self.text.to_string(None).is_err();
        let write = self.text.insert(0, "x", None, None).is_err();
        let subscribe = self.text.observe(Arc::new(TextRecorder::default())).is_err();
        let mut results = self.results.lock().unwrap();
        results.push(("read fails", read));
        results.push(("write fails", write));
        results.push(("subscribe fails", subscribe));
    }
}

#[test]
fn reading_writing_or_subscribing_inside_a_callback_fails() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let observer = Arc::new(Misbehaving { text: doc.get_text("t"), results: Mutex::new(vec![]) });
    let _sub = text.observe(observer.clone()).unwrap();

    text.insert(0, "a", None, None).unwrap();

    let results = observer.results.lock().unwrap().clone();
    assert_eq!(results, vec![("read fails", true), ("write fails", true), ("subscribe fails", true)]);
    // The document is fine afterwards: nothing was written by the callback.
    assert_eq!(text.to_string(None).unwrap(), "a");
}

/// Unsubscribes itself on its first event.
struct OneShot {
    sub: Mutex<Option<Arc<YSubscription>>>,
    calls: Mutex<usize>,
}

impl YTextObserver for OneShot {
    fn on_change(&self, _event: YTextEvent) {
        *self.calls.lock().unwrap() += 1;
        if let Some(sub) = self.sub.lock().unwrap().take() {
            assert!(sub.unsubscribe());
        }
    }
}

#[test]
fn unsubscribing_inside_its_own_callback_stops_it() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let observer = Arc::new(OneShot { sub: Mutex::new(None), calls: Mutex::new(0) });
    let other = Arc::new(TextRecorder::default());
    let sub = text.observe(observer.clone()).unwrap();
    let _other_sub = text.observe(other.clone()).unwrap();
    *observer.sub.lock().unwrap() = Some(sub);

    text.insert(0, "a", None, None).unwrap();
    text.insert(1, "b", None, None).unwrap();

    assert_eq!(*observer.calls.lock().unwrap(), 1);
    assert_eq!(other.count(), 2, "the other observer goes on");
}

// Document: updates and cleanups -------------------------------------------------------------------

#[test]
fn update_events_replay_on_another_replica() {
    for v2 in [false, true] {
        let a = doc(1);
        let b = doc(2);
        let rec = Arc::new(UpdateRecorder::default());
        let _sub = if v2 { a.observe_update_v2(rec.clone()) } else { a.observe_update_v1(rec.clone()) }.unwrap();
        let text = a.get_text("t");

        text.insert(0, "hello", None, None).unwrap();
        let txn = Arc::new(a.transaction(Some("editor".into())).unwrap());
        text.insert(5, " world", None, Some(txn.clone())).unwrap();
        text.delete(0, 1, Some(txn.clone())).unwrap();
        txn.commit().unwrap();

        let events = rec.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].origin, None);
        assert_eq!(events[1].origin, Some(b"editor".to_vec()));
        for event in events.iter() {
            if v2 {
                apply_update_v2(&b, &event.update, None).unwrap();
            } else {
                crate::snapshots::apply_update(&b, &event.update, None).unwrap();
            }
        }
        assert_eq!(b.get_text("t").to_string(None).unwrap(), "ello world");
    }
}

#[test]
fn applying_a_remote_update_is_an_update_event_too() {
    let a = doc(1);
    a.get_text("t").insert(0, "hi", None, None).unwrap();
    let b = doc(2);
    let rec = Arc::new(UpdateRecorder::default());
    let _sub = b.observe_update_v2(rec.clone()).unwrap();

    apply_update_v2(&b, &encode_state_as_update_v2(&a, None).unwrap(), Some(b"server".to_vec())).unwrap();

    let events = rec.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].origin, Some(b"server".to_vec()));
}

#[test]
fn a_transaction_without_changes_has_no_update() {
    let doc = doc(1);
    let rec = Arc::new(UpdateRecorder::default());
    let _sub = doc.observe_update_v2(rec.clone()).unwrap();

    doc.transaction(None).unwrap().commit().unwrap();

    assert_eq!(rec.events.lock().unwrap().len(), 0);
}

#[test]
fn subscribing_to_updates_while_a_transaction_is_open_fails() {
    let doc = doc(1);
    let txn = doc.transaction(None).unwrap();

    assert!(doc.observe_update_v2(Arc::new(UpdateRecorder::default())).is_err());
    assert!(doc.observe_transaction_cleanup(Arc::new(CleanupRecorder::default())).is_err());

    txn.commit().unwrap();
    assert!(doc.observe_update_v2(Arc::new(UpdateRecorder::default())).is_ok());
}

/// Unsubscribes from updates on its first update (the document is locked then: the callback is
/// switched off, and removed later).
struct OneShotUpdate {
    sub: Mutex<Option<Arc<YSubscription>>>,
    calls: Mutex<usize>,
}

impl YUpdateObserver for OneShotUpdate {
    fn on_update(&self, _event: YUpdateEvent) {
        *self.calls.lock().unwrap() += 1;
        if let Some(sub) = self.sub.lock().unwrap().take() {
            assert!(sub.unsubscribe());
            assert!(!sub.is_active());
        }
    }
}

#[test]
fn unsubscribing_from_updates_inside_the_callback_stops_it() {
    let doc = doc(1);
    let text = doc.get_text("t");
    let observer = Arc::new(OneShotUpdate { sub: Mutex::new(None), calls: Mutex::new(0) });
    let sub = doc.observe_update_v2(observer.clone()).unwrap();
    *observer.sub.lock().unwrap() = Some(sub);

    text.insert(0, "a", None, None).unwrap();
    text.insert(1, "b", None, None).unwrap();
    text.insert(2, "c", None, None).unwrap();

    assert_eq!(*observer.calls.lock().unwrap(), 1);
}

#[test]
fn update_subscriptions_end_on_release_and_may_outlive_the_document() {
    let rec = Arc::new(UpdateRecorder::default());
    let doc = doc(1);
    let text = doc.get_text("t");
    {
        let _sub = doc.observe_update_v1(rec.clone()).unwrap();
        text.insert(0, "a", None, None).unwrap();
    }
    text.insert(1, "b", None, None).unwrap();
    assert_eq!(rec.events.lock().unwrap().len(), 1);

    let sub = doc.observe_update_v2(rec.clone()).unwrap();
    drop(text);
    drop(doc);
    assert!(sub.unsubscribe());
}

#[test]
fn cleanup_events_carry_the_state_before_and_after_and_what_was_deleted() {
    let doc = doc(7);
    let text = doc.get_text("t");
    let rec = Arc::new(CleanupRecorder::default());
    let _sub = doc.observe_transaction_cleanup(rec.clone()).unwrap();

    text.insert(0, "hello", None, None).unwrap();
    let txn = Arc::new(doc.transaction(Some("cut".into())).unwrap());
    text.delete(0, 2, Some(txn.clone())).unwrap();
    txn.commit().unwrap();

    // Every committed transaction has one, an empty one too (as Yjs's afterTransaction); the
    // subscription's own registration doesn't.
    doc.transaction(Some("empty".into())).unwrap().commit().unwrap();

    let events = rec.events.lock().unwrap();
    assert_eq!(events.len(), 3);
    assert_eq!(events[2].before_state, events[2].after_state);
    assert_eq!(events[2].origin, Some(b"empty".to_vec()));
    assert_eq!(events[0].before_state.get(&7).copied().unwrap_or(0), 0, "nothing from client 7 before");
    assert_eq!(events[0].after_state.get(&7), Some(&5));
    assert_eq!(events[1].before_state.get(&7), Some(&5));
    assert_eq!(events[1].after_state.get(&7), Some(&5), "a delete adds no clock");
    assert!(events[1].delete_set.len() > 1, "the delete set names the deleted items");
    assert_eq!(events[1].origin, Some(b"cut".to_vec()));
}

// Threads --------------------------------------------------------------------------------------------

/// ykt's contract: one call into a document at a time, from any thread. Here eight threads share a
/// document behind a lock; every commit calls the observer on the thread that commits it.
#[test]
fn a_shared_document_behind_a_lock_calls_observers_on_the_committing_thread() {
    let doc = Arc::new(doc(1));
    let text = Arc::new(doc.get_text("t"));
    let lock = Arc::new(Mutex::new(()));
    let rec = Arc::new(TextRecorder::default());
    let updates = Arc::new(UpdateRecorder::default());
    let _sub = text.observe(rec.clone()).unwrap();
    let _updates_sub = doc.observe_update_v2(updates.clone()).unwrap();
    let threads = 8;
    let per_thread = 50;
    let start = Arc::new(Barrier::new(threads));

    let ids: Vec<ThreadId> = (0..threads)
        .map(|_| {
            let (text, lock, start) = (text.clone(), lock.clone(), start.clone());
            thread::spawn(move || {
                start.wait();
                for _ in 0..per_thread {
                    let _guard = lock.lock().unwrap();
                    text.insert(0, "x", None, None).unwrap();
                }
                thread::current().id()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();

    assert_eq!(rec.count(), threads * per_thread);
    assert_eq!(updates.events.lock().unwrap().len(), threads * per_thread);
    assert_eq!(text.length(None).unwrap() as usize, threads * per_thread);
    let callback_threads: std::collections::HashSet<ThreadId> =
        rec.events.lock().unwrap().iter().map(|e| e.2).collect();
    assert_eq!(callback_threads, ids.into_iter().collect(), "each callback ran on the thread that committed");
}

/// Two replicas, each owned by its own thread, edit at the same time and sync through their update
/// events (a channel each way, as a network would): they converge, and each one's text observer
/// hears the other's edits as remote ones. Updates applied from the other side carry the origin
/// "remote", so they aren't sent back.
#[test]
fn two_replicas_on_two_threads_converge_through_update_events() {
    struct Forward(Mutex<std::sync::mpsc::Sender<Vec<u8>>>);
    impl YUpdateObserver for Forward {
        fn on_update(&self, event: YUpdateEvent) {
            if event.origin.as_deref() != Some(b"remote") {
                self.0.lock().unwrap().send(event.update).unwrap();
            }
        }
    }

    let (to_b, from_a) = channel::<Vec<u8>>();
    let (to_a, from_b) = channel::<Vec<u8>>();
    let edits = 200;
    let both_done = Arc::new(Barrier::new(2));

    let run = |client: u64, letter: &'static str, out: std::sync::mpsc::Sender<Vec<u8>>, inbox: std::sync::mpsc::Receiver<Vec<u8>>, done: Arc<Barrier>| {
        thread::spawn(move || {
            let doc = doc(client);
            let text = doc.get_text("t");
            let heard = Arc::new(TextRecorder::default());
            let _heard_sub = text.observe(heard.clone()).unwrap();
            let _forward = doc.observe_update_v2(Arc::new(Forward(Mutex::new(out)))).unwrap();
            for i in 0..edits {
                text.insert(0, letter, None, None).unwrap();
                // Take in what has come so far, as a live connection would.
                if i % 7 == 0 {
                    while let Ok(update) = inbox.try_recv() {
                        apply_update_v2(&doc, &update, Some(b"remote".to_vec())).unwrap();
                    }
                }
            }
            done.wait(); // both sides have sent everything
            while let Ok(update) = inbox.recv_timeout(std::time::Duration::from_millis(200)) {
                apply_update_v2(&doc, &update, Some(b"remote".to_vec())).unwrap();
            }
            let remote = heard.origins().iter().filter(|o| o.as_deref() == Some(b"remote")).count();
            (text.to_string(None).unwrap(), remote)
        })
    };

    let a = run(1, "a", to_b, from_b, both_done.clone());
    let b = run(2, "b", to_a, from_a, both_done);
    let (text_a, remote_a) = a.join().unwrap();
    let (text_b, remote_b) = b.join().unwrap();

    assert_eq!(text_a, text_b, "the replicas converge");
    assert_eq!(text_a.len(), 2 * edits);
    assert_eq!(text_a.matches('a').count(), edits);
    assert!(remote_a > 0 && remote_b > 0, "each side heard the other's edits as remote");
}
