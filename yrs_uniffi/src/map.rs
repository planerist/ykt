//! `YMap`: a shared map of string keys to values (Yjs's `Y.Map`). Values are `YValue`s; a shared type
//! nested in a map (written by a client that has them, as Yjs) is read in its JSON form — nested
//! shared types aren't in ykt's API yet.

use crate::attrs::{out_into_yvalue, YAttributes, YValue};
use crate::collection::Integrated;
use crate::observers::{origin_bytes, subscription_key, ObserveResult, YSubscription};
use crate::tools::Result;
use crate::transaction::YTransaction;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::{EntryChange, ToJson};
use yrs::{Any, Map, MapRef, Observable, TransactionMut};

/// A shared map (see the module docs). Reads take a read-only transaction when none is given;
/// writes, a transaction of their own.
#[derive(uniffi::Object)]
pub struct YMap(pub(crate) Integrated<MapRef>);

// SAFETY: as YText: the embedding application calls into a document one call at a time.
unsafe impl Send for YMap {}
unsafe impl Sync for YMap {}

/// What happened to one key of a map in a transaction.
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum YEntryChange {
    /// The key is new, with this value.
    Inserted { value: YValue },
    /// The key had `old_value`, now `new_value`.
    Updated { old_value: YValue, new_value: YValue },
    /// The key, with `old_value`, was deleted.
    Removed { old_value: YValue },
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

    /// The value of `key`; `null` if the map hasn't got it.
    #[uniffi::method(default(txn = None))]
    pub fn get(&self, key: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YValue>> {
        self.0.read(txn, |map, txn| Ok(map.get(txn, key).map(|v| out_into_yvalue(&v, txn))))
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

fn key_changes(keys: &HashMap<Arc<str>, EntryChange>, txn: &TransactionMut) -> HashMap<String, YEntryChange> {
    keys.iter()
        .map(|(key, change)| {
            let change = match change {
                EntryChange::Inserted(value) => YEntryChange::Inserted { value: out_into_yvalue(value, txn) },
                EntryChange::Updated(old, new) => YEntryChange::Updated {
                    old_value: out_into_yvalue(old, txn),
                    new_value: out_into_yvalue(new, txn),
                },
                EntryChange::Removed(old) => YEntryChange::Removed { old_value: out_into_yvalue(old, txn) },
            };
            (key.to_string(), change)
        })
        .collect()
}
