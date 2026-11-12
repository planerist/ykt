//! Typed attributes (`insertWithAttributes`, `formatWithAttributes`), embeds of every value kind
//! (`insertEmbed`) and the text's id, through ykt's API: round trips through `toDelta`, an observer
//! and a replica; removal by `null`; the same results as the JSON calls; preliminary texts.

use crate::attrs::{YAttributes, YValue};
use crate::collection::YBranchId;
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YTextEvent, YTextObserver};
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2};
use crate::text::YText;
use crate::tools::Error;
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

fn attrs(pairs: &[(&str, YValue)]) -> YAttributes {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

fn s(text: &str) -> YValue {
    YValue::String(text.to_string())
}

fn insert(text: &str, a: Option<YAttributes>) -> YDelta {
    YDelta::YInsert(s(text), a)
}

fn replica_of(source: &YDoc) -> YDoc {
    let replica = doc(99);
    apply_update_v2(&replica, &encode_state_as_update_v2(source, None).unwrap(), None).unwrap();
    replica
}

#[derive(Default)]
struct Deltas(Mutex<Vec<Vec<YDelta>>>);

impl YTextObserver for Deltas {
    fn on_change(&self, event: YTextEvent) {
        self.0.lock().unwrap().push(event.delta);
    }
}

/// Every kind of attribute value.
fn rich() -> YAttributes {
    attrs(&[
        ("bold", YValue::Bool(true)),
        ("color", s("#ff0000")),
        ("size", YValue::Number(14.0)),
        ("link", YValue::AttrMap(attrs(&[("href", s("https://yaranga.net")), ("id", YValue::BigInt(1 << 60 | 1))]))),
        ("tags", YValue::Array(vec![s("a"), YValue::Null])),
    ])
}

#[test]
fn typed_attributes_round_trip_through_to_delta_an_observer_and_a_replica() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    let deltas = Arc::new(Deltas::default());
    let _sub = text.observe(deltas.clone()).unwrap();

    text.insert_with_attributes(0, "hello", rich(), None).unwrap();
    text.insert_with_attributes(5, " world", HashMap::new(), None).unwrap(); // plain: not the formatting before it
    text.format_with_attributes(6, 5, attrs(&[("italic", YValue::Bool(true))]), None).unwrap();

    let expected = vec![insert("hello", Some(rich())), insert(" ", None), insert("world", Some(attrs(&[("italic", YValue::Bool(true))])))];
    assert_eq!(text.to_delta(None, None, None).unwrap(), expected);
    assert_eq!(replica_of(&doc).get_text("t").unwrap().to_delta(None, None, None).unwrap(), expected);
    assert_eq!(
        *deltas.0.lock().unwrap(),
        vec![
            vec![insert("hello", Some(rich()))],
            vec![YDelta::YRetain(5, None), insert(" world", None)],
            vec![YDelta::YRetain(6, None), YDelta::YRetain(5, Some(attrs(&[("italic", YValue::Bool(true))])))],
        ]
    );
}

#[test]
fn a_null_value_removes_an_attribute() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    text.insert_with_attributes(0, "hello", attrs(&[("bold", YValue::Bool(true)), ("italic", YValue::Bool(true))]), None).unwrap();
    let deltas = Arc::new(Deltas::default());
    let _sub = text.observe(deltas.clone()).unwrap();

    text.format_with_attributes(0, 2, attrs(&[("bold", YValue::Null)]), None).unwrap();

    assert_eq!(*deltas.0.lock().unwrap(), vec![vec![YDelta::YRetain(2, Some(attrs(&[("bold", YValue::Null)])))]]);
    assert_eq!(
        text.to_delta(None, None, None).unwrap(),
        vec![
            insert("he", Some(attrs(&[("italic", YValue::Bool(true))]))),
            insert("llo", Some(attrs(&[("bold", YValue::Bool(true)), ("italic", YValue::Bool(true))]))),
        ]
    );
}

#[test]
fn inserting_with_attributes_takes_exactly_those_and_without_takes_the_surrounding_ones() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    let bold = attrs(&[("bold", YValue::Bool(true))]);
    text.insert_with_attributes(0, "ab", bold.clone(), None).unwrap();

    text.insert(1, "x", None, None).unwrap(); // takes bold
    text.insert_with_attributes(2, "y", HashMap::new(), None).unwrap(); // plain, inside bold
    text.insert_with_attributes(3, "z", attrs(&[("bold", YValue::Null), ("code", YValue::Bool(true))]), None).unwrap();

    assert_eq!(
        text.to_delta(None, None, None).unwrap(),
        vec![
            insert("ax", Some(bold.clone())),
            insert("y", None),
            insert("z", Some(attrs(&[("code", YValue::Bool(true))]))),
            insert("b", Some(bold)),
        ]
    );
}

#[test]
fn typed_and_json_calls_give_the_same_document() {
    let json = doc(1);
    let typed = doc(1);
    let (a, b) = (json.get_text("t").unwrap(), typed.get_text("t").unwrap());
    a.insert(0, "hello", Some(r#"{"bold":true,"size":14,"link":{"href":"x"}}"#.into()), None).unwrap();
    b.insert_with_attributes(
        0,
        "hello",
        attrs(&[("bold", YValue::Bool(true)), ("size", YValue::Number(14.0)), ("link", YValue::AttrMap(attrs(&[("href", s("x"))])))]),
        None,
    )
    .unwrap();
    a.format(1, 2, r#"{"bold":null}"#.into(), None).unwrap();
    b.format_with_attributes(1, 2, attrs(&[("bold", YValue::Null)]), None).unwrap();

    assert_eq!(a.to_delta(None, None, None).unwrap(), b.to_delta(None, None, None).unwrap());
    assert_eq!(
        replica_of(&json).get_text("t").unwrap().to_delta(None, None, None).unwrap(),
        replica_of(&typed).get_text("t").unwrap().to_delta(None, None, None).unwrap()
    );
}

#[test]
fn embeds_of_every_kind_come_back_as_they_were() {
    let every = vec![
        YValue::Null,
        YValue::Bool(false),
        YValue::Number(42.0),
        YValue::Number(-1.5),
        YValue::BigInt((1 << 60) + 1),
        s("an embedded string"),
        YValue::Buffer(vec![0, 1, 255]),
        YValue::Array(vec![YValue::Number(1.0), s("two")]),
        YValue::AttrMap(attrs(&[("mention", s("note://1")), ("nested", YValue::AttrMap(HashMap::new()))])),
    ];
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    let deltas = Arc::new(Deltas::default());
    let _sub = text.observe(deltas.clone()).unwrap();
    text.insert(0, "ab", None, None).unwrap();

    for (i, value) in every.iter().enumerate() {
        text.insert_embed(1 + i as u32, value.clone(), None, None).unwrap();
    }
    text.insert_embed(1, YValue::Bool(true), Some(attrs(&[("bold", YValue::Bool(true))])), None).unwrap();

    assert_eq!(text.length(None).unwrap(), 2 + every.len() as u32 + 1, "an embed is one unit");
    let mut expected = vec![insert("a", None), YDelta::YInsert(YValue::Bool(true), Some(attrs(&[("bold", YValue::Bool(true))])))];
    expected.extend(every.iter().map(|v| YDelta::YInsert(v.clone(), None)));
    expected.push(insert("b", None));
    assert_eq!(text.to_delta(None, None, None).unwrap(), expected);
    assert_eq!(replica_of(&doc).get_text("t").unwrap().to_delta(None, None, None).unwrap(), expected);
    let heard = deltas.0.lock().unwrap();
    for (i, value) in every.iter().enumerate() {
        assert_eq!(heard[1 + i], vec![YDelta::YRetain(1 + i as u32, None), YDelta::YInsert(value.clone(), None)]);
    }
}

#[test]
fn past_the_end_and_preliminary_texts() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    text.insert(0, "abc", None, None).unwrap();
    assert!(matches!(text.insert_with_attributes(4, "x", rich(), None), Err(Error::OutOfBounds)));
    assert!(matches!(text.format_with_attributes(2, 2, rich(), None), Err(Error::OutOfBounds)));
    assert!(matches!(text.insert_embed(4, YValue::Null, None, None), Err(Error::OutOfBounds)));
    assert_eq!(text.to_delta(None, None, None).unwrap(), vec![insert("abc", None)]);

    let prelim = YText::new_with_text(Some("abc".into()));
    assert!(matches!(prelim.insert_with_attributes(0, "x", rich(), None), Err(Error::NotIntegrated)));
    assert!(matches!(prelim.format_with_attributes(0, 1, rich(), None), Err(Error::NotIntegrated)));
    assert!(matches!(prelim.insert_embed(0, YValue::Null, None, None), Err(Error::NotIntegrated)));
    assert!(matches!(prelim.id(), Err(Error::NotIntegrated)));
    prelim.insert_with_attributes(3, "d", HashMap::new(), None).unwrap(); // no attributes: fine
    assert_eq!(prelim.to_string(None).unwrap(), "abcd");
}

#[test]
fn a_root_texts_id_is_its_name_on_every_replica() {
    let doc = doc(1);
    let text = doc.get_text("lines").unwrap();
    text.insert(0, "x", None, None).unwrap();
    assert_eq!(text.id().unwrap(), YBranchId::Root { name: "lines".into() });
    assert_eq!(replica_of(&doc).get_text("lines").unwrap().id().unwrap(), text.id().unwrap());
    assert_ne!(doc.get_text("other").unwrap().id().unwrap(), text.id().unwrap());
}
