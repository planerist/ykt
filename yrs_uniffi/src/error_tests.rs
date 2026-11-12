//! ykt's errors, one case at a time, through the API the bindings call: each `YError` where it
//! happens, and nothing changed when one is thrown. Also the paths that used to panic in yrs or in
//! ykt's cells (an index past the end, a delta deleting past the end, malformed v2 bytes, a handle
//! used inside its own callback, a transaction used after its commit) — they throw now.

use crate::attrs::YValue;
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YTextEvent, YTextObserver};
use crate::snapshots::*;
use crate::sticky::YAssoc;
use crate::text::YText;
use crate::tools::Error;
use crate::transaction::YTransaction;
use crate::undo::YUndoManager;
use crate::xml::YXmlChild;
use crate::xml_elem::YXmlElement;
use crate::xml_text::YXmlText;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn doc(client_id: u64, gc: bool) -> YDoc {
    YDoc::new(Some(YDocOptions {
        client_id: Some(client_id),
        guid: None,
        collection_id: None,
        gc: Some(gc),
        auto_load: None,
        should_load: None,
        bytes_offset: None,
    }))
}

fn hello() -> (YDoc, YText) {
    let doc = doc(1, true);
    let text = doc.get_text("t").unwrap();
    text.insert(0, "hello", None, None).unwrap();
    (doc, text)
}

macro_rules! assert_err {
    ($result:expr, $pattern:pat) => {
        match $result {
            Err($pattern) => {}
            Err(other) => panic!("expected {}, got {:?}", stringify!($pattern), other),
            Ok(_) => panic!("expected {}, got Ok", stringify!($pattern)),
        }
    };
}

#[test]
fn locked_while_a_transaction_is_open() {
    let (doc, text) = hello();
    let txn = doc.transaction(None).unwrap();
    assert_err!(text.to_string(None), Error::Locked);
    assert_err!(text.insert(0, "x", None, None), Error::Locked);
    assert_err!(doc.transaction(None), Error::Locked);
    assert_err!(encode_state_vector(&doc), Error::Locked);
    assert_err!(snapshot(&doc), Error::Locked);
    assert_err!(apply_update_v2(&doc, &[0, 0], None), Error::Locked);
    txn.commit().unwrap();
    assert_eq!(text.to_string(None).unwrap(), "hello");
}

/// Uses the very handle it observes (not another one of the same text) inside its callback.
struct SameHandle {
    text: Mutex<Option<Arc<YText>>>,
    results: Mutex<Vec<String>>,
}

impl YTextObserver for SameHandle {
    fn on_change(&self, _event: YTextEvent) {
        let text = self.text.lock().unwrap().clone().unwrap();
        let mut results = self.results.lock().unwrap();
        results.push(format!("{:?}", text.to_string(None).err()));
        results.push(format!("{:?}", text.insert(0, "x", None, None).err()));
        results.push(format!("{:?}", text.length(None).err()));
    }
}

#[test]
fn the_same_handle_inside_its_own_callback_is_locked_not_a_panic() {
    let (_doc, text) = hello();
    let text = Arc::new(text);
    let observer = Arc::new(SameHandle { text: Mutex::new(Some(text.clone())), results: Mutex::new(vec![]) });
    let _sub = text.observe(observer.clone()).unwrap();

    text.insert(5, "!", None, None).unwrap();

    assert_eq!(*observer.results.lock().unwrap(), vec!["Some(Locked)"; 3]);
    assert_eq!(text.to_string(None).unwrap(), "hello!");
}

/// Uses the committing transaction itself inside a callback.
struct SameTransaction {
    txn: Mutex<Option<Arc<YTransaction>>>,
    text: YText,
    results: Mutex<Vec<String>>,
}

impl YTextObserver for SameTransaction {
    fn on_change(&self, _event: YTextEvent) {
        let txn = self.txn.lock().unwrap().clone().unwrap();
        let mut results = self.results.lock().unwrap();
        results.push(format!("{:?}", txn.origin().err()));
        results.push(format!("{:?}", self.text.insert(0, "x", None, Some(txn.clone())).err()));
        results.push(format!("{:?}", txn.commit().err()));
    }
}

#[test]
fn the_committing_transaction_inside_its_callbacks_is_locked() {
    let (doc, text) = hello();
    let observer = Arc::new(SameTransaction { txn: Mutex::new(None), text: doc.get_text("t").unwrap(), results: Mutex::new(vec![]) });
    let _sub = text.observe(observer.clone()).unwrap();
    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    *observer.txn.lock().unwrap() = Some(txn.clone());
    text.insert(0, "> ", None, Some(txn.clone())).unwrap();

    txn.commit().unwrap();

    assert_eq!(*observer.results.lock().unwrap(), vec!["Some(Locked)"; 3]);
    assert_eq!(text.to_string(None).unwrap(), "> hello");
}

#[test]
fn a_committed_transaction_throws_everywhere() {
    let (doc, text) = hello();
    let txn = Arc::new(doc.transaction(None).unwrap());
    txn.commit().unwrap();

    assert_err!(txn.commit(), Error::TransactionCommitted);
    assert_err!(txn.before_state(), Error::TransactionCommitted);
    assert_err!(txn.after_state(), Error::TransactionCommitted);
    assert_err!(txn.origin(), Error::TransactionCommitted);
    assert_err!(txn.state_vector_v1(), Error::TransactionCommitted);
    assert_err!(txn.state_vector_v2(), Error::TransactionCommitted);
    assert_err!(txn.diff_v1(vec![0]), Error::TransactionCommitted);
    assert_err!(txn.encode_update(), Error::TransactionCommitted);
    assert_err!(txn.encode_update_v2(), Error::TransactionCommitted);
    assert_err!(txn.apply_v2(encode_state_as_update_v2(&doc, None).unwrap()), Error::TransactionCommitted);
    assert_err!(txn.gc(), Error::TransactionCommitted);
    assert_err!(text.alive(&txn), Error::TransactionCommitted);
    assert_err!(text.insert(0, "x", None, Some(txn.clone())), Error::TransactionCommitted);
    assert_err!(text.to_delta(None, None, Some(txn)), Error::TransactionCommitted);
}

#[test]
fn another_documents_transaction_or_text_is_the_wrong_document() {
    let (a, text) = hello();
    // Root types are found by name: without the check, b's transaction would write into b's "t".
    let b = doc(2, true);
    let b_text = b.get_text("t").unwrap();
    let txn = Arc::new(b.transaction(None).unwrap());
    assert_err!(text.insert(0, "x", None, Some(txn.clone())), Error::WrongDocument);
    assert_err!(text.to_string(Some(txn.clone())), Error::WrongDocument);
    assert_err!(text.alive(&txn), Error::WrongDocument);
    assert_err!(text.sticky_index(0, YAssoc::After, None).unwrap().offset(&a, Some(txn.clone())), Error::WrongDocument);
    assert_err!(a.get_map("m").unwrap().set("k", YValue::Null, Some(txn.clone())), Error::WrongDocument);
    txn.commit().unwrap();
    assert_err!(YUndoManager::new(&a, &b_text, None), Error::WrongDocument);
    assert_eq!(b_text.to_string(None).unwrap(), "");
    assert_eq!(text.to_string(None).unwrap(), "hello");
}

#[test]
fn a_deleted_node_is_type_deleted() {
    let doc = doc(1, true);
    let fragment = doc.get_xml_fragment("x").unwrap();
    fragment.push(YXmlChild::Element(Arc::new(YXmlElement::new("p".into(), None, None).unwrap())), None).unwrap();
    let element = match fragment.first_child(None).unwrap().unwrap() {
        YXmlChild::Element(e) => e,
        _ => unreachable!(),
    };
    assert_eq!(element.length(None).unwrap(), 0);

    fragment.delete(0, None, None).unwrap();

    assert_err!(element.length(None), Error::TypeDeleted);
    assert_err!(element.set_attribute("a", YValue::Null, None), Error::TypeDeleted);
    let txn = doc.transaction(None).unwrap();
    assert!(!element.alive(&txn).unwrap());
}

#[test]
fn a_preliminary_text_is_not_integrated() {
    let text = YText::new_with_text(Some("abc".into()));
    assert_err!(text.format(0, 1, r#"{"bold":true}"#.into(), None), Error::NotIntegrated);
    assert_err!(text.insert(0, "x", Some(r#"{"bold":true}"#.into()), None), Error::NotIntegrated);
    assert_err!(text.to_delta(None, None, None), Error::NotIntegrated);
    assert_err!(text.apply_delta(vec![], None), Error::NotIntegrated);
    assert_err!(text.sticky_index(0, YAssoc::After, None), Error::NotIntegrated);
    assert_err!(YUndoManager::new(&doc(1, true), &text, None), Error::NotIntegrated);
    let xml = YXmlText::new("abc".into(), None);
    assert_err!(xml.format(0, 1, HashMap::new(), None), Error::NotIntegrated);
    assert_err!(xml.next_sibling(None), Error::NotIntegrated);
    assert_eq!(text.to_string(None).unwrap(), "abc");
}

#[test]
fn an_integrated_node_cannot_be_inserted_again() {
    let doc = doc(1, true);
    let fragment = doc.get_xml_fragment("x").unwrap();
    let element = Arc::new(YXmlElement::new("p".into(), None, None).unwrap());
    fragment.push(YXmlChild::Element(element.clone()), None).unwrap();

    assert_err!(fragment.push(YXmlChild::Element(element.clone()), None), Error::AlreadyIntegrated);
    assert_err!(YXmlElement::new("div".into(), None, Some(vec![YXmlChild::Element(element.clone())])), Error::AlreadyIntegrated);
    // One already in a document deep inside a new one (it was made before its child was inserted).
    let child = Arc::new(YXmlElement::new("b".into(), None, None).unwrap());
    let parent = Arc::new(YXmlElement::new("div".into(), None, Some(vec![YXmlChild::Element(child.clone())])).unwrap());
    fragment.push(YXmlChild::Element(child), None).unwrap();
    assert_err!(fragment.push(YXmlChild::Element(parent), None), Error::AlreadyIntegrated);
    assert_eq!(fragment.length(None).unwrap(), 2);
}

#[test]
fn indexes_past_the_end_are_out_of_bounds_and_change_nothing() {
    let (_doc, text) = hello();
    assert_err!(text.insert(6, "x", None, None), Error::OutOfBounds);
    assert_err!(text.insert(6, "x", Some("{}".into()), None), Error::OutOfBounds);
    assert_err!(text.delete(3, 3, None), Error::OutOfBounds);
    assert_err!(text.delete(1, u32::MAX, None), Error::OutOfBounds);
    assert_err!(text.format(4, 2, r#"{"bold":true}"#.into(), None), Error::OutOfBounds);
    // A delta deleting past the end: yrs would delete what there is, then panic.
    assert_err!(
        text.apply_delta(vec![YDelta::YRetain(2, None), YDelta::YInsert(YValue::String("ab".into()), None), YDelta::YDelete(6)], None),
        Error::OutOfBounds
    );
    assert_eq!(text.to_string(None).unwrap(), "hello");
    // A retain past the end stops there.
    text.apply_delta(vec![YDelta::YRetain(9, None), YDelta::YInsert(YValue::String("!".into()), None)], None).unwrap();
    assert_eq!(text.to_string(None).unwrap(), "hello!");

    // A preliminary text counts UTF-16 units, as an integrated one.
    let prelim = YText::new_with_text(Some("😀a".into()));
    assert_eq!(prelim.length(None).unwrap(), 3);
    assert_err!(prelim.insert(1, "x", None, None), Error::OutOfBounds); // inside the emoji
    assert_err!(prelim.delete(2, 2, None), Error::OutOfBounds);
    prelim.insert(2, "b", None, None).unwrap();
    prelim.delete(0, 2, None).unwrap();
    assert_eq!(prelim.to_string(None).unwrap(), "ba");

    let doc = doc(1, true);
    let fragment = doc.get_xml_fragment("x").unwrap();
    let p = || YXmlChild::Element(Arc::new(YXmlElement::new("p".into(), None, None).unwrap()));
    assert_err!(fragment.insert(1, p(), None), Error::OutOfBounds);
    assert_err!(fragment.delete(0, None, None), Error::OutOfBounds);
    fragment.push(p(), None).unwrap();
    assert_err!(fragment.delete(0, Some(2), None), Error::OutOfBounds);
    let prelim_element = YXmlElement::new("div".into(), None, None).unwrap();
    assert_err!(prelim_element.insert(1, p(), None), Error::OutOfBounds);
    assert_err!(prelim_element.delete(0, None, None), Error::OutOfBounds);
    assert_eq!(fragment.length(None).unwrap(), 1);
}

#[test]
fn attributes_that_are_not_a_json_object_are_invalid() {
    let (_doc, text) = hello();
    for bad in ["[1, 2]", "true", "not json", "{\"bold\":"] {
        assert_err!(text.insert(0, "x", Some(bad.into()), None), Error::InvalidAttributes(_));
        assert_err!(text.format(0, 1, bad.into(), None), Error::InvalidAttributes(_));
    }
    assert_eq!(text.to_string(None).unwrap(), "hello");
}

#[test]
fn malformed_bytes_are_invalid_data_never_a_panic() {
    let (doc, _text) = hello();
    let v2 = encode_state_as_update_v2(&doc, None).unwrap();
    let mut garbage: Vec<Vec<u8>> = vec![vec![], vec![0xff; 9], vec![1, 2, 3]];
    for cut in 1..v2.len() {
        garbage.push(v2[..cut].to_vec());
    }
    for bytes in &garbage {
        let target = self::doc(2, true);
        // Either it is an incomplete update yrs keeps as pending (Ok), or InvalidData: never a panic.
        if let Err(e) = apply_update_v2(&target, bytes, None) {
            assert!(matches!(e, Error::InvalidData(_)), "{e:?}");
        }
        if let Err(e) = target.transaction(None).unwrap().apply_v2(bytes.clone()) {
            assert!(matches!(e, Error::InvalidData(_)), "{e:?}");
        }
        assert!(matches!(decode_snapshot_v2(bytes), Err(Error::InvalidData(_)) | Ok(_)));
        assert!(matches!(debug_update_v2(bytes), Err(Error::InvalidData(_)) | Ok(_)));
    }
    assert_err!(apply_update(&doc, &[0xff; 9], None), Error::InvalidData(_));
    assert_err!(decode_snapshot_v1(&[0xff; 9]), Error::InvalidData(_));
    assert_err!(decode_state_vector(Some(vec![0xff; 9])), Error::InvalidData(_));
    assert_err!(doc.transaction(None).unwrap().diff_v1(vec![0xff; 9]), Error::InvalidData(_));
}

#[test]
fn snapshots_of_a_document_with_gc_are_gc_enabled() {
    let (doc, text) = hello();
    let before = snapshot(&doc).unwrap();
    text.insert(5, "!", None, None).unwrap();
    assert_err!(encode_state_from_snapshot_v1(&doc, &before), Error::GcEnabled);
    assert_err!(encode_state_from_snapshot_v2(&doc, &before), Error::GcEnabled);

    let kept = self::doc(1, false);
    let text = kept.get_text("t").unwrap();
    text.insert(0, "hello", None, None).unwrap();
    let before = snapshot(&kept).unwrap();
    text.insert(5, "!", None, None).unwrap();
    let old = self::doc(2, true);
    apply_update(&old, &encode_state_from_snapshot_v1(&kept, &before).unwrap(), None).unwrap();
    assert_eq!(old.get_text("t").unwrap().to_string(None).unwrap(), "hello");
}

#[test]
fn messages_say_what_happened() {
    let (doc, text) = hello();
    let txn = doc.transaction(None).unwrap();
    assert_eq!(
        text.to_string(None).unwrap_err().to_string(),
        "the document is locked: a transaction is open or a callback is running"
    );
    txn.commit().unwrap();
    let e = text.insert(0, "x", Some("[1]".into()), None).unwrap_err().to_string();
    assert_eq!(e, "formatting attributes must be a JSON object: not an object: [1]");
    let e = apply_update(&doc, &[0xff; 9], None).unwrap_err().to_string();
    assert!(e.starts_with("invalid data: "), "{e}");
}
