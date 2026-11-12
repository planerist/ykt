//! Weak links through ykt's API: links to map entries (`YMap.link`) and quotes of array and text
//! ranges (`YArray.quote`, `YText.quote`); inserting them into maps, arrays and texts; reading them
//! (`unquote`, `deref`, `toText`)
//! locally and on a replica after v2 updates, with edits inside the quoted range from either side;
//! their bounds and errors; their observers, and what a callback may not do.

use crate::array::YArray;
use crate::attrs::{YAttributes, YValue};
use crate::delta::YDelta;
use crate::doc::{YDoc, YDocOptions};
use crate::events::{YDeepEvent, YDeepObserver, YEvent, YPathSegment};
use crate::map::YMap;
use crate::observers::{YTextEvent, YTextObserver};
use crate::text::YText;
use crate::output::YOutput;
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2, encode_state_vector};
use crate::tools::Error;
use crate::updates::diff_update_v2;
use crate::weak::{YWeakLink, YWeakLinkEvent, YWeakLinkObserver};
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

/// What `from` has that `to` lacks, as a v2 update.
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

fn values(link: &YWeakLink) -> Vec<YOutput> {
    link.unquote(None).unwrap()
}

fn link_of(output: Option<YOutput>) -> Arc<YWeakLink> {
    match output {
        Some(YOutput::WeakLink { link }) => link,
        other => panic!("not a weak link: {other:?}"),
    }
}

fn numbers(array: &YArray) {
    array.push((1..=4).map(|i| n(i as f64)).collect(), None).unwrap();
}

#[test]
fn a_map_link_follows_its_entry() {
    let doc = doc(1);
    let settings = doc.get_map("settings").unwrap();
    settings.set("theme", s("dark"), None).unwrap();

    let quote = settings.link("theme", None).unwrap().expect("the key is there");
    assert!(quote.prelim());
    assert!(matches!(quote.id(), Err(Error::NotIntegrated)));
    assert!(matches!(quote.deref(None), Err(Error::NotIntegrated)), "a quote isn't read before it is inserted");

    let current = settings.set_weak_link("current", &quote, None).unwrap();
    assert!(!current.prelim());
    assert_eq!(current.deref(None).unwrap(), Some(out(s("dark"))));
    assert_eq!(values(&current), vec![out(s("dark"))]);

    settings.set("theme", s("light"), None).unwrap();
    assert_eq!(current.deref(None).unwrap(), Some(out(s("light"))), "the entry's value now");

    // Read back: a handle of the same link; the plain API gives its JSON form, null.
    let again = link_of(settings.get_output("current", None).unwrap());
    assert_eq!(*again, *current);
    assert_eq!(settings.get("current", None).unwrap(), Some(YValue::Null));

    // A shared type in the entry comes as its handle.
    let meta = settings.set_map("meta", None, None).unwrap();
    let to_meta = settings.set_weak_link("toMeta", &settings.link("meta", None).unwrap().unwrap(), None).unwrap();
    assert_eq!(to_meta.deref(None).unwrap(), Some(YOutput::Map { map: meta.clone() }));

    settings.delete("theme", None).unwrap();
    assert_eq!(current.deref(None).unwrap(), None);
    assert_eq!(values(&current), vec![]);
    assert!(settings.link("theme", None).unwrap().is_none(), "a deleted key has no entry to link");
    assert!(settings.link("missing", None).unwrap().is_none());
}

#[test]
fn an_array_quote_follows_its_range_across_replicas() {
    let a = doc(1);
    let list = a.get_array("list").unwrap();
    numbers(&list);
    let notes = a.get_map("notes").unwrap();
    let link = notes.set_weak_link("q", &list.quote(1, 2, None).unwrap(), None).unwrap();
    assert_eq!(values(&link), vec![out(n(2.0)), out(n(3.0))]);
    assert_eq!(link.deref(None).unwrap(), Some(out(n(2.0))), "a quote's first value");

    // Both ends are in the quote: inserts inside it join it, inserts at its edges don't.
    list.insert(2, vec![s("inside")], None).unwrap();
    list.insert(1, vec![s("before")], None).unwrap();
    list.insert(5, vec![s("after")], None).unwrap();
    assert_eq!(list.to_json(None).unwrap(), vec![n(1.0), s("before"), n(2.0), s("inside"), n(3.0), s("after"), n(4.0)]);
    assert_eq!(values(&link), vec![out(n(2.0)), out(s("inside")), out(n(3.0))]);

    // The link reaches a replica in a v2 update, and reads the same there.
    let b = doc(2);
    sync(&a, &b, None);
    let replica = link_of(b.get_map("notes").unwrap().get_output("q", None).unwrap());
    assert_eq!(values(&replica), values(&link));
    assert_eq!(replica.id().unwrap(), link.id().unwrap());

    // The replica edits inside the range; both see it.
    let b_list = b.get_array("list").unwrap();
    b_list.insert(3, vec![s("remote")], None).unwrap();
    b_list.delete(4, 1, None).unwrap(); // "inside"
    sync(&b, &a, None);
    let expected = vec![out(n(2.0)), out(s("remote")), out(n(3.0))];
    assert_eq!(values(&replica), expected);
    assert_eq!(values(&link), expected);

    // Deleting every quoted value leaves it empty, for good.
    list.delete(2, 3, None).unwrap();
    assert_eq!(values(&link), vec![]);
    assert_eq!(link.deref(None).unwrap(), None);
    list.insert(2, vec![s("new")], None).unwrap();
    assert_eq!(values(&link), vec![]);
    sync(&a, &b, None);
    assert_eq!(values(&replica), vec![]);
}

#[test]
fn quotes_lie_within_the_array_and_hold_a_value() {
    let doc = doc(1);
    let list = doc.get_array("list").unwrap();
    assert!(matches!(list.quote(0, 1, None), Err(Error::OutOfBounds)), "an empty array");
    numbers(&list);
    let whole = list.quote(0, 4, None).unwrap();
    let last = list.quote(3, 1, None).unwrap();
    for (index, length) in [(0, 0), (2, 0), (4, 1), (3, 2), (0, 5), (1, u32::MAX)] {
        assert!(matches!(list.quote(index, length, None), Err(Error::OutOfBounds)), "{index}, {length}");
    }
    let notes = doc.get_map("notes").unwrap();
    assert_eq!(values(&notes.set_weak_link("whole", &whole, None).unwrap()).len(), 4);
    assert_eq!(values(&notes.set_weak_link("last", &last, None).unwrap()), vec![out(n(4.0))]);
}

#[test]
fn links_go_into_maps_arrays_and_texts() {
    let a = doc(1);
    let list = a.get_array("list").unwrap();
    numbers(&list);
    let quote = list.quote(0, 2, None).unwrap();

    // A preliminary link can be inserted again and again: each time a new link to the same values.
    let notes = a.get_map("notes").unwrap();
    let in_map = notes.set_weak_link("q", &quote, None).unwrap();
    let refs = a.get_array("refs").unwrap();
    let in_array = refs.insert_weak_link(0, &quote, None).unwrap();
    let text = a.get_text("body").unwrap();
    text.insert(0, "see ", None, None).unwrap();
    let bold: YAttributes = [("bold".to_string(), YValue::Bool(true))].into();
    let in_text = text.insert_embed_weak_link(4, &quote, Some(bold.clone()), None).unwrap();
    // An integrated link is copied: a new link quoting the same.
    let copy = notes.set_weak_link("copy", &in_text, None).unwrap();
    for link in [&in_map, &in_array, &in_text, &copy] {
        assert_eq!(values(link), vec![out(n(1.0)), out(n(2.0))]);
    }
    assert_ne!(*in_map, *in_array, "different links");
    assert_eq!(*in_array, *link_of(Some(refs.get_output(0, None).unwrap())));
    // In a text, a link is an embed of one unit; deltas give its JSON form, null.
    assert_eq!(text.length(None).unwrap(), 5);
    assert_eq!(text.to_delta(None, None, None).unwrap()[1], YDelta::YInsert(YValue::Null, Some(bold)));

    // All of them reach a replica.
    let b = doc(2);
    sync(&a, &b, None);
    let b_refs = b.get_array("refs").unwrap();
    let b_list = b.get_array("list").unwrap();
    b_list.insert(1, vec![s("x")], None).unwrap();
    assert_eq!(values(&link_of(Some(b_refs.get_output(0, None).unwrap()))), vec![out(n(1.0)), out(s("x")), out(n(2.0))]);
    assert_eq!(values(&link_of(b.get_map("notes").unwrap().get_output("copy", None).unwrap())).len(), 3);

    // Past the end; another document's link; a deleted link to copy.
    assert!(matches!(refs.insert_weak_link(5, &quote, None), Err(Error::OutOfBounds)));
    assert!(matches!(text.insert_embed_weak_link(9, &quote, None, None), Err(Error::OutOfBounds)));
    assert!(matches!(b_refs.insert_weak_link(0, &quote, None), Err(Error::WrongDocument)));
    assert!(matches!(b.get_map("notes").unwrap().set_weak_link("k", &in_map, None), Err(Error::WrongDocument)));
    notes.delete("q", None).unwrap();
    assert!(matches!(notes.set_weak_link("again", &in_map, None), Err(Error::TypeDeleted)));
    assert_eq!(refs.len(None).unwrap(), 1, "nothing inserted by the failed calls");
}

#[derive(Default)]
struct Heard(Mutex<Vec<YWeakLinkEvent>>);

impl YWeakLinkObserver for Heard {
    fn on_change(&self, event: YWeakLinkEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[derive(Default)]
struct Deep(Mutex<Vec<YDeepEvent>>);

impl YDeepObserver for Deep {
    fn on_change(&self, event: YDeepEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[test]
fn a_links_observer_hears_its_range_change_locally_and_remotely() {
    let a = doc(1);
    let list = a.get_array("list").unwrap();
    numbers(&list);
    let link = a.get_map("notes").unwrap().set_weak_link("q", &list.quote(1, 2, None).unwrap(), None).unwrap();
    let b = doc(2);
    sync(&a, &b, None);

    let heard = Arc::new(Heard::default());
    let sub = link.observe(heard.clone()).unwrap();
    list.insert(2, vec![s("inside")], None).unwrap();
    list.push(vec![s("outside")], None).unwrap();
    b.get_array("list").unwrap().delete(1, 1, None).unwrap();
    sync(&b, &a, Some("server"));
    {
        let events = heard.0.lock().unwrap();
        assert_eq!(events.iter().map(|e| e.origin.clone()).collect::<Vec<_>>(), vec![None, Some(b"server".to_vec())]);
        assert_eq!(*events[0].target, *link, "the target is the link, usable after the callback");
    }
    assert_eq!(values(&link), vec![out(s("inside")), out(n(3.0))]);
    assert!(sub.unsubscribe());
    list.delete(2, 1, None).unwrap();
    assert_eq!(heard.0.lock().unwrap().len(), 2);
}

#[test]
fn a_map_links_observers_hear_the_entry_and_the_types_in_it() {
    let doc = doc(1);
    let settings = doc.get_map("settings").unwrap();
    let meta = settings.set_map("meta", None, None).unwrap();
    let link = settings.set_weak_link("current", &settings.link("meta", None).unwrap().unwrap(), None).unwrap();

    let deep = Arc::new(Deep::default());
    let _deep_sub = link.observe_deep(deep.clone()).unwrap();
    let heard = Arc::new(Heard::default());
    let _sub = link.observe(heard.clone()).unwrap();
    meta.set("color", s("red"), None).unwrap();
    settings.set("meta", s("replaced"), None).unwrap();

    let events = std::mem::take(&mut *deep.0.lock().unwrap());
    match &events[0].events[..] {
        [YEvent::MapChange { target, .. }] => assert_eq!(**target, *meta),
        other => panic!("not the linked map's change: {other:?}"),
    }
    assert!(
        events[1].events.iter().any(|e| matches!(e, YEvent::WeakLinkChange { target, .. } if **target == *link)),
        "the entry replaced: the link's own change"
    );
    assert_eq!(heard.0.lock().unwrap().len(), 1, "observe hears the entry, not the types in it");
    assert_eq!(link.deref(None).unwrap(), Some(out(s("replaced"))));

    // A deep observer of the map holding the link hears the link change too.
    let outer = Arc::new(Deep::default());
    let _outer_sub = settings.observe_deep(outer.clone()).unwrap();
    settings.set("meta", s("again"), None).unwrap();
    let paths: Vec<Vec<YPathSegment>> = outer.0.lock().unwrap()[0]
        .events
        .iter()
        .filter_map(|e| match e {
            YEvent::WeakLinkChange { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(paths, vec![vec![YPathSegment::Key { key: "current".into() }]]);
}

#[test]
fn inside_a_callback_links_and_quotes_are_locked() {
    struct Inside {
        link: Arc<YWeakLink>,
        list: Arc<YArray>,
        map: Arc<YMap>,
        results: Mutex<Vec<bool>>,
    }
    impl YWeakLinkObserver for Inside {
        fn on_change(&self, _event: YWeakLinkEvent) {
            let locked = |r: Result<(), Error>| matches!(r, Err(Error::Locked));
            let mut results = self.results.lock().unwrap();
            results.push(locked(self.link.unquote(None).map(|_| ())));
            results.push(locked(self.link.deref(None).map(|_| ())));
            results.push(locked(self.list.quote(0, 1, None).map(|_| ())));
            results.push(locked(self.map.link("q", None).map(|_| ())));
            results.push(locked(self.map.set_weak_link("k", &self.link, None).map(|_| ())));
            results.push(locked(self.list.insert_weak_link(0, &self.link, None).map(|_| ())));
        }
    }

    let doc = doc(1);
    let list = Arc::new(doc.get_array("list").unwrap());
    numbers(&list);
    let map = Arc::new(doc.get_map("notes").unwrap());
    let link = map.set_weak_link("q", &list.quote(0, 2, None).unwrap(), None).unwrap();
    let inside = Arc::new(Inside { link: link.clone(), list: list.clone(), map: map.clone(), results: Mutex::new(vec![]) });
    let _sub = link.observe(inside.clone()).unwrap();
    list.insert(1, vec![s("x")], None).unwrap();
    assert_eq!(*inside.results.lock().unwrap(), vec![true; 6]);
    assert_eq!(map.len(None).unwrap(), 1);
    assert_eq!(list.len(None).unwrap(), 5);

    // While a transaction is open, calls without it throw Locked; with it, they work.
    let txn = Arc::new(doc.transaction(None).unwrap());
    assert!(matches!(link.unquote(None), Err(Error::Locked)));
    assert!(matches!(list.quote(0, 1, None), Err(Error::Locked)));
    assert_eq!(link.unquote(Some(txn.clone())).unwrap().len(), 3);
    let quote = list.quote(0, 1, Some(txn.clone())).unwrap();
    assert_eq!(map.set_weak_link("first", &quote, Some(txn.clone())).unwrap().deref(Some(txn.clone())).unwrap(), Some(out(n(1.0))));
    txn.commit().unwrap();
}

#[test]
fn a_text_quote_follows_its_range_across_replicas() {
    let a = doc(1);
    let text = a.get_text("body").unwrap();
    text.insert(0, "hello world", None, None).unwrap();
    let quote = text.quote(6, 5, None).unwrap();
    assert!(quote.prelim());
    assert!(matches!(quote.to_string(None), Err(Error::NotIntegrated)));
    let excerpt = a.get_map("notes").unwrap().set_weak_link("excerpt", &quote, None).unwrap();
    text.format(6, 5, r#"{"bold":true}"#.into(), None).unwrap();
    assert_eq!(excerpt.to_string(None).unwrap(), "world", "formatting left out");
    assert_eq!(values(&excerpt), "world".chars().map(|c| out(s(&c.to_string()))).collect::<Vec<_>>(), "its characters");
    assert_eq!(excerpt.deref(None).unwrap(), Some(out(s("w"))));

    // Inside it joins; at either edge doesn't.
    text.insert(8, "--", None, None).unwrap();
    text.insert(6, "<", None, None).unwrap();
    text.push(">", None, None).unwrap();
    assert_eq!(text.to_string(None).unwrap(), "hello <wo--rld>");
    assert_eq!(excerpt.to_string(None).unwrap(), "wo--rld");

    // On a replica after a v2 update; its edits inside reach both.
    let b = doc(2);
    sync(&a, &b, None);
    let replica = link_of(b.get_map("notes").unwrap().get_output("excerpt", None).unwrap());
    assert_eq!(replica.to_string(None).unwrap(), "wo--rld");
    let b_text = b.get_text("body").unwrap();
    b_text.delete(9, 2, None).unwrap();
    b_text.insert(9, "!", None, None).unwrap();
    sync(&b, &a, Some("server"));
    assert_eq!(excerpt.to_string(None).unwrap(), "wo!rld");
    assert_eq!(replica.to_string(None).unwrap(), "wo!rld");

    // Its text deleted, the quote is empty for good.
    text.delete(0, text.length(None).unwrap(), None).unwrap();
    assert_eq!(excerpt.to_string(None).unwrap(), "");
    assert_eq!(values(&excerpt), vec![]);
    assert_eq!(excerpt.deref(None).unwrap(), None);
    text.insert(0, "world", None, None).unwrap();
    assert_eq!(excerpt.to_string(None).unwrap(), "");
    sync(&a, &b, None);
    assert_eq!(replica.to_string(None).unwrap(), "");
}

#[test]
fn text_quotes_bounds_and_errors() {
    let doc = doc(1);
    let text = Arc::new(doc.get_text("body").unwrap());
    assert!(matches!(text.quote(0, 1, None), Err(Error::OutOfBounds)), "an empty text");
    text.insert(0, "a😀b", None, None).unwrap(); // the emoji is two UTF-16 units
    for (index, length) in [(0, 0), (1, 0), (4, 1), (2, 3), (0, 5), (1, u32::MAX)] {
        assert!(matches!(text.quote(index, length, None), Err(Error::OutOfBounds)), "{index}, {length}");
    }
    let notes = doc.get_map("notes").unwrap();
    assert_eq!(notes.set_weak_link("all", &text.quote(0, 4, None).unwrap(), None).unwrap().to_string(None).unwrap(), "a😀b");
    assert_eq!(notes.set_weak_link("emoji", &text.quote(1, 2, None).unwrap(), None).unwrap().to_string(None).unwrap(), "😀");
    assert!(matches!(YText::new_with_text(Some("x".into())).quote(0, 1, None), Err(Error::NotIntegrated)));

    // A quote embedded in its own text, before what it quotes.
    let inside = text.insert_embed_weak_link(0, &text.quote(3, 1, None).unwrap(), None, None).unwrap();
    assert_eq!(inside.to_string(None).unwrap(), "b");
    assert_eq!(text.to_string(None).unwrap(), "a😀b", "an embed has no text");

    // A link to a map entry or array values has no text.
    notes.set("k", s("v"), None).unwrap();
    assert_eq!(notes.set_weak_link("toK", &notes.link("k", None).unwrap().unwrap(), None).unwrap().to_string(None).unwrap(), "");
    let list = doc.get_array("list").unwrap();
    list.push(vec![s("text in an array")], None).unwrap();
    assert_eq!(notes.set_weak_link("toList", &list.quote(0, 1, None).unwrap(), None).unwrap().to_string(None).unwrap(), "");

    // Inside a callback, Locked.
    struct Inside(Arc<YText>, Mutex<Vec<bool>>);
    impl YTextObserver for Inside {
        fn on_change(&self, _event: YTextEvent) {
            self.1.lock().unwrap().push(matches!(self.0.quote(0, 1, None), Err(Error::Locked)));
        }
    }
    let heard = Arc::new(Inside(text.clone(), Mutex::new(vec![])));
    let _sub = text.observe(heard.clone()).unwrap();
    text.push("!", None, None).unwrap();
    assert_eq!(*heard.1.lock().unwrap(), vec![true]);
}
