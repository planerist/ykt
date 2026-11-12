//! `YArray`: a shared list of values (Yjs's `Y.Array`). Values are plain (`YValue`) or shared types
//! nested in the array — made with `insertText`, `insertMap`, `insertArray`, `insertDoc` and read
//! back as handles with `getOutput`; `get` and `toJson` give a nested type in its JSON form. `quote`
//! and `insertWeakLink`: weak links (`weak` module).

use crate::attrs::{into_yvalue, out_into_yvalue, YAttributes, YValue};
use crate::collection::{Integrated, SharedCollection, YBranchId};
use crate::doc::YDoc;
use crate::map::{array_prelim, map_prelim, subdoc_prelim, YMap};
use crate::observers::{origin_bytes, subscription_key, ObserveResult, YSubscription};
use crate::output::YOutput;
use crate::text::YText;
use crate::weak::{quote_range, YWeakLink};
use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::{Change, ToJson};
use yrs::{Any, Array, ArrayRef, Observable, TextPrelim, TransactionMut};

/// A shared array (see the module docs). Indexes count values. Reads take a read-only transaction
/// when none is given; writes, a transaction of their own. Two handles of the same array compare
/// equal.
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
pub struct YArray(pub(crate) Integrated<ArrayRef>);

// SAFETY: as YText: the embedding application calls into a document one call at a time.
unsafe impl Send for YArray {}
unsafe impl Sync for YArray {}

/// One step of an array's change, as Yjs's `event.delta`. Inserted shared types come as handles.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
#[uniffi::export(Eq)]
pub enum YArrayDelta {
    /// These values were inserted here.
    Insert(Vec<YOutput>),
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

    /// Inserts a new text with `text` in it at `index` (0 ..= `len()`, else `OutOfBounds`), and
    /// returns it.
    #[uniffi::method(default(text = None, txn = None))]
    pub fn insert_text(&self, index: u32, text: Option<String>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YText>> {
        self.0.mutably(txn, |array, txn| {
            check_index(index, array.len(txn))?;
            let nested = array.insert(txn, index, TextPrelim::new(text.unwrap_or_default()));
            Ok(Arc::new(YText::new(SharedCollection::integrated(nested, self.0.doc.clone()))))
        })
    }

    /// Inserts a new map with `entries` in it at `index`, and returns it.
    #[uniffi::method(default(entries = None, txn = None))]
    pub fn insert_map(&self, index: u32, entries: Option<YAttributes>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YMap>> {
        self.0.mutably(txn, |array, txn| {
            check_index(index, array.len(txn))?;
            let nested = array.insert(txn, index, map_prelim(entries));
            Ok(Arc::new(YMap(Integrated::new(nested, self.0.doc.clone()))))
        })
    }

    /// Inserts a new array with `values` in it at `index`, and returns it.
    #[uniffi::method(default(values = None, txn = None))]
    pub fn insert_array(&self, index: u32, values: Option<Vec<YValue>>, txn: Option<Arc<YTransaction>>) -> Result<Arc<YArray>> {
        self.0.mutably(txn, |array, txn| {
            check_index(index, array.len(txn))?;
            let nested = array.insert(txn, index, array_prelim(values));
            Ok(Arc::new(YArray(Integrated::new(nested, self.0.doc.clone()))))
        })
    }

    /// Inserts `doc` as a subdocument at `index` (as `YMap.setDoc`), and returns its handle.
    #[uniffi::method(default(txn = None))]
    pub fn insert_doc(&self, index: u32, doc: &YDoc, txn: Option<Arc<YTransaction>>) -> Result<Arc<YDoc>> {
        let subdoc = subdoc_prelim(doc, &self.0.doc)?;
        self.0.mutably(txn, |array, txn| {
            check_index(index, array.len(txn))?;
            crate::map::check_subdoc_parent(txn)?;
            Ok(Arc::new(YDoc(array.insert(txn, index, subdoc))))
        })
    }

    /// Inserts a weak link at `index` (0 ..= `len()`, else `OutOfBounds`), as `YMap.setWeakLink`,
    /// and returns it.
    #[uniffi::method(default(txn = None))]
    pub fn insert_weak_link(&self, index: u32, link: &YWeakLink, txn: Option<Arc<YTransaction>>) -> Result<Arc<YWeakLink>> {
        self.0.mutably(txn, |array, txn| {
            check_index(index, array.len(txn))?;
            let quote = link.quote_for(txn)?;
            Ok(Arc::new(YWeakLink::from_ref(array.insert(txn, index, quote), self.0.doc.clone())))
        })
    }

    /// A quote of `length` values from `index` on (yrs's `quote`): a preliminary weak link to insert
    /// with `YMap.setWeakLink` & co. Both ends are in it: values inserted inside the range join it,
    /// at its edges don't. `OutOfBounds` unless it holds at least one value and lies in the array.
    #[uniffi::method(default(txn = None))]
    pub fn quote(&self, index: u32, length: u32, txn: Option<Arc<YTransaction>>) -> Result<Arc<YWeakLink>> {
        self.0.read(txn, |array, txn| {
            let quote = quote_range(array, txn, index, length, array.len(txn))?;
            Ok(Arc::new(YWeakLink::from_quote(quote, self.0.doc.clone())))
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

    /// The value at `index` (`OutOfBounds` past the end); a shared type in its JSON form
    /// (`getOutput` gives its handle).
    #[uniffi::method(default(txn = None))]
    pub fn get(&self, index: u32, txn: Option<Arc<YTransaction>>) -> Result<YValue> {
        self.0.read(txn, |array, txn| {
            array.get(txn, index).map(|v| out_into_yvalue(&v, txn)).ok_or(Error::OutOfBounds)
        })
    }

    /// The value at `index` as a plain value or a shared type's handle (`OutOfBounds` past the end).
    #[uniffi::method(default(txn = None))]
    pub fn get_output(&self, index: u32, txn: Option<Arc<YTransaction>>) -> Result<YOutput> {
        self.0.read(txn, |array, txn| {
            array.get(txn, index).map(|v| YOutput::from_out(v, &self.0.doc, txn)).ok_or(Error::OutOfBounds)
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

    /// The array's identity in its document, the same on every replica (see `YText.id`).
    pub fn id(&self) -> YBranchId {
        self.0.id().into()
    }

    /// Calls `observer` after every committed transaction that changed this array or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> ObserveResult {
        let shared: ArrayRef = self.0.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_deep(shared, self.0.doc.clone(), observer))
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

fn check_index(index: u32, len: u32) -> Result<()> {
    if index > len {
        Err(Error::OutOfBounds)
    } else {
        Ok(())
    }
}

pub(crate) fn array_delta(delta: &[Change], txn: &TransactionMut) -> Vec<YArrayDelta> {
    delta
        .iter()
        .map(|change| match change {
            Change::Added(values) => {
                YArrayDelta::Insert(values.iter().map(|v| YOutput::from_out(v.clone(), txn.doc(), txn)).collect())
            }
            Change::Removed(len) => YArrayDelta::Delete(*len),
            Change::Retain(len) => YArrayDelta::Retain(*len),
        })
        .collect()
}
