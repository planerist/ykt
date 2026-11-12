//! Deep observers (texts, maps, arrays, XML nodes), XML observers and the document's
//! after-transaction, subdocument and destroy events, through ykt's API: targets, paths and changes;
//! one event per changed type per transaction; remote updates and their origin; handles usable after
//! the callback; unsubscribing inside a callback and by release; what a callback may not do.

use crate::array::YArrayDelta;
use crate::attrs::{YAttributes, YValue};
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::events::*;
use crate::map::YEntryChange;
use crate::observers::YSubscription;
use crate::output::YOutput;
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2, encode_state_vector};
use crate::tools::Error;
use crate::updates::diff_update_v2;
use crate::xml::YXmlChild;
use crate::xml_elem::YXmlElement;
use crate::xml_text::YXmlText;
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

fn sync(from: &YDoc, to: &YDoc, origin: Option<&str>) {
    let missing = diff_update_v2(encode_state_as_update_v2(from, None).unwrap(), encode_state_vector(to).unwrap()).unwrap();
    apply_update_v2(to, &missing, origin.map(|o| o.as_bytes().to_vec())).unwrap();
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

fn key(k: &str) -> YPathSegment {
    YPathSegment::Key { key: k.into() }
}

fn index(i: u32) -> YPathSegment {
    YPathSegment::Index { index: i }
}

fn entries(pairs: &[(&str, YValue)]) -> YAttributes {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}

#[derive(Default)]
struct Deep(Mutex<Vec<YDeepEvent>>);

impl YDeepObserver for Deep {
    fn on_change(&self, event: YDeepEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Deep {
    fn take(&self) -> Vec<YDeepEvent> {
        std::mem::take(&mut *self.0.lock().unwrap())
    }
}

/// The paths of one transaction's events, in their order.
fn paths(event: &YDeepEvent) -> Vec<Vec<YPathSegment>> {
    event
        .events
        .iter()
        .map(|e| match e {
            YEvent::TextChange { path, .. } | YEvent::MapChange { path, .. } | YEvent::ArrayChange { path, .. } => path.clone(),
            YEvent::XmlChange { path, .. } | YEvent::XmlTextChange { path, .. } => path.clone(),
            YEvent::WeakLinkChange { path, .. } => path.clone(),
        })
        .collect()
}

// Texts ----------------------------------------------------------------------------------------------

#[test]
fn a_texts_deep_observer_hears_the_text_and_a_map_embedded_in_it() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    text.insert(0, "ab", None, None).unwrap();
    let table = text.insert_embed_map(1, None, None, None).unwrap();
    let deep = Arc::new(Deep::default());
    let _sub = text.observe_deep(deep.clone()).unwrap();

    text.insert(0, ">", None, None).unwrap();
    table.set("rows", n(2.0), None).unwrap();
    let txn = Arc::new(doc.transaction(Some("editor".into())).unwrap());
    table.set("cols", n(3.0), Some(txn.clone())).unwrap();
    text.delete(0, 1, Some(txn.clone())).unwrap();
    txn.commit().unwrap();

    let events = deep.take();
    assert_eq!(events.len(), 3);
    match &events[0].events[..] {
        [YEvent::TextChange { target, path, delta }] => {
            assert_eq!(**target, text);
            assert!(path.is_empty());
            assert_eq!(*delta, vec![YDelta::YInsert(s(">"), None)]);
        }
        other => panic!("{other:?}"),
    }
    match &events[1].events[..] {
        [YEvent::MapChange { target, path, keys }] => {
            assert_eq!(**target, *table);
            assert_eq!(*path, vec![index(2)], "the embed is at index 2 of the text now");
            assert_eq!(keys["rows"], YEntryChange::Inserted { value: out(n(2.0)) });
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(paths(&events[2]), vec![vec![], vec![index(1)]], "one transaction, both types, the text first");
    assert_eq!(events[2].origin, Some(b"editor".to_vec()));
}

// Maps and arrays ----------------------------------------------------------------------------------------

#[test]
fn a_maps_deep_observer_hears_nested_types_with_their_paths() {
    let doc = doc(1);
    let root = doc.get_map("root").unwrap();
    let meta = root.set_map("meta", None, None).unwrap();
    let list = root.set_array("list", None, None).unwrap();
    let item = list.insert_map(0, None, None).unwrap();
    let title = item.set_text("title", None, None).unwrap();
    let deep = Arc::new(Deep::default());
    let _sub = root.observe_deep(deep.clone()).unwrap();

    root.set("plain", n(1.0), None).unwrap();
    meta.set("pinned", YValue::Bool(true), None).unwrap();
    list.push(vec![s("x")], None).unwrap();
    title.insert(0, "hi", None, None).unwrap();

    let events = deep.take();
    assert_eq!(
        events.iter().map(paths).collect::<Vec<_>>(),
        vec![vec![vec![]], vec![vec![key("meta")]], vec![vec![key("list")]], vec![vec![key("list"), index(0), key("title")]]]
    );
    match &events[3].events[0] {
        YEvent::TextChange { target, delta, .. } => {
            assert_eq!(**target, *title);
            assert_eq!(*delta, vec![YDelta::YInsert(s("hi"), None)]);
        }
        other => panic!("{other:?}"),
    }
    match &events[2].events[0] {
        YEvent::ArrayChange { target, delta, .. } => {
            assert_eq!(**target, *list);
            assert_eq!(*delta, vec![YArrayDelta::Retain(1), YArrayDelta::Insert(vec![out(s("x"))])]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn an_arrays_deep_observer_hears_remote_changes_with_their_origin_and_targets_stay_usable() {
    let a = doc(1);
    let list = a.get_array("list").unwrap();
    let item = list.insert_map(0, Some(entries(&[("k", n(1.0))])), None).unwrap();
    let b = doc(2);
    sync(&a, &b, None);
    let b_list = b.get_array("list").unwrap();
    let deep = Arc::new(Deep::default());
    let _sub = b_list.observe_deep(deep.clone()).unwrap();

    item.set("k", n(2.0), None).unwrap();
    sync(&a, &b, Some("server"));

    let events = deep.take();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].origin, Some(b"server".to_vec()));
    match &events[0].events[..] {
        [YEvent::MapChange { target, path, keys }] => {
            assert_eq!(*path, vec![index(0)]);
            assert_eq!(keys["k"], YEntryChange::Updated { old_value: out(n(1.0)), new_value: out(n(2.0)) });
            // The handle works after the callback, on the replica.
            assert_eq!(target.get("k", None).unwrap(), Some(n(2.0)));
            target.set("k", n(3.0), None).unwrap();
        }
        other => panic!("{other:?}"),
    }
}

// XML ----------------------------------------------------------------------------------------------

#[derive(Default)]
struct Xml(Mutex<Vec<YXmlEvent>>);

impl YXmlObserver for Xml {
    fn on_change(&self, event: YXmlEvent) {
        self.0.lock().unwrap().push(event);
    }
}

fn element(name: &str) -> YXmlChild {
    YXmlChild::Element(Arc::new(YXmlElement::new(name.into(), None, None).unwrap()))
}

#[test]
fn xml_observers_hear_children_attributes_and_text() {
    let doc = doc(1);
    let fragment = doc.get_xml_fragment("x").unwrap();
    let heard = Arc::new(Xml::default());
    let _fragment_sub = fragment.observe(heard.clone()).unwrap();
    fragment.push(element("p"), None).unwrap();
    let p = match fragment.first_child(None).unwrap() {
        Some(YXmlChild::Element(p)) => p,
        other => panic!("{other:?}"),
    };
    let element_heard = Arc::new(Xml::default());
    let _element_sub = p.observe(element_heard.clone()).unwrap();
    p.set_attribute("class", s("a"), None).unwrap();
    p.set_attribute("class", s("b"), None).unwrap();
    p.remove_attribute("class".into(), None).unwrap();
    p.push(YXmlChild::Text(Arc::new(YXmlText::new("hi".into(), None))), None).unwrap();

    let events = heard.0.lock().unwrap();
    assert_eq!(events.len(), 1, "the fragment hears its own children only");
    match &events[0].children[..] {
        [YArrayDelta::Insert(nodes)] => assert!(matches!(&nodes[..], [YOutput::XmlElement { element }] if **element == *p)),
        other => panic!("{other:?}"),
    }
    let attrs: Vec<_> = element_heard.0.lock().unwrap().iter().map(|e| e.attributes.clone()).collect();
    assert_eq!(
        attrs[..3],
        [
            HashMap::from([("class".to_string(), YEntryChange::Inserted { value: out(s("a")) })]),
            HashMap::from([("class".to_string(), YEntryChange::Updated { old_value: out(s("a")), new_value: out(s("b")) })]),
            HashMap::from([("class".to_string(), YEntryChange::Removed { old_value: out(s("b")) })]),
        ]
    );
    let last = &element_heard.0.lock().unwrap()[3];
    assert!(matches!(&last.target, YXmlChild::Element(e) if **e == *p));
    assert!(matches!(&last.children[..], [YArrayDelta::Insert(nodes)] if matches!(nodes[..], [YOutput::XmlText { .. }])));

    // An XML text's observer: its text's delta and its XML attributes.
    let text = match p.first_child(None).unwrap() {
        Some(YXmlChild::Text(t)) => t,
        other => panic!("{other:?}"),
    };
    let text_heard = Arc::new(Xml::default());
    let _text_sub = text.observe(text_heard.clone()).unwrap();
    text.insert(2, "!", None, None).unwrap();
    text.set_attribute("lang", s("en"), None).unwrap();
    let events = text_heard.0.lock().unwrap();
    assert_eq!(events[0].delta, vec![YDelta::YRetain(2, None), YDelta::YInsert(s("!"), None)]);
    assert!(events[0].children.is_empty());
    assert_eq!(events[1].attributes, HashMap::from([("lang".to_string(), YEntryChange::Inserted { value: out(s("en")) })]));
}

#[test]
fn an_xml_fragments_deep_observer_hears_nested_nodes() {
    let doc = doc(1);
    let fragment = doc.get_xml_fragment("x").unwrap();
    fragment.push(element("div"), None).unwrap();
    let div = match fragment.first_child(None).unwrap() {
        Some(YXmlChild::Element(e)) => e,
        _ => unreachable!(),
    };
    div.push(YXmlChild::Text(Arc::new(YXmlText::new("hello".into(), None))), None).unwrap();
    let text = match div.first_child(None).unwrap() {
        Some(YXmlChild::Text(t)) => t,
        _ => unreachable!(),
    };
    let deep = Arc::new(Deep::default());
    let _sub = fragment.observe_deep(deep.clone()).unwrap();

    div.set_attribute("id", s("main"), None).unwrap();
    text.format(0, 5, HashMap::from([("bold".to_string(), YValue::Bool(true))]), None).unwrap();

    let events = deep.take();
    match &events[0].events[..] {
        [YEvent::XmlChange { target: YXmlChild::Element(e), path, attributes, .. }] => {
            assert_eq!(**e, *div);
            assert_eq!(*path, vec![index(0)]);
            assert_eq!(attributes["id"], YEntryChange::Inserted { value: out(s("main")) });
        }
        other => panic!("{other:?}"),
    }
    match &events[1].events[..] {
        [YEvent::XmlTextChange { target, path, delta, .. }] => {
            assert_eq!(**target, *text);
            assert_eq!(*path, vec![index(0), index(0)]);
            assert_eq!(*delta, vec![YDelta::YRetain(5, Some(HashMap::from([("bold".to_string(), YValue::Bool(true))])))]);
        }
        other => panic!("{other:?}"),
    }
}

// Subscriptions ---------------------------------------------------------------------------------------

/// Unsubscribes itself on its first event, and tries to read the document there.
struct OneShot {
    sub: Mutex<Option<Arc<YSubscription>>>,
    doc: YDoc,
    results: Mutex<Vec<bool>>,
}

impl YDeepObserver for OneShot {
    fn on_change(&self, _event: YDeepEvent) {
        let locked = matches!(self.doc.get_map("root"), Err(Error::Locked));
        self.results.lock().unwrap().push(locked);
        if let Some(sub) = self.sub.lock().unwrap().take() {
            assert!(sub.unsubscribe());
        }
    }
}

#[test]
fn deep_subscriptions_end_inside_their_callback_and_on_release() {
    let doc = doc(1);
    let root = doc.get_map("root").unwrap();
    let observer = Arc::new(OneShot { sub: Mutex::new(None), doc: YDoc(doc.0.clone()), results: Mutex::new(vec![]) });
    let sub = root.observe_deep(observer.clone()).unwrap();
    *observer.sub.lock().unwrap() = Some(sub);
    let released = Arc::new(Deep::default());
    {
        let _sub = root.observe_deep(released.clone()).unwrap();
        root.set("a", n(1.0), None).unwrap();
    }
    root.set("b", n(2.0), None).unwrap();

    assert_eq!(*observer.results.lock().unwrap(), vec![true], "once, and the document is locked in it");
    assert_eq!(released.take().len(), 1);
    let prelim = crate::text::YText::new_with_text(None);
    assert!(matches!(prelim.observe_deep(Arc::new(Deep::default())), Err(Error::NotIntegrated)));
}

// The document's events --------------------------------------------------------------------------------

#[derive(Default)]
struct DocEvents {
    after: Mutex<Vec<YAfterTransactionEvent>>,
    subdocs: Mutex<Vec<YSubdocsEvent>>,
    destroyed: Mutex<Vec<YDestroyEvent>>,
}

impl YAfterTransactionObserver for DocEvents {
    fn on_after_transaction(&self, event: YAfterTransactionEvent) {
        self.after.lock().unwrap().push(event);
    }
}

impl YSubdocsObserver for DocEvents {
    fn on_subdocs(&self, event: YSubdocsEvent) {
        self.subdocs.lock().unwrap().push(event);
    }
}

impl YDestroyObserver for DocEvents {
    fn on_destroy(&self, event: YDestroyEvent) {
        self.destroyed.lock().unwrap().push(event);
    }
}

#[test]
fn after_transaction_events_come_for_every_transaction() {
    let doc = doc(7);
    let text = doc.get_text("t").unwrap();
    let heard = Arc::new(DocEvents::default());
    let sub = doc.observe_after_transaction(heard.clone()).unwrap();

    text.insert(0, "hello", None, None).unwrap();
    let txn = Arc::new(doc.transaction(Some("cut".into())).unwrap());
    text.delete(0, 2, Some(txn.clone())).unwrap();
    txn.commit().unwrap();
    doc.transaction(Some("empty".into())).unwrap().commit().unwrap();
    assert!(sub.unsubscribe());
    text.insert(0, "x", None, None).unwrap();

    let events = heard.after.lock().unwrap();
    assert_eq!(events.len(), 3, "not its own registration, not after unsubscribing");
    assert_eq!(events[0].after_state.get(&7), Some(&5));
    assert_eq!(events[1].origin, Some(b"cut".to_vec()));
    assert!(events[1].delete_set.len() > 1);
    assert_eq!(events[2].before_state, events[2].after_state);
    let locked = doc.transaction(None).unwrap();
    assert!(matches!(doc.observe_after_transaction(heard.clone()), Err(Error::Locked)));
    locked.commit().unwrap();
}

#[test]
fn subdocument_events_and_destroy() {
    let doc = doc(1);
    let root = doc.get_map("root").unwrap();
    let heard = Arc::new(DocEvents::default());
    let _sub = doc.observe_subdocs(heard.clone()).unwrap();

    let page = root.set_doc("page", &YDoc::new(None), None).unwrap();
    let guid = page.guid();
    let destroyed = Arc::new(DocEvents::default());
    let _destroy_sub = page.observe_destroy(destroyed.clone()).unwrap();
    page.destroy().unwrap();
    root.delete("page", None).unwrap();

    let events = heard.subdocs.lock().unwrap();
    let guids = |docs: &Vec<Arc<YDoc>>| docs.iter().map(|d| d.guid()).collect::<Vec<_>>();
    assert_eq!(guids(&events[0].added), vec![guid.clone()]);
    assert_eq!(guids(&events[0].loaded), vec![guid.clone()], "should_load: added and loaded");
    assert_eq!(guids(&events[1].removed), vec![guid.clone()], "destroyed: replaced by a new instance");
    assert_eq!(guids(&events[2].removed), vec![guid.clone()], "deleted from the map");
    assert_eq!(destroyed.destroyed.lock().unwrap().iter().map(|e| e.guid.clone()).collect::<Vec<_>>(), vec![guid]);
}

#[test]
fn a_subdocument_made_lazy_loads_on_request() {
    let doc = doc(1);
    let root = doc.get_map("root").unwrap();
    let lazy = YDoc::new(Some(YDocOptions {
        client_id: None,
        guid: Some("lazy".into()),
        collection_id: None,
        gc: None,
        auto_load: None,
        should_load: Some(false),
        bytes_offset: None,
    }));
    let heard = Arc::new(DocEvents::default());
    let _sub = doc.observe_subdocs(heard.clone()).unwrap();
    let sub = root.set_doc("lazy", &lazy, None).unwrap();
    sub.load().unwrap();

    let events = heard.subdocs.lock().unwrap();
    assert!(events[0].loaded.is_empty(), "not loaded until asked");
    assert_eq!(events[1].loaded.iter().map(|d| d.guid()).collect::<Vec<_>>(), vec!["lazy".to_string()]);
    assert!(sub.should_load());
    assert_eq!(sub.parent_doc().unwrap().map(|p| p.guid()), Some(doc.guid()));
    assert!(!sub.prelim().unwrap());
    assert!(doc.prelim().unwrap());
}

#[test]
fn destroy_and_load_throw_while_locked() {
    let doc = doc(1);
    let page = doc.get_map("root").unwrap().set_doc("page", &YDoc::new(None), None).unwrap();
    let txn = doc.transaction(None).unwrap();
    assert!(matches!(page.destroy(), Err(Error::Locked)), "the parent is locked");
    assert!(matches!(page.load(), Err(Error::Locked)));
    txn.commit().unwrap();
    let own = page.transaction(None).unwrap();
    assert!(matches!(page.destroy(), Err(Error::Locked)));
    assert!(matches!(page.parent_doc(), Err(Error::Locked)));
    own.commit().unwrap();
    page.destroy().unwrap();
}
