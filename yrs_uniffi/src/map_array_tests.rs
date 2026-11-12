//! YMap and YArray through ykt's API (the same calls the Kotlin and Swift bindings make): reads and
//! writes, values of every `YValue` kind (locally and through a remote replica), out-of-range
//! errors, concurrent edits converging, observers (key changes with old and new values, array
//! deltas, origins, remote updates, unsubscribing), transactions, and nested shared types written by
//! another client (read in their JSON form).

use crate::array::{YArrayDelta, YArrayEvent, YArrayObserver};
use crate::attrs::YValue;
use crate::doc::{YDoc, YDocOptions};
use crate::map::{YEntryChange, YMapEvent, YMapObserver};
use crate::output::YOutput;
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2};
use crate::tools::Error;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use yrs::{Array, ArrayPrelim, Map, MapPrelim, Text, TextPrelim, Transact, WriteTxn};

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

fn sync(from: &YDoc, to: &YDoc, origin: Option<&str>) {
    let update = encode_state_as_update_v2(from, None).unwrap();
    apply_update_v2(to, &update, origin.map(|o| o.as_bytes().to_vec())).unwrap();
}

fn s(v: &str) -> YValue {
    YValue::String(v.to_string())
}

fn n(v: f64) -> YValue {
    YValue::Number(v)
}

fn out(value: YValue) -> YOutput {
    YOutput::Value { value }
}

fn outs(values: Vec<YValue>) -> Vec<YOutput> {
    values.into_iter().map(out).collect()
}

/// One value of every kind, nested ones too, as they read back.
fn every_kind() -> Vec<(&'static str, YValue)> {
    vec![
        ("null", YValue::Null),
        ("bool", YValue::Bool(true)),
        ("int", n(42.0)),
        ("float", n(-1.5)),
        ("bigint", YValue::BigInt((1 << 60) + 1)),
        ("string", s("héllo 😀")),
        ("empty string", s("")),
        ("buffer", YValue::Buffer(vec![0, 1, 255])),
        ("array", YValue::Array(vec![n(1.0), s("two"), YValue::Null, YValue::Array(vec![YValue::Bool(false)])])),
        ("map", YValue::AttrMap(HashMap::from([("a".to_string(), n(1.0)), ("b".to_string(), YValue::AttrMap(HashMap::new()))]))),
    ]
}

struct MapEvents(Mutex<Vec<YMapEvent>>);
impl YMapObserver for MapEvents {
    fn on_change(&self, event: YMapEvent) {
        self.0.lock().unwrap().push(event);
    }
}

struct ArrayEvents(Mutex<Vec<YArrayEvent>>);
impl YArrayObserver for ArrayEvents {
    fn on_change(&self, event: YArrayEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn map_events() -> Arc<MapEvents> {
    Arc::new(MapEvents(Mutex::new(vec![])))
}

fn array_events() -> Arc<ArrayEvents> {
    Arc::new(ArrayEvents(Mutex::new(vec![])))
}

// YMap ---------------------------------------------------------------------------------------------

#[test]
fn a_map_sets_gets_and_deletes() {
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    assert_eq!(map.len(None).unwrap(), 0);
    assert_eq!(map.get("a", None).unwrap(), None);

    map.set("a", n(1.0), None).unwrap();
    map.set("b", s("two"), None).unwrap();
    map.set("a", n(3.0), None).unwrap(); // replaces

    assert_eq!(map.get("a", None).unwrap(), Some(n(3.0)));
    assert!(map.contains_key("b", None).unwrap());
    assert!(!map.contains_key("c", None).unwrap());
    assert_eq!(map.len(None).unwrap(), 2);
    let keys = map.keys(None).unwrap();
    let values = map.values(None).unwrap();
    let pairs: HashMap<String, YValue> = keys.into_iter().zip(values).collect();
    assert_eq!(pairs, HashMap::from([("a".into(), n(3.0)), ("b".into(), s("two"))]), "keys and values in the same order");
    assert_eq!(map.to_json(None).unwrap(), pairs);

    assert_eq!(map.delete("a", None).unwrap(), Some(n(3.0)));
    assert_eq!(map.delete("a", None).unwrap(), None, "already gone");
    assert_eq!(map.get("a", None).unwrap(), None);
    map.clear(None).unwrap();
    assert_eq!(map.len(None).unwrap(), 0);
    assert!(map.to_json(None).unwrap().is_empty());
}

#[test]
fn a_map_holds_values_of_every_kind_and_a_replica_reads_them_the_same() {
    let a = doc(1);
    let map = a.get_map("m").unwrap();
    for (key, value) in every_kind() {
        map.set(key, value, None).unwrap();
    }
    map.set("small bigint", YValue::BigInt(7), None).unwrap();
    map.set("round bigint", YValue::BigInt(1 << 60), None).unwrap();
    let b = doc(2);
    sync(&a, &b, None);

    for replica in [&a, &b] {
        let map = replica.get_map("m").unwrap();
        for (key, value) in every_kind() {
            assert_eq!(map.get(key, None).unwrap(), Some(value), "{key}");
        }
        assert_eq!(map.get("small bigint", None).unwrap(), Some(n(7.0)), "an integer in JS's safe range is a number");
    }
    // lib0's encoding (Yjs's too) sends an integer a double holds exactly as a double: a replica
    // reads it as a number.
    assert_eq!(a.get_map("m").unwrap().get("round bigint", None).unwrap(), Some(YValue::BigInt(1 << 60)));
    assert_eq!(b.get_map("m").unwrap().get("round bigint", None).unwrap(), Some(n((1u64 << 60) as f64)));
}

#[test]
fn concurrent_sets_of_a_key_converge() {
    let a = doc(1);
    let b = doc(2);
    a.get_map("m").unwrap().set("k", s("from a"), None).unwrap();
    b.get_map("m").unwrap().set("k", s("from b"), None).unwrap();
    b.get_map("m").unwrap().set("only b", YValue::Bool(true), None).unwrap();

    sync(&a, &b, None);
    sync(&b, &a, None);

    let in_a = a.get_map("m").unwrap().to_json(None).unwrap();
    assert_eq!(in_a, b.get_map("m").unwrap().to_json(None).unwrap());
    assert_eq!(in_a.len(), 2);
}

#[test]
fn a_map_observer_hears_each_keys_change_with_old_and_new_values() {
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    map.set("kept", n(1.0), None).unwrap();
    map.set("gone", n(2.0), None).unwrap();
    let events = map_events();
    let _sub = map.observe(events.clone()).unwrap();

    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    map.set("new", s("x"), Some(txn.clone())).unwrap();
    map.set("new", s("y"), Some(txn.clone())).unwrap(); // a new key set twice: inserted, with the last value
    map.set("kept", n(10.0), Some(txn.clone())).unwrap();
    map.delete("gone", Some(txn.clone())).unwrap();
    assert!(events.0.lock().unwrap().is_empty(), "nothing before the commit");
    txn.commit().unwrap();
    map.clear(None).unwrap();

    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].keys,
        HashMap::from([
            ("new".into(), YEntryChange::Inserted { value: out(s("y")) }),
            ("kept".into(), YEntryChange::Updated { old_value: out(n(1.0)), new_value: out(n(10.0)) }),
            ("gone".into(), YEntryChange::Removed { old_value: out(n(2.0)) }),
        ])
    );
    assert_eq!(events[0].origin, Some(b"editor".to_vec()));
    assert_eq!(
        events[1].keys,
        HashMap::from([
            ("new".into(), YEntryChange::Removed { old_value: out(s("y")) }),
            ("kept".into(), YEntryChange::Removed { old_value: out(n(10.0)) }),
        ])
    );
    assert_eq!(events[1].origin, None);
}

#[test]
fn a_map_observer_hears_remote_updates_once_and_stops_when_released() {
    let a = doc(1);
    a.get_map("m").unwrap().set("k", n(1.0), None).unwrap();
    let b = doc(2);
    let map = b.get_map("m").unwrap();
    let events = map_events();
    let sub = map.observe(events.clone()).unwrap();

    sync(&a, &b, Some("server"));
    sync(&a, &b, Some("server")); // nothing new
    assert!(sub.unsubscribe());
    map.set("after", n(2.0), None).unwrap();

    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].keys, HashMap::from([("k".into(), YEntryChange::Inserted { value: out(n(1.0)) })]));
    assert_eq!(events[0].origin, Some(b"server".to_vec()));
}

#[test]
fn other_types_changes_dont_reach_a_map_observer() {
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    let events = map_events();
    let _sub = map.observe(events.clone()).unwrap();

    doc.get_map("other").unwrap().set("k", n(1.0), None).unwrap();
    doc.get_text("t").unwrap().insert(0, "x", None, None).unwrap();

    assert!(events.0.lock().unwrap().is_empty());
}

#[test]
fn a_nested_shared_type_reads_as_its_json() {
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    {
        // What a Yjs client could write: shared types inside the map.
        let mut txn = doc.0.transact_mut();
        let raw = txn.get_or_insert_map("m");
        raw.insert(&mut txn, "map", MapPrelim::from([("a", yrs::In::from(1))]));
        raw.insert(&mut txn, "text", TextPrelim::new("hi"));
        raw.insert(&mut txn, "array", ArrayPrelim::from([1, 2]));
    }

    assert_eq!(map.get("map", None).unwrap(), Some(YValue::AttrMap(HashMap::from([("a".into(), n(1.0))]))));
    assert_eq!(map.get("text", None).unwrap(), Some(s("hi")));
    assert_eq!(map.get("array", None).unwrap(), Some(YValue::Array(vec![n(1.0), n(2.0)])));
    assert_eq!(map.to_json(None).unwrap().get("text"), Some(&s("hi")));
}

// YArray -------------------------------------------------------------------------------------------

#[test]
fn an_array_inserts_pushes_gets_and_deletes() {
    let doc = doc(1);
    let array = doc.get_array("a").unwrap();
    assert_eq!(array.len(None).unwrap(), 0);

    array.push(vec![n(1.0), n(2.0)], None).unwrap();
    array.insert(0, vec![s("first")], None).unwrap();
    array.insert(3, vec![s("last")], None).unwrap(); // at the end
    array.insert(2, vec![], None).unwrap(); // nothing
    assert_eq!(array.to_json(None).unwrap(), vec![s("first"), n(1.0), n(2.0), s("last")]);
    assert_eq!(array.get(0, None).unwrap(), s("first"));
    assert_eq!(array.get(3, None).unwrap(), s("last"));

    array.delete(1, 2, None).unwrap();
    array.delete(0, 0, None).unwrap(); // nothing
    assert_eq!(array.to_json(None).unwrap(), vec![s("first"), s("last")]);
    assert_eq!(array.len(None).unwrap(), 2);
}

#[test]
fn out_of_range_is_an_error_and_changes_nothing() {
    let doc = doc(1);
    let array = doc.get_array("a").unwrap();
    array.push(vec![n(1.0), n(2.0)], None).unwrap();

    assert!(matches!(array.insert(3, vec![n(9.0)], None), Err(Error::OutOfBounds)));
    assert!(matches!(array.delete(1, 2, None), Err(Error::OutOfBounds)));
    assert!(matches!(array.delete(3, 0, None), Err(Error::OutOfBounds)));
    assert!(matches!(array.delete(1, u32::MAX, None), Err(Error::OutOfBounds)), "no overflow");
    assert!(matches!(array.get(2, None), Err(Error::OutOfBounds)));
    assert_eq!(array.to_json(None).unwrap(), vec![n(1.0), n(2.0)]);
}

#[test]
fn an_array_holds_values_of_every_kind_and_a_replica_reads_them_the_same() {
    let a = doc(1);
    let array = a.get_array("a").unwrap();
    let values: Vec<YValue> = every_kind().into_iter().map(|(_, v)| v).collect();
    array.push(values.clone(), None).unwrap();
    let b = doc(2);
    sync(&a, &b, None);

    assert_eq!(array.to_json(None).unwrap(), values);
    assert_eq!(b.get_array("a").unwrap().to_json(None).unwrap(), values);
    for (i, value) in values.iter().enumerate() {
        assert_eq!(&b.get_array("a").unwrap().get(i as u32, None).unwrap(), value);
    }
}

#[test]
fn concurrent_inserts_into_an_array_converge() {
    let a = doc(1);
    let b = doc(2);
    a.get_array("a").unwrap().push(vec![s("base")], None).unwrap();
    sync(&a, &b, None);
    a.get_array("a").unwrap().insert(0, vec![s("a1"), s("a2")], None).unwrap();
    b.get_array("a").unwrap().insert(0, vec![s("b1")], None).unwrap();
    b.get_array("a").unwrap().delete(1, 1, None).unwrap(); // "base"

    sync(&a, &b, None);
    sync(&b, &a, None);

    let in_a = a.get_array("a").unwrap().to_json(None).unwrap();
    assert_eq!(in_a, b.get_array("a").unwrap().to_json(None).unwrap());
    assert_eq!(in_a.len(), 3);
    assert!(!in_a.contains(&s("base")));
}

#[test]
fn an_array_observer_hears_deltas_with_their_origin() {
    let doc = doc(1);
    let array = doc.get_array("a").unwrap();
    let events = array_events();
    let _sub = array.observe(events.clone()).unwrap();

    array.push(vec![n(1.0), n(2.0), n(3.0)], None).unwrap();
    array.insert(1, vec![s("x")], None).unwrap();
    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    array.delete(0, 1, Some(txn.clone())).unwrap();
    array.push(vec![YValue::Bool(true)], Some(txn.clone())).unwrap();
    txn.commit().unwrap();

    let events = events.0.lock().unwrap();
    let deltas: Vec<Vec<YArrayDelta>> = events.iter().map(|e| e.delta.clone()).collect();
    assert_eq!(
        deltas,
        vec![
            vec![YArrayDelta::Insert(outs(vec![n(1.0), n(2.0), n(3.0)]))],
            vec![YArrayDelta::Retain(1), YArrayDelta::Insert(outs(vec![s("x")]))],
            vec![YArrayDelta::Delete(1), YArrayDelta::Retain(3), YArrayDelta::Insert(outs(vec![YValue::Bool(true)]))],
        ]
    );
    assert_eq!(events.iter().map(|e| e.origin.clone()).collect::<Vec<_>>(), vec![None, None, Some(b"editor".to_vec())]);
}

#[test]
fn an_array_observer_hears_remote_updates_and_stops_when_released() {
    let a = doc(1);
    a.get_array("a").unwrap().push(vec![s("x")], None).unwrap();
    let b = doc(2);
    let array = b.get_array("a").unwrap();
    let events = array_events();
    {
        let _sub = array.observe(events.clone()).unwrap();
        sync(&a, &b, Some("server"));
    }
    array.push(vec![s("y")], None).unwrap();

    let events = events.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].delta, vec![YArrayDelta::Insert(outs(vec![s("x")]))]);
    assert_eq!(events[0].origin, Some(b"server".to_vec()));
}

#[test]
fn a_nested_shared_type_in_an_array_reads_as_its_json() {
    let doc = doc(1);
    let array = doc.get_array("a").unwrap();
    {
        let mut txn = doc.0.transact_mut();
        let raw = txn.get_or_insert_array("a");
        let text = raw.insert(&mut txn, 0, TextPrelim::new("hi"));
        text.push(&mut txn, "!");
    }
    assert_eq!(array.get(0, None).unwrap(), s("hi!"));
    assert_eq!(array.to_json(None).unwrap(), vec![s("hi!")]);
}

// Transactions and callbacks ---------------------------------------------------------------------------

#[test]
fn reads_and_writes_in_a_transaction() {
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    let array = doc.get_array("a").unwrap();
    let txn = Arc::new(doc.transaction(None).unwrap());
    map.set("k", n(1.0), Some(txn.clone())).unwrap();
    array.push(vec![n(1.0)], Some(txn.clone())).unwrap();

    // The transaction sees its own changes; without it, the document is locked.
    assert_eq!(map.get("k", Some(txn.clone())).unwrap(), Some(n(1.0)));
    assert_eq!(array.len(Some(txn.clone())).unwrap(), 1);
    assert!(matches!(map.get("k", None), Err(Error::Locked)));
    assert!(matches!(array.push(vec![], None), Err(Error::Locked)));
    assert!(matches!(doc.get_map("m"), Err(Error::Locked)), "no waiting for the lock");
    assert!(matches!(doc.get_array("a"), Err(Error::Locked)));

    txn.commit().unwrap();
    assert!(matches!(map.get("k", Some(txn.clone())), Err(Error::TransactionCommitted)));
    assert!(matches!(map.set("k", n(2.0), Some(txn.clone())), Err(Error::TransactionCommitted)));
    assert!(matches!(array.len(Some(txn)), Err(Error::TransactionCommitted)));
    assert_eq!(map.get("k", None).unwrap(), Some(n(1.0)));
}

#[test]
fn reading_or_writing_inside_a_callback_fails() {
    struct Misbehaving {
        map: crate::map::YMap,
        array: crate::array::YArray,
        results: Mutex<Vec<bool>>,
    }
    impl YMapObserver for Misbehaving {
        fn on_change(&self, _: YMapEvent) {
            let mut r = self.results.lock().unwrap();
            r.push(self.map.get("k", None).is_err());
            r.push(self.map.set("x", n(1.0), None).is_err());
            r.push(self.array.len(None).is_err());
        }
    }
    let doc = doc(1);
    let map = doc.get_map("m").unwrap();
    let observer = Arc::new(Misbehaving {
        map: doc.get_map("m").unwrap(),
        array: doc.get_array("a").unwrap(),
        results: Mutex::new(vec![]),
    });
    let _sub = map.observe(observer.clone()).unwrap();

    map.set("k", n(1.0), None).unwrap();

    assert_eq!(*observer.results.lock().unwrap(), vec![true, true, true]);
    assert_eq!(map.keys(None).unwrap(), vec!["k".to_string()]);
}

