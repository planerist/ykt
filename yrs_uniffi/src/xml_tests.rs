//! XML texts and trees, through ykt's API: `YXmlText.applyDelta` (formatting, inserts, embeds,
//! deletes; replicating; what it throws).

use crate::attrs::{YAttributes, YValue};
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::events::{YXmlEvent, YXmlObserver};
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2};
use crate::tools::Error;
use crate::xml::{YDeltaXmlChild, YXmlChild, YXmlDelta};
use crate::xml_text::YXmlText;
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

/// Everything `from` has, applied to `to` (lib0 v2).
fn sync(from: &YDoc, to: &YDoc) {
    apply_update_v2(to, &encode_state_as_update_v2(from, None).unwrap(), None).unwrap();
}

fn s(v: &str) -> YValue {
    YValue::String(v.to_string())
}

fn attrs(pairs: &[(&str, YValue)]) -> Option<YAttributes> {
    Some(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect())
}

/// An XML text with `text` in fragment "x" of `doc`.
fn xml_text(doc: &YDoc, text: &str) -> Arc<YXmlText> {
    let node = Arc::new(YXmlText::new(text.to_string(), None));
    doc.get_xml_fragment("x").unwrap().push(YXmlChild::Text(node.clone()), None).unwrap();
    node
}

/// The inserts of an XML text's delta: (value, attributes).
fn inserts(text: &YXmlText) -> Vec<(YValue, Option<YAttributes>)> {
    text.to_delta(None, None, None)
        .unwrap()
        .into_iter()
        .map(|d| match d {
            YXmlDelta::YInsert(YDeltaXmlChild::Embed(value, attrs)) => (value, attrs),
            _ => panic!("not a plain insert"),
        })
        .collect()
}

#[test]
fn apply_delta_formats_inserts_and_deletes_and_replicates() {
    let a = doc(1);
    let text = xml_text(&a, "hello world");
    text.apply_delta(
        vec![
            YDelta::YRetain(5, attrs(&[("bold", YValue::Bool(true))])),
            YDelta::YInsert(s(","), None),
            YDelta::YRetain(1, None),
            YDelta::YDelete(5),
            YDelta::YInsert(s("there"), attrs(&[("italic", YValue::Bool(true))])),
            YDelta::YInsert(YValue::AttrMap(attrs(&[("mention", s("note://42"))]).unwrap()), None),
        ],
        None,
    )
    .unwrap();

    let expected = vec![
        (s("hello"), attrs(&[("bold", YValue::Bool(true))])),
        (s(", "), None),
        (s("there"), attrs(&[("italic", YValue::Bool(true))])),
        (YValue::AttrMap(attrs(&[("mention", s("note://42"))]).unwrap()), None),
    ];
    assert_eq!(inserts(&text), expected);
    assert_eq!(text.length(None).unwrap(), 13, "an embed takes one unit");

    let b = doc(2);
    sync(&a, &b);
    let replica = match b.get_xml_fragment("x").unwrap().first_child(None).unwrap() {
        Some(YXmlChild::Text(text)) => text,
        other => panic!("not a text: {other:?}"),
    };
    assert_eq!(inserts(&replica), expected);

    // Removing formatting with a null attribute, from the replica back.
    replica.apply_delta(vec![YDelta::YRetain(5, attrs(&[("bold", YValue::Null)]))], None).unwrap();
    sync(&b, &a);
    assert_eq!(inserts(&text)[0], (s("hello, "), None));
}

#[test]
fn apply_delta_in_a_transaction_reaches_the_observer_as_one_change() {
    #[derive(Default)]
    struct Heard(Mutex<Vec<Vec<YDelta>>>);
    impl YXmlObserver for Heard {
        fn on_change(&self, event: YXmlEvent) {
            self.0.lock().unwrap().push(event.delta);
        }
    }

    let doc = doc(1);
    let text = xml_text(&doc, "ab");
    let heard = Arc::new(Heard::default());
    let _sub = text.observe(heard.clone()).unwrap();
    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    text.apply_delta(vec![YDelta::YRetain(1, None), YDelta::YInsert(s("X"), None)], Some(txn.clone())).unwrap();
    text.apply_delta(vec![YDelta::YDelete(1)], Some(txn.clone())).unwrap();
    assert_eq!(text.to_string(Some(txn.clone())).unwrap(), "Xb");
    txn.commit().unwrap();

    assert_eq!(*heard.0.lock().unwrap(), vec![vec![YDelta::YDelete(1), YDelta::YInsert(s("X"), None)]]);
}

#[test]
fn apply_delta_edges_and_errors() {
    let doc = doc(1);
    let text = xml_text(&doc, "abc");

    // A delete past the end throws before anything is applied.
    let past_end = vec![YDelta::YInsert(s("!"), None), YDelta::YRetain(2, None), YDelta::YDelete(5)];
    assert!(matches!(text.apply_delta(past_end, None), Err(Error::OutOfBounds)));
    assert_eq!(text.to_string(None).unwrap(), "abc");

    // A retain past the end stops at it; an empty delta changes nothing.
    text.apply_delta(vec![YDelta::YRetain(10, None), YDelta::YInsert(s("d"), None)], None).unwrap();
    text.apply_delta(vec![], None).unwrap();
    assert_eq!(text.to_string(None).unwrap(), "abcd");

    // A preliminary XML text has nothing to apply a delta to yet.
    let prelim = YXmlText::new("x".into(), None);
    assert!(matches!(prelim.apply_delta(vec![YDelta::YDelete(1)], None), Err(Error::NotIntegrated)));

    // While a transaction is open, a call without it throws Locked; so does one in a callback.
    let txn = doc.transaction(None).unwrap();
    assert!(matches!(text.apply_delta(vec![YDelta::YDelete(1)], None), Err(Error::Locked)));
    drop(txn);
    struct Inside(Arc<YXmlText>, Mutex<Vec<bool>>);
    impl YXmlObserver for Inside {
        fn on_change(&self, _event: YXmlEvent) {
            let locked = matches!(self.0.apply_delta(vec![YDelta::YDelete(1)], None), Err(Error::Locked));
            self.1.lock().unwrap().push(locked);
        }
    }
    let inside = Arc::new(Inside(text.clone(), Mutex::new(vec![])));
    let _sub = text.observe(inside.clone()).unwrap();
    text.insert(4, "e", None, None).unwrap();
    assert_eq!(*inside.1.lock().unwrap(), vec![true]);
    assert_eq!(text.to_string(None).unwrap(), "abcde");
}
