//! `YMap`: a shared map of string keys to values (Yjs's `Y.Map`). Values are plain (`YValue`) or
//! shared types nested in the map — a text, a map, an array, a subdocument — made with `setText`,
//! `setMap`, `setArray`, `setDoc` and read back as handles with `getOutput`; `get`, `values` and
//! `toJson` give a nested type in its JSON form. `link` and `setWeakLink`: weak links (`weak` module).

use crate::array::YArray;
use crate::attrs::{out_into_yvalue, YAttributes, YValue};
use crate::collection::{Integrated, SharedCollection, YBranchId};
use crate::doc::YDoc;
use crate::observers::{origin_bytes, subscription_key, ObserveResult, YSubscription};
use crate::output::YOutput;
use crate::text::YText;
use crate::weak::YWeakLink;
use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::{EntryChange, ToJson};
use yrs::{Any, ArrayPrelim, Map, MapPrelim, MapRef, Observable, ReadTxn, TextPrelim, Transact, TransactionMut};

/// A shared map (see the module docs). Reads take a read-only transaction when none is given;
/// writes, a transaction of their own. Two handles of the same map compare equal.
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
pub struct YMap(pub(crate) Integrated<MapRef>);

// SAFETY: as YText: the embedding application calls into a document one call at a time.
unsafe impl Send for YMap {}
unsafe impl Sync for YMap {}

/// What happened to one key of a map in a transaction. A shared type comes as a handle.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
#[uniffi::export(Eq)]
pub enum YEntryChange {
    /// The key is new, with this value.
    Inserted { value: YOutput },
    /// The key had `old_value`, now `new_value`.
    Updated { old_value: YOutput, new_value: YOutput },
    /// The key, with `old_value`, was deleted.
    Removed { old_value: YOutput },
}

/// What a map's observer gets for a committed transaction that changed the map.
#[derive(uniffi::Record)]
pub struct YMapEvent {
    /// The changed keys (Yjs's `event.changes.keys`).
    pub keys: HashMap<String, YEntryChange>,
    /// The origin the transaction was opened with, as bytes; none for a transaction without one.
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YMapObserver: Send + Sync {
    fn on_change(&self, event: YMapEvent);
}

#[uniffi::export]
impl YMap {
    /// Sets `key` to `value`, replacing what it had (Yjs's `map.set`).
    #[uniffi::method(default(txn = None))]
    pub fn set(&self, key: &str, value: YValue, txn: Option<Arc<YTransaction>>) -> Result<()> {
        self.0.mutably(txn, |map, txn| {
            let value: Any = value.into();
            map.insert(txn, key, value);
            Ok(())
        })
    }

    /// Sets `key` to a new text with `text` in it (Yjs's `map.set(key, new Y.Text(text))`), and
    /// returns it.
    #[uniffi::method(default(text = None, txn = None))]
    pub fn set_text(&self, key: &str, text: Option<String>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YText>> {
        self.0.mutably(txn, |map, txn| {
            let nested = map.insert(txn, key, TextPrelim::new(text.unwrap_or_default()));
            Ok(Arc::new(YText::new(SharedCollection::integrated(nested, self.0.doc.clone()))))
        })
    }

    /// Sets `key` to a new map with `entries` in it, and returns it.
    #[uniffi::method(default(entries = None, txn = None))]
    pub fn set_map(&self, key: &str, entries: Option<YAttributes>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YMap>> {
        self.0.mutably(txn, |map, txn| {
            let nested = map.insert(txn, key, map_prelim(entries));
            Ok(Arc::new(YMap(Integrated::new(nested, self.0.doc.clone()))))
        })
    }

    /// Sets `key` to a new array with `values` in it, and returns it.
    #[uniffi::method(default(values = None, txn = None))]
    pub fn set_array(&self, key: &str, values: Option<Vec<YValue>>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YArray>> {
        self.0.mutably(txn, |map, txn| {
            let nested = map.insert(txn, key, array_prelim(values));
            Ok(Arc::new(YArray(Integrated::new(nested, self.0.doc.clone()))))
        })
    }

    /// Sets `key` to `doc` as a subdocument (Yjs's subdocuments), and returns its handle in this
    /// document. `doc` must be a new document, not a subdocument already (`AlreadyIntegrated`); a
    /// subdocument can't hold subdocuments of its own in yrs (`WrongDocument`).
    #[uniffi::method(default(txn = None))]
    pub fn set_doc(&self, key: &str, doc: &YDoc, txn: Option<Arc<YTransaction>>) -> Result<Arc<YDoc>> {
        let subdoc = subdoc_prelim(doc, &self.0.doc)?;
        self.0.mutably(txn, |map, txn| {
            check_subdoc_parent(txn)?;
            Ok(Arc::new(YDoc(map.insert(txn, key, subdoc))))
        })
    }

    /// Sets `key` to a weak link (see the `weak` module): `link`'s quote — a preliminary link, or a
    /// copy of one in the document — and returns the new link. `WrongDocument` for another
    /// document's link, `TypeDeleted` for a deleted one.
    #[uniffi::method(default(txn = None))]
    pub fn set_weak_link(&self, key: &str, link: &YWeakLink, txn: Option<Arc<YTransaction>>) -> Result<Arc<YWeakLink>> {
        self.0.mutably(txn, |map, txn| {
            let quote = link.quote_for(txn)?;
            Ok(Arc::new(YWeakLink::from_ref(map.insert(txn, key, quote), self.0.doc.clone())))
        })
    }

    /// A preliminary link to `key`'s entry (yrs's `Map::link`), to insert with `setWeakLink` & co.:
    /// it follows the entry, `deref()` giving its value as it is set again; `null` if the map
    /// hasn't got `key`.
    #[uniffi::method(default(txn = None))]
    pub fn link(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<Arc<YWeakLink>>> {
        self.0.read(txn, |map, txn| {
            if !map.contains_key(txn, key) {
                return Ok(None);
            }
            Ok(map.link(txn, key).map(|quote| Arc::new(YWeakLink::from_quote(quote, self.0.doc.clone()))))
        })
    }

    /// The value of `key`; `null` if the map hasn't got it. A shared type comes in its JSON form
    /// (`getOutput` gives its handle).
    #[uniffi::method(default(txn = None))]
    pub fn get(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YValue>> {
        self.0.read(txn, |map, txn| Ok(map.get(txn, key).map(|v| out_into_yvalue(&v, txn))))
    }

    /// The value of `key` as a plain value or a shared type's handle; `null` if the map hasn't got
    /// it.
    #[uniffi::method(default(txn = None))]
    pub fn get_output(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YOutput>> {
        self.0.read(txn, |map, txn| Ok(map.get(txn, key).map(|v| YOutput::from_out(v, &self.0.doc, txn))))
    }

    /// Deletes `key`. Returns the value it had, `null` if the map hadn't got it.
    #[uniffi::method(default(txn = None))]
    pub fn delete(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YValue>> {
        self.0.mutably(txn, |map, txn| Ok(map.remove(txn, key).map(|v| out_into_yvalue(&v, &*txn))))
    }

    #[uniffi::method(default(txn = None))]
    pub fn contains_key(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<bool> {
        self.0.read(txn, |map, txn| Ok(map.contains_key(txn, key)))
    }

    /// The keys, in no particular order (the same as `values()`'s while the map doesn't change).
    #[uniffi::method(default(txn = None))]
    pub fn keys(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<String>> {
        self.0.read(txn, |map, txn| Ok(map.iter(txn).map(|(k, _)| k.to_string()).collect()))
    }

    /// The values, in the order of `keys()`.
    #[uniffi::method(default(txn = None))]
    pub fn values(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<YValue>> {
        self.0.read(txn, |map, txn| Ok(map.iter(txn).map(|(_, v)| out_into_yvalue(&v, txn)).collect()))
    }

    /// The number of keys.
    #[uniffi::method(default(txn = None))]
    pub fn len(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        self.0.read(txn, |map, txn| Ok(map.len(txn)))
    }

    /// Deletes every key.
    #[uniffi::method(default(txn = None))]
    pub fn clear(&self, txn: Option<Arc<YTransaction>>) -> Result<()> {
        self.0.mutably(txn, |map, txn| {
            map.clear(txn);
            Ok(())
        })
    }

    /// The whole map as plain values (Yjs's `toJSON()`).
    #[uniffi::method(default(txn = None))]
    pub fn to_json(&self, txn: Option<Arc<YTransaction>>) -> Result<YAttributes> {
        self.0.read(txn, |map, txn| match map.to_json(txn) {
            Any::Map(entries) => Ok(entries.iter().map(|(k, v)| (k.clone(), crate::attrs::into_yvalue(v))).collect()),
            _ => Ok(HashMap::new()),
        })
    }

    /// The map's identity in its document, the same on every replica (see `YText.id`).
    pub fn id(&self) -> YBranchId {
        self.0.id().into()
    }

    /// Calls `observer` after every committed transaction that changed this map or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> ObserveResult {
        let shared: MapRef = self.0.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_deep(shared, self.0.doc.clone(), observer))
    }

    /// Calls `observer` after every committed transaction that changed this map, with the changed
    /// keys and the transaction's origin (Yjs's `ymap.observe`). The callback runs on the
    /// committing thread while the document is still locked: it must not read or write the
    /// document (see the `observers` module). The subscription stops on `unsubscribe()` or when it
    /// is released.
    pub fn observe(&self, observer: Arc<dyn YMapObserver>) -> ObserveResult {
        let map: MapRef = self.0.read(None, |map, _| Ok(map.clone()))?;
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        map.observe(key.clone(), move |txn, event| {
            if running.load(Ordering::SeqCst) {
                observer.on_change(YMapEvent { keys: key_changes(event.keys(txn), txn), origin: origin_bytes(txn) });
            }
        });
        // The handle keeps the document alive: `map` points into its store.
        let doc = self.0.doc.clone();
        Ok(YSubscription::new(active, move || {
            let removed = map.unobserve(key);
            drop(doc);
            removed
        }))
    }
}

pub(crate) fn key_changes(keys: &HashMap<Arc<str>, EntryChange>, txn: &TransactionMut) -> HashMap<String, YEntryChange> {
    let out = |value: &yrs::Out| YOutput::from_out(value.clone(), txn.doc(), txn);
    keys.iter()
        .map(|(key, change)| {
            let change = match change {
                EntryChange::Inserted(value) => YEntryChange::Inserted { value: out(value) },
                EntryChange::Updated(old, new) => YEntryChange::Updated { old_value: out(old), new_value: out(new) },
                EntryChange::Removed(old) => YEntryChange::Removed { old_value: out(old) },
            };
            (key.to_string(), change)
        })
        .collect()
}

pub(crate) fn map_prelim(entries: Option<YAttributes>) -> MapPrelim {
    entries.unwrap_or_default().into_iter().map(|(k, v)| (k, Into::<Any>::into(v))).collect()
}

pub(crate) fn array_prelim(values: Option<Vec<YValue>>) -> ArrayPrelim {
    values.unwrap_or_default().into_iter().map(Into::<Any>::into).collect()
}

/// `doc`, to insert into `parent` as a subdocument: a new one (`AlreadyIntegrated` for a
/// subdocument, or the parent itself). yrs's own check would wait for `doc`'s lock: this one throws.
pub(crate) fn subdoc_prelim(doc: &YDoc, parent: &yrs::Doc) -> Result<yrs::Doc> {
    if crate::collection::same_doc(&doc.0, parent) {
        return Err(Error::AlreadyIntegrated);
    }
    let is_subdoc = doc.0.try_transact().map_err(|_| Error::Locked)?.parent_doc().is_some();
    if is_subdoc {
        return Err(Error::AlreadyIntegrated);
    }
    Ok(doc.0.clone())
}

/// `WrongDocument` if the transaction's document is a subdocument: yrs can't nest them (it panics).
pub(crate) fn check_subdoc_parent(txn: &TransactionMut) -> Result<()> {
    if yrs::ReadTxn::parent_doc(txn).is_some() {
        Err(Error::WrongDocument)
    } else {
        Ok(())
    }
}
