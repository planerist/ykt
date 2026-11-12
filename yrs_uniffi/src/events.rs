//! Deep and XML observers, and the remaining document events.
//!
//! - `observeDeep` on a text, a map, an array or an XML node (Yjs's `observeDeep`): one
//!   `YDeepEvent` per committed transaction with an event for each changed type in it — the type
//!   itself and those nested in it (a map in a map, a map embedded in a text, XML children) — each
//!   with its target (a handle), its path from the observed type, and its change.
//! - `observe` on XML nodes (`YXmlFragment`, `YXmlElement`, `YXmlText`): a `YXmlEvent` with the
//!   children's delta, the text's delta (an XML text) and the attribute changes.
//! - `YDoc.observeAfterTransaction`, `observeSubdocs`, `observeDestroy`.
//!
//! - `observeDeep` on a weak link, and on a type holding one: `WeakLinkChange` when what it links
//!   to changed, with the linked types' own events.
//!
//! The rules of the `observers` module hold: callbacks run on the committing thread with the
//! document locked; unsubscribing is safe anywhere; a subscription keeps the document's store alive.

use crate::array::{array_delta, YArray, YArrayDelta};
use crate::collection::{Integrated, SharedCollection};
use crate::delta::YDelta;
use crate::doc::YDoc;
use crate::map::{key_changes, YEntryChange, YMap};
use crate::observers::{origin_bytes, subscription_key, text_delta, YSubscription};
use crate::text::YText;
use crate::weak::YWeakLink;
use crate::xml::YXmlChild;
use crate::xml_text::YXmlText;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::xml::{XmlEvent, XmlTextEvent};
use yrs::types::{Event, PathSegment};
use yrs::{DeepObservable, Doc, Observable, TransactionMut, XmlTextRef};

/// One step of a path from the observed type to a changed one: a key of a map, or an index of an
/// array, a text (an embedded type) or an XML node's children.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, Hash)]
pub enum YPathSegment {
    Key { key: String },
    Index { index: u32 },
}

/// What changed in one shared type, as a deep observer hears it: the type (`target`), where it is
/// from the observed type (`path`, empty for the observed type itself) and its change.
#[derive(uniffi::Enum, Clone, Debug)]
pub enum YEvent {
    TextChange { target: Arc<YText>, path: Vec<YPathSegment>, delta: Vec<YDelta> },
    MapChange { target: Arc<YMap>, path: Vec<YPathSegment>, keys: HashMap<String, YEntryChange> },
    ArrayChange { target: Arc<YArray>, path: Vec<YPathSegment>, delta: Vec<YArrayDelta> },
    /// An XML element or fragment: its children's change (inserted nodes as handles) and its
    /// attributes' (always empty for a fragment).
    XmlChange { target: YXmlChild, path: Vec<YPathSegment>, children: Vec<YArrayDelta>, attributes: HashMap<String, YEntryChange> },
    /// An XML text: its text's change and its XML attributes'.
    XmlTextChange { target: Arc<YXmlText>, path: Vec<YPathSegment>, delta: Vec<YDelta>, attributes: HashMap<String, YEntryChange> },
    /// A weak link: what it links to changed (read it after the callback).
    WeakLinkChange { target: Arc<YWeakLink>, path: Vec<YPathSegment> },
}

/// What a deep observer gets for a committed transaction that changed the observed type or any
/// type in it: an event per changed type, the observed type's own first (shallower paths first).
#[derive(uniffi::Record)]
pub struct YDeepEvent {
    pub events: Vec<YEvent>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YDeepObserver: Send + Sync {
    fn on_change(&self, event: YDeepEvent);
}

/// What an XML node's observer gets for a committed transaction that changed the node: for an
/// element or a fragment, its children's change (`children`); for an XML text, its text's change
/// (`delta`); for both, its attributes' changes with old and new values.
#[derive(uniffi::Record)]
pub struct YXmlEvent {
    pub target: YXmlChild,
    pub children: Vec<YArrayDelta>,
    pub delta: Vec<YDelta>,
    pub attributes: HashMap<String, YEntryChange>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YXmlObserver: Send + Sync {
    fn on_change(&self, event: YXmlEvent);
}

/// What an after-transaction observer gets for every committed transaction, after the types'
/// observers and before the cleanup (Yjs's `doc.on('afterTransaction')`).
#[derive(uniffi::Record)]
pub struct YAfterTransactionEvent {
    pub before_state: HashMap<u64, u32>,
    pub after_state: HashMap<u64, u32>,
    /// What the transaction deleted, encoded as a lib0 v1 delete set.
    pub delete_set: Vec<u8>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YAfterTransactionObserver: Send + Sync {
    fn on_after_transaction(&self, event: YAfterTransactionEvent);
}

/// Subdocuments a transaction added, removed or asked to load (Yjs's `doc.on('subdocs')`).
#[derive(uniffi::Record)]
pub struct YSubdocsEvent {
    pub added: Vec<Arc<YDoc>>,
    pub removed: Vec<Arc<YDoc>>,
    pub loaded: Vec<Arc<YDoc>>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YSubdocsObserver: Send + Sync {
    fn on_subdocs(&self, event: YSubdocsEvent);
}

/// The document being destroyed (`YDoc.destroy()`, Yjs's `doc.on('destroy')`).
#[derive(uniffi::Record)]
pub struct YDestroyEvent {
    pub guid: String,
}

#[uniffi::export(with_foreign)]
pub trait YDestroyObserver: Send + Sync {
    fn on_destroy(&self, event: YDestroyEvent);
}

pub(crate) fn path(path: yrs::types::Path) -> Vec<YPathSegment> {
    path.into_iter()
        .map(|segment| match segment {
            PathSegment::Key(key) => YPathSegment::Key { key: key.to_string() },
            PathSegment::Index(index) => YPathSegment::Index { index },
        })
        .collect()
}

/// A yrs event in ykt's form.
pub(crate) fn event(event: &Event, txn: &TransactionMut) -> YEvent {
    let doc = txn.doc();
    let at = path(event.path());
    match event {
        Event::Text(e) => YEvent::TextChange {
            target: Arc::new(YText::new(SharedCollection::integrated(e.target().clone(), doc.clone()))),
            path: at,
            delta: text_delta(e.delta(txn), txn),
        },
        Event::Map(e) => YEvent::MapChange {
            target: Arc::new(YMap(Integrated::new(e.target().clone(), doc.clone()))),
            path: at,
            keys: key_changes(e.keys(txn), txn),
        },
        Event::Array(e) => YEvent::ArrayChange {
            target: Arc::new(YArray(Integrated::new(e.target().clone(), doc.clone()))),
            path: at,
            delta: array_delta(e.delta(txn), txn),
        },
        Event::XmlFragment(e) => YEvent::XmlChange {
            target: YXmlChild::from_xml(e.target().clone(), doc.clone()),
            path: at,
            children: array_delta(e.delta(txn), txn),
            attributes: key_changes(e.keys(txn), txn),
        },
        Event::XmlText(e) => YEvent::XmlTextChange {
            target: Arc::new(YXmlText::from_ref(e.target().clone(), doc.clone())),
            path: at,
            delta: text_delta(e.delta(txn), txn),
            attributes: key_changes(e.keys(txn), txn),
        },
        Event::Weak(e) => YEvent::WeakLinkChange { target: Arc::new(YWeakLink::from_ref(e.as_target(), doc.clone())), path: at },
    }
}

/// Subscribes `observer` to `shared`'s deep changes. The subscription keeps `doc`'s store alive.
pub(crate) fn observe_deep<S>(shared: S, doc: Doc, observer: Arc<dyn YDeepObserver>) -> Arc<YSubscription>
where
    S: DeepObservable + Send + 'static,
{
    let key = subscription_key();
    let active = Arc::new(AtomicBool::new(true));
    let running = active.clone();
    shared.observe_deep(key.clone(), move |txn, events| {
        if running.load(Ordering::SeqCst) {
            let events = events.iter().map(|e| event(e, txn)).collect();
            observer.on_change(YDeepEvent { events, origin: origin_bytes(txn) });
        }
    });
    YSubscription::new(active, move || {
        let removed = shared.unobserve_deep(key);
        drop(doc);
        removed
    })
}

/// Subscribes `observer` to an XML element's or fragment's own changes.
pub(crate) fn observe_xml_node<S>(shared: S, doc: Doc, observer: Arc<dyn YXmlObserver>) -> Arc<YSubscription>
where
    S: Observable<Event = XmlEvent> + Send + 'static,
{
    let key = subscription_key();
    let active = Arc::new(AtomicBool::new(true));
    let running = active.clone();
    shared.observe(key.clone(), move |txn, e: &XmlEvent| {
        if running.load(Ordering::SeqCst) {
            observer.on_change(YXmlEvent {
                target: YXmlChild::from_xml(e.target().clone(), txn.doc().clone()),
                children: array_delta(e.delta(txn), txn),
                delta: vec![],
                attributes: key_changes(e.keys(txn), txn),
                origin: origin_bytes(txn),
            });
        }
    });
    YSubscription::new(active, move || {
        let removed = shared.unobserve(key);
        drop(doc);
        removed
    })
}

/// Subscribes `observer` to an XML text's own changes.
pub(crate) fn observe_xml_text(shared: XmlTextRef, doc: Doc, observer: Arc<dyn YXmlObserver>) -> Arc<YSubscription> {
    let key = subscription_key();
    let active = Arc::new(AtomicBool::new(true));
    let running = active.clone();
    shared.observe(key.clone(), move |txn, e: &XmlTextEvent| {
        if running.load(Ordering::SeqCst) {
            observer.on_change(YXmlEvent {
                target: YXmlChild::Text(Arc::new(YXmlText::from_ref(e.target().clone(), txn.doc().clone()))),
                children: vec![],
                delta: text_delta(e.delta(txn), txn),
                attributes: key_changes(e.keys(txn), txn),
                origin: origin_bytes(txn),
            });
        }
    });
    YSubscription::new(active, move || {
        let removed = shared.unobserve(key);
        drop(doc);
        removed
    })
}
