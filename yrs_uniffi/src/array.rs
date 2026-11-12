//! `YArray`: a shared list of values (Yjs's `Y.Array`). Values are `YValue`s; a shared type nested in
//! an array (written by a client that has them, as Yjs) is read in its JSON form — nested shared
//! types aren't in ykt's API yet.

use crate::attrs::{into_yvalue, out_into_yvalue, YValue};
use crate::collection::Integrated;
use crate::observers::{origin_bytes, subscription_key, ObserveResult, YSubscription};
use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::{Change, ToJson};
use yrs::{Any, Array, ArrayRef, Observable, TransactionMut};

/// A shared array (see the module docs). Indexes count values. Reads take a read-only transaction
/// when none is given; writes, a transaction of their own.
#[derive(uniffi::Object)]
pub struct YArray(pub(crate) Integrated<ArrayRef>);

// SAFETY: as YText: the embedding application calls into a document one call at a time.
unsafe impl Send for YArray {}
unsafe impl Sync for YArray {}

/// One step of an array's change, as Yjs's `event.delta`.
#[derive(uniffi::Enum, Clone, Debug, PartialEq)]
pub enum YArrayDelta {
    /// These values were inserted here.
    Insert(Vec<YValue>),
    /// This many values were deleted here.
    Delete(u32),
    /// This many values are unchanged.
    Retain(u32),
}

/// What an array's observer gets for a committed transaction that changed the array.
#[derive(uniffi::Record)]
pub struct YArrayEvent {
    pub delta: Vec<YArrayDelta>,
    /// The origin the transaction was opened with, as bytes; none for a transaction without one.
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YArrayObserver: Send + Sync {
    fn on_change(&self, event: YArrayEvent);
}

#[uniffi::export]
impl YArray {
    /// Inserts `values` at `index` (0 ..= `len()`, else `OutOfBounds`).
    #[uniffi::method(default(txn = None))]
    pub fn insert(&self, index: u32, values: Vec<YValue>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        self.0.mutably(txn, |array, txn| {
            if index > array.len(txn) {
                return Err(Error::OutOfBounds);
            }
            array.insert_range(txn, index, values);
            Ok(())
        })
    }

    /// Appends `values` at the end.
    #[uniffi::method(default(txn = None))]
    pub fn push(&self, values: Vec<YValue>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        self.0.mutably(txn, |array, txn| {
            let len = array.len(txn);
            array.insert_range(txn, len, values);
            Ok(())
        })
    }

    /// Deletes `length` values from `index` on (`OutOfBounds` if the array hasn't got them all).
    #[uniffi::method(default(txn = None))]
    pub fn delete(&self, index: u32, length: u32, txn: Option<Arc<YTransaction>>) -> Result<()> {
        self.0.mutably(txn, |array, txn| {
            match index.checked_add(length) {
                Some(end) if end <= array.len(txn) => {}
                _ => return Err(Error::OutOfBounds),
            }
            if length > 0 {
                array.remove_range(txn, index, length);
            }
            Ok(())
        })
    }

    /// The value at `index` (`OutOfBounds` past the end).
    #[uniffi::method(default(txn = None))]
    pub fn get(&self, index: u32, txn: Option<Arc<YTransaction>>) -> Result<YValue> {
        self.0.read(txn, |array, txn| {
            array.get(txn, index).map(|v| out_into_yvalue(&v, txn)).ok_or(Error::OutOfBounds)
        })
    }

    /// The number of values.
    #[uniffi::method(default(txn = None))]
    pub fn len(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        self.0.read(txn, |array, txn| Ok(array.len(txn)))
    }

    /// The whole array as plain values (Yjs's `toJSON()` / `toArray()`).
    #[uniffi::method(default(txn = None))]
    pub fn to_json(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<YValue>> {
        self.0.read(txn, |array, txn| match array.to_json(txn) {
            Any::Array(values) => Ok(values.iter().map(into_yvalue).collect()),
            _ => Ok(vec![]),
        })
    }

    /// Calls `observer` after every committed transaction that changed this array, with the change
    /// as a delta and the transaction's origin (Yjs's `yarray.observe`). The callback runs on the
    /// committing thread while the document is still locked: it must not read or write the
    /// document (see the `observers` module). The subscription stops on `unsubscribe()` or when it
    /// is released.
    pub fn observe(&self, observer: Arc<dyn YArrayObserver>) -> ObserveResult {
        let array: ArrayRef = self.0.read(None, |array, _| Ok(array.clone()))?;
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        array.observe(key.clone(), move |txn, event| {
            if running.load(Ordering::SeqCst) {
                observer.on_change(YArrayEvent { delta: array_delta(event.delta(txn), txn), origin: origin_bytes(txn) });
            }
        });
        // The handle keeps the document alive: `array` points into its store.
        let doc = self.0.doc.clone();
        Ok(YSubscription::new(active, move || {
            let removed = array.unobserve(key);
            drop(doc);
            removed
        }))
    }
}

fn array_delta(delta: &[Change], txn: &TransactionMut) -> Vec<YArrayDelta> {
    delta
        .iter()
        .map(|change| match change {
            Change::Added(values) => YArrayDelta::Insert(values.iter().map(|v| out_into_yvalue(v, txn)).collect()),
            Change::Removed(len) => YArrayDelta::Delete(*len),
            Change::Retain(len) => YArrayDelta::Retain(*len),
        })
        .collect()
}
