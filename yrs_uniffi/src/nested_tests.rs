//! Shared types nested in maps, arrays and texts, through ykt's API: made with `setText` / `setMap`
//! / `setArray` / `setDoc`, `insertText` / `insertMap` / `insertArray`, `insertEmbedMap`; read back
//! as handles (`getOutput`) and as JSON (`get`); edits through the handles replicating; observers
//! reporting them; ids; deleted ones; undo over a map and an array.

use crate::array::{YArrayDelta, YArrayEvent, YArrayObserver};
use crate::attrs::{YAttributes, YValue};
use crate::collection::YBranchId;
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::map::{YEntryChange, YMapEvent, YMapObserver};
use crate::output::YOutput;
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2, encode_state_vector};
use crate::tools::Error;
use crate::undo::{YUndoManager, YUndoOptions};
use crate::updates::diff_update_v2;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

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

/// What `from` has that `to` lacks, applied to `to`.
fn sync(from: &YDoc, to: &YDoc) {
    let missing = diff_update_v2(encode_state_as_update_v2(from, None).unwrap(), encode_state_vector(to).unwrap()).unwrap();
    apply_update_v2(to, &missing, None).unwrap();
}

fn s(v: &str) -> YValue {
    YValue::String(v.to_string())
}

fn n(v: f64) -> YValue {
    YValue::Number(v)
}

fn entries(pairs: &[(&str, YValue)]) -> YAttributes {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn text_of(output: Option<YOutput>) -> Arc<crate::text::YText> {
    match output {
        Some(YOutput::Text { text }) => text,
        other => panic!("not a text: {other:?}"),
    }
}

fn map_of(output: Option<YOutput>) -> Arc<crate::map::YMap> {
    match output {
        Some(YOutput::Map { map }) => map,
        other => panic!("not a map: {other:?}"),
    }
}

fn array_of(output: Option<YOutput>) -> Arc<crate::array::YArray> {
    match output {
        Some(YOutput::Array { array }) => array,
        other => panic!("not an array: {other:?}"),
    }
}

#[test]
fn types_set_in_a_map_are_handles_whose_edits_replicate() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let title = root.set_text("title", Some("Hello".into()), None).unwrap();
    let meta = root.set_map("meta", Some(entries(&[("pinned", YValue::Bool(true))])), None).unwrap();
    let tags = root.set_array("tags", Some(vec![s("work")]), None).unwrap();
    title.insert(5, ", world", None, None).unwrap();
    meta.set("color", s("red"), None).unwrap();
    tags.push(vec![s("home")], None).unwrap();

    let b = doc(2);
    sync(&a, &b);
    let replica = b.get_map("root").unwrap();
    let b_title = text_of(replica.get_output("title", None).unwrap());
    let b_meta = map_of(replica.get_output("meta", None).unwrap());
    let b_tags = array_of(replica.get_output("tags", None).unwrap());
    assert_eq!(b_title.to_string(None).unwrap(), "Hello, world");
    assert_eq!(b_meta.to_json(None).unwrap(), entries(&[("pinned", YValue::Bool(true)), ("color", s("red"))]));
    assert_eq!(b_tags.to_json(None).unwrap(), vec![s("work"), s("home")]);

    // And back: edits on the replica's handles reach the first document's.
    b_title.insert(0, "> ", None, None).unwrap();
    b_meta.delete("pinned", None).unwrap();
    sync(&b, &a);
    assert_eq!(title.to_string(None).unwrap(), "> Hello, world");
    assert_eq!(meta.to_json(None).unwrap(), entries(&[("color", s("red"))]));

    // The plain API still gives values: a nested type in its JSON form.
    assert_eq!(replica.get("title", None).unwrap(), Some(s("> Hello, world")));
    assert_eq!(replica.get_output("missing", None).unwrap(), None);
    root.set("plain", n(1.0), None).unwrap();
    assert_eq!(root.get_output("plain", None).unwrap(), Some(YOutput::Value { value: n(1.0) }));
}

#[test]
fn types_inserted_into_an_array_are_handles_too() {
    let a = doc(1);
    let list = a.get_array("list").unwrap();
    list.push(vec![s("plain")], None).unwrap();
    let text = list.insert_text(0, Some("first".into()), None).unwrap();
    let map = list.insert_map(2, None, None).unwrap();
    let inner = list.insert_array(1, Some(vec![n(1.0)]), None).unwrap();
    assert!(matches!(list.insert_map(5, None, None), Err(Error::OutOfBounds)));
    map.set_text("deep", Some("deeper".into()), None).unwrap();
    inner.push(vec![n(2.0)], None).unwrap();
    text.push("!", None, None).unwrap();

    let b = doc(2);
    sync(&a, &b);
    let replica = b.get_array("list").unwrap();
    assert_eq!(replica.len(None).unwrap(), 4);
    assert_eq!(text_of(Some(replica.get_output(0, None).unwrap())).to_string(None).unwrap(), "first!");
    assert_eq!(array_of(Some(replica.get_output(1, None).unwrap())).to_json(None).unwrap(), vec![n(1.0), n(2.0)]);
    assert_eq!(replica.get_output(2, None).unwrap(), YOutput::Value { value: s("plain") });
    let b_map = map_of(Some(replica.get_output(3, None).unwrap()));
    assert_eq!(text_of(b_map.get_output("deep", None).unwrap()).to_string(None).unwrap(), "deeper");
    assert_eq!(replica.get(3, None).unwrap(), YValue::AttrMap(entries(&[("deep", s("deeper"))])));
    assert!(matches!(replica.get_output(4, None), Err(Error::OutOfBounds)));
}

#[test]
fn handles_compare_by_identity_and_ids_match_across_replicas() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let meta = root.set_map("meta", None, None).unwrap();
    let again = map_of(root.get_output("meta", None).unwrap());
    assert_eq!(*meta, *again, "two handles of one map");
    assert_eq!(root.get_output("meta", None).unwrap(), root.get_output("meta", None).unwrap());
    assert!(matches!(meta.id(), YBranchId::Nested { client: 1, .. }));
    assert_eq!(root.id(), YBranchId::Root { name: "root".into() });
    let text = root.set_text("t", None, None).unwrap();
    assert!(matches!(text.id().unwrap(), YBranchId::Nested { client: 1, .. }));

    let b = doc(2);
    sync(&a, &b);
    let b_meta = map_of(b.get_map("root").unwrap().get_output("meta", None).unwrap());
    assert_eq!(b_meta.id(), meta.id(), "the same id on every replica");
    assert_ne!(*b_meta, *meta, "but another document's handle");
}

#[test]
fn a_replaced_nested_type_is_deleted() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let meta = root.set_map("meta", None, None).unwrap();
    let title = root.set_text("title", Some("x".into()), None).unwrap();
    root.set("meta", n(1.0), None).unwrap();
    root.delete("title", None).unwrap();
    assert!(matches!(meta.set("k", n(1.0), None), Err(Error::TypeDeleted)));
    assert!(matches!(title.to_string(None), Err(Error::TypeDeleted)));
}

#[derive(Default)]
struct Heard {
    maps: Mutex<Vec<YMapEvent>>,
    arrays: Mutex<Vec<YArrayEvent>>,
}

impl YMapObserver for Heard {
    fn on_change(&self, event: YMapEvent) {
        self.maps.lock().unwrap().push(event);
    }
}

impl YArrayObserver for Heard {
    fn on_change(&self, event: YArrayEvent) {
        self.arrays.lock().unwrap().push(event);
    }
}

#[test]
fn observers_report_nested_types_as_handles_usable_after_the_callback() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let list = a.get_array("list").unwrap();
    let heard = Arc::new(Heard::default());
    let _map_sub = root.observe(heard.clone()).unwrap();
    let _array_sub = list.observe(heard.clone()).unwrap();

    let meta = root.set_map("meta", Some(entries(&[("k", n(1.0))])), None).unwrap();
    list.insert_text(0, Some("hi".into()), None).unwrap();
    root.set("meta", s("replaced"), None).unwrap();

    let maps = heard.maps.lock().unwrap();
    let inserted = match &maps[0].keys["meta"] {
        YEntryChange::Inserted { value: YOutput::Map { map } } => map.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(*inserted, *meta);
    assert_eq!(
        maps[1].keys["meta"],
        YEntryChange::Updated { old_value: YOutput::Map { map: meta.clone() }, new_value: YOutput::Value { value: s("replaced") } }
    );
    let arrays = heard.arrays.lock().unwrap();
    match &arrays[0].delta[..] {
        [YArrayDelta::Insert(values)] => assert_eq!(text_of(values.first().cloned()).to_string(None).unwrap(), "hi"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_remote_nested_type_reaches_observers_as_a_handle() {
    let a = doc(1);
    a.get_map("root").unwrap().set_array("tags", Some(vec![s("x")]), None).unwrap();
    let b = doc(2);
    let root = b.get_map("root").unwrap();
    let heard = Arc::new(Heard::default());
    let _sub = root.observe(heard.clone()).unwrap();

    sync(&a, &b);

    let maps = heard.maps.lock().unwrap();
    match &maps[0].keys["tags"] {
        YEntryChange::Inserted { value: YOutput::Array { array } } => assert_eq!(array.to_json(None).unwrap(), vec![s("x")]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_map_embedded_in_a_text_is_a_handle_and_json_in_deltas() {
    let a = doc(1);
    let text = a.get_text("t").unwrap();
    text.insert(0, "ab", None, None).unwrap();
    let table = text.insert_embed_map(1, Some(entries(&[("rows", n(2.0))])), None, None).unwrap();
    table.set("cols", n(3.0), None).unwrap();
    assert!(matches!(text.insert_embed_array(9, None, None, None), Err(Error::OutOfBounds)));

    let expected = vec![
        YDelta::YInsert(s("a"), None),
        YDelta::YInsert(YValue::AttrMap(entries(&[("rows", n(2.0)), ("cols", n(3.0))])), None),
        YDelta::YInsert(s("b"), None),
    ];
    assert_eq!(text.to_delta(None, None, None).unwrap(), expected);
    let b = doc(2);
    sync(&a, &b);
    assert_eq!(b.get_text("t").unwrap().to_delta(None, None, None).unwrap(), expected);
}

#[test]
fn subdocuments_set_in_a_map() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let page = YDoc::new(None);
    let guid = page.guid();
    let sub = root.set_doc("page", &page, None).unwrap();
    assert_eq!(sub.guid(), guid);
    assert!(matches!(root.get_output("page", None).unwrap(), Some(YOutput::Doc { doc }) if doc.guid() == guid));
    assert!(matches!(root.set_doc("again", &sub, None), Err(Error::AlreadyIntegrated)));
    assert!(matches!(root.set_doc("self", &a, None), Err(Error::AlreadyIntegrated)));
    assert!(matches!(sub.get_map("m").unwrap().set_doc("nested", &YDoc::new(None), None), Err(Error::WrongDocument)));
}

/// The undo manager with a map in its scope: changes to its keys and to the types nested in it.
#[test]
fn undo_over_a_map_and_its_nested_types() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let undo = YUndoManager::for_map(&a, &root, Some(YUndoOptions { capture_timeout_millis: 0, tracked_origins: vec!["editor".into()] })).unwrap();
    let edit = |f: &dyn Fn(Arc<crate::transaction::YTransaction>)| {
        let txn = Arc::new(a.transaction(Some("editor".into())).unwrap());
        f(txn.clone());
        txn.commit().unwrap();
    };

    edit(&|txn| root.set("title", s("Draft"), Some(txn)).unwrap());
    let body = root.set_text("body", Some("".into()), None).unwrap(); // untracked: not undone
    edit(&|txn| body.insert(0, "hello", None, Some(txn)).unwrap());
    edit(&|txn| root.set("title", s("Final"), Some(txn)).unwrap());
    assert_eq!(root.get("title", None).unwrap(), Some(s("Final")));

    assert!(undo.undo().unwrap());
    assert_eq!(root.get("title", None).unwrap(), Some(s("Draft")));
    assert!(undo.undo().unwrap());
    assert_eq!(body.to_string(None).unwrap(), "", "the nested text's edit is undone");
    assert!(undo.undo().unwrap());
    assert_eq!(root.get("title", None).unwrap(), None);
    assert!(root.contains_key("body", None).unwrap(), "the untracked change stays");
    assert!(undo.redo().unwrap());
    assert!(undo.redo().unwrap());
    assert_eq!(body.to_string(None).unwrap(), "hello");

    // An array joins the scope.
    let list = a.get_array("list").unwrap();
    undo.expand_scope_array(&list).unwrap();
    edit(&|txn| list.push(vec![n(1.0)], Some(txn)).unwrap());
    assert!(undo.undo().unwrap());
    assert_eq!(list.len(None).unwrap(), 0);
    assert!(matches!(undo.expand_scope_map(&doc(2).get_map("m").unwrap()), Err(Error::WrongDocument)));
    let for_array = YUndoManager::for_array(&a, &list, None).unwrap();
    list.push(vec![n(2.0)], None).unwrap();
    assert!(for_array.undo().unwrap());
    assert_eq!(list.len(None).unwrap(), 0);
}

#[test]
fn nested_values_of_a_new_map_and_array_keep_their_kinds() {
    let a = doc(1);
    let root = a.get_map("root").unwrap();
    let every = entries(&[("n", n(1.5)), ("b", YValue::Bool(false)), ("s", s("x")), ("nested", YValue::AttrMap(HashMap::new()))]);
    let map = root.set_map("m", Some(every.clone()), None).unwrap();
    let array = root.set_array("a", Some(every.values().cloned().collect()), None).unwrap();
    assert_eq!(map.to_json(None).unwrap(), every);
    assert_eq!(array.len(None).unwrap(), 4);
    let b = doc(2);
    sync(&a, &b);
    assert_eq!(map_of(b.get_map("root").unwrap().get_output("m", None).unwrap()).to_json(None).unwrap(), every);
}
