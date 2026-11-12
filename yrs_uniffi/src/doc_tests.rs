//! Root types by name (`getText`, `getXmlFragment`, `getMap`, `getArray`): the same name gives the
//! same type, and while the document is locked — an open transaction, an observer's callback — they
//! throw instead of waiting for a lock that can't come. Every case runs under a timeout, so a
//! regression to yrs's waiting `get_or_insert_*` fails instead of hanging the suite.

use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YTextEvent, YTextObserver, YUpdateEvent, YUpdateObserver};
use crate::tools::Error;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

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

/// Runs `f` on a thread of its own and waits at most 10 s for it: a call that waits for the
/// document's lock fails the test instead of hanging it.
fn within<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = channel();
    thread::spawn(move || {
        let _ = tx.send(f());
    });
    rx.recv_timeout(Duration::from_secs(10)).expect("timed out: a call waited for the document's lock")
}

/// What each root-type getter gave, by name: `true` if it threw `AnotherTx`.
fn every_getter_throws(doc: &YDoc) -> Vec<(&'static str, bool)> {
    let locked = |r: Result<(), Error>| matches!(r, Err(Error::Locked));
    vec![
        ("getText", locked(doc.get_text("t").map(|_| ()))),
        ("getXmlFragment", locked(doc.get_xml_fragment("x").map(|_| ()))),
        ("getMap", locked(doc.get_map("m").map(|_| ()))),
        ("getArray", locked(doc.get_array("a").map(|_| ()))),
    ]
}

fn all_throw() -> Vec<(&'static str, bool)> {
    vec![("getText", true), ("getXmlFragment", true), ("getMap", true), ("getArray", true)]
}

#[test]
fn the_same_name_gives_the_same_text() {
    within(|| {
        let doc = doc(1);
        let first = doc.get_text("t").unwrap();
        let second = doc.get_text("t").unwrap();
        first.insert(0, "hello", None, None).unwrap();
        assert_eq!(second.to_string(None).unwrap(), "hello");
        let fragment = doc.get_xml_fragment("x").unwrap();
        assert_eq!(fragment.length(None).unwrap(), 0);
        assert_eq!(doc.get_xml_fragment("x").unwrap().length(None).unwrap(), 0);
    });
}

#[test]
fn getters_throw_inside_a_transaction_and_work_after_it() {
    within(|| {
        let doc = doc(1);
        let txn = doc.transaction(None).unwrap();
        assert_eq!(every_getter_throws(&doc), all_throw());
        txn.commit().unwrap();
        assert_eq!(every_getter_throws(&doc).iter().filter(|(_, threw)| *threw).count(), 0);
    });
}

/// Calls every getter of `doc` in its callback, and keeps what they gave.
struct Getters {
    doc: YDoc,
    results: Mutex<Vec<Vec<(&'static str, bool)>>>,
}

impl YTextObserver for Getters {
    fn on_change(&self, _event: YTextEvent) {
        self.results.lock().unwrap().push(every_getter_throws(&self.doc));
    }
}

impl YUpdateObserver for Getters {
    fn on_update(&self, _event: YUpdateEvent) {
        self.results.lock().unwrap().push(every_getter_throws(&self.doc));
    }
}

#[test]
fn getters_throw_inside_a_text_observer() {
    within(|| {
        let doc = doc(1);
        let text = doc.get_text("t").unwrap();
        let getters = Arc::new(Getters { doc: YDoc(doc.0.clone()), results: Mutex::new(vec![]) });
        let _sub = text.observe(getters.clone()).unwrap();

        text.insert(0, "a", None, None).unwrap();

        assert_eq!(*getters.results.lock().unwrap(), vec![all_throw()]);
        assert_eq!(text.to_string(None).unwrap(), "a");
    });
}

#[test]
fn getters_throw_inside_an_update_observer_of_a_remote_update() {
    within(|| {
        let a = doc(1);
        a.get_text("t").unwrap().insert(0, "hi", None, None).unwrap();
        let b = doc(2);
        let getters = Arc::new(Getters { doc: YDoc(b.0.clone()), results: Mutex::new(vec![]) });
        let _sub = b.observe_update_v2(getters.clone()).unwrap();

        let update = crate::snapshots::encode_state_as_update_v2(&a, None).unwrap();
        crate::snapshots::apply_update_v2(&b, &update, None).unwrap();

        assert_eq!(*getters.results.lock().unwrap(), vec![all_throw()]);
        assert_eq!(b.get_text("t").unwrap().to_string(None).unwrap(), "hi");
    });
}

#[derive(Default)]
struct Commits(Mutex<usize>);

impl crate::observers::YTransactionCleanupObserver for Commits {
    fn on_cleanup(&self, _event: crate::observers::YTransactionCleanupEvent) {
        *self.0.lock().unwrap() += 1;
    }
}

#[test]
fn getting_an_existing_root_commits_nothing_and_a_remote_one_becomes_typed() {
    within(|| {
        let doc = doc(1);
        doc.get_text("t").unwrap();
        let commits = Arc::new(Commits::default());
        let _sub = doc.observe_transaction_cleanup(commits.clone()).unwrap();
        doc.get_text("t").unwrap();
        doc.get_text("t").unwrap();
        assert_eq!(*commits.0.lock().unwrap(), 0, "no empty transactions for observers");
        doc.get_map("m").unwrap();
        doc.get_map("m").unwrap();
        assert_eq!(*commits.0.lock().unwrap(), 1, "creating the map is one");

        // A root that came in an update has no type until it is got: its observers then work.
        let a = self::doc(2);
        a.get_text("t").unwrap().insert(0, "hi", None, None).unwrap();
        let b = self::doc(3);
        let update = crate::snapshots::encode_state_as_update_v2(&a, None).unwrap();
        crate::snapshots::apply_update_v2(&b, &update, None).unwrap();
        assert_eq!(b.get_text("t").unwrap().to_string(None).unwrap(), "hi");
    });
}
