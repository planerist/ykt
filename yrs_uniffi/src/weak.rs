//! `YWeakLink`: a link to a map entry or a quote of a range of an array or a text (yrs's weak
//! links). It follows what it links to: a map entry's value as it changes, the values or the text
//! in the range, what any replica inserts inside it included.
//!
//! - Making one: `YMap.link(key)`, `YArray.quote(index, length)`, `YText.quote(index, length)`
//!   give a preliminary link (a quote of the document, not in it yet).
//! - Inserting it: `YMap.setWeakLink`, `YArray.insertWeakLink`, `YText.insertEmbedWeakLink` put it
//!   into the document and give its integrated handle. A preliminary link can be inserted any number
//!   of times; an integrated one is copied (the new link quotes the same).
//! - Reading it: `getOutput` (`YOutput.WeakLink`), then `unquote()` (the quoted values), `toText()`
//!   (a text quote's text) and `deref()` (a map entry's value). `get` / `toJson` give a link as `null`, its JSON form in yrs.
//! - Observing it: `observe` hears what changed in the quoted range or the linked entry; `observeDeep`
//!   also hears the types inside it.

use crate::collection::{integrated_of, is_alive, is_prelim, same_doc, Integrated, SharedCell, SharedCollection, YBranchId};
use crate::events::YDeepObserver;
use crate::observers::{origin_bytes, subscription_key, ObserveResult, YSubscription};
use crate::output::YOutput;
use crate::tools::{check_range, Error, Result};
use crate::transaction::YTransaction;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::branch::BranchPtr;
use yrs::types::weak::WeakEvent;
use yrs::types::TypeRef;
use yrs::{ArrayRef, Assoc, Doc, GetString, MapRef, Observable, Out, Quotable, ReadTxn, StickyIndex, TextRef, TransactionMut, WeakPrelim, WeakRef, XmlTextRef};

/// A quote not in its document yet, with the document it quotes.
#[derive(Clone)]
pub(crate) struct PrelimLink {
    pub quote: WeakPrelim<BranchPtr>,
    pub doc: Doc,
}

/// A weak link (see the module docs): preliminary, from `YMap.link` / `YArray.quote` /
/// `YText.quote`, or in the document, from inserting one or from `getOutput`. Two handles of the
/// same link compare equal; a preliminary one is equal only to itself.
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
#[repr(transparent)]
pub struct YWeakLink(pub(crate) Arc<SharedCell<PrelimLink, WeakRef<BranchPtr>>>);

// SAFETY: as YText: the embedding application calls into a document one call at a time.
unsafe impl Sync for YWeakLink {}
unsafe impl Send for YWeakLink {}

/// What a weak link's observer gets for a committed transaction that changed what it links to:
/// the link (to read once the callback returns) and the transaction's origin.
#[derive(uniffi::Record)]
pub struct YWeakLinkEvent {
    pub target: Arc<YWeakLink>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YWeakLinkObserver: Send + Sync {
    fn on_change(&self, event: YWeakLinkEvent);
}

impl YWeakLink {
    pub(crate) fn from_quote<P>(quote: WeakPrelim<P>, doc: Doc) -> Self {
        YWeakLink(Arc::new(RefCell::new(SharedCollection::prelim(PrelimLink { quote: quote.into_inner(), doc }))))
    }

    pub(crate) fn from_ref(link: WeakRef<BranchPtr>, doc: Doc) -> Self {
        YWeakLink(Arc::new(RefCell::new(SharedCollection::integrated(link, doc))))
    }

    /// The quote to insert in `txn`: a preliminary link's, or a copy of an integrated one's.
    /// `WrongDocument` for another document's link, `TypeDeleted` for a deleted one.
    pub(crate) fn quote_for(&self, txn: &TransactionMut) -> Result<WeakPrelim<BranchPtr>> {
        match &*self.0.try_borrow().map_err(|_| Error::Locked)? {
            SharedCollection::Prelim(p) if same_doc(&p.doc, txn.doc()) => Ok(p.quote.clone()),
            SharedCollection::Prelim(_) => Err(Error::WrongDocument),
            SharedCollection::Integrated(c) => {
                c.check_doc(txn.doc())?;
                Ok(WeakPrelim::from(c.resolve(txn)?))
            }
        }
    }

    fn integrated(&self) -> Result<Integrated<WeakRef<BranchPtr>>> {
        integrated_of(&self.0)
    }
}

/// A quote of `length` values (units of a text) from `index` on, both ends in it: what is inserted
/// inside it joins it, at its edges doesn't. `OutOfBounds` unless it holds at least one and lies
/// within the `len` of `shared`.
pub(crate) fn quote_range<Q: Quotable, T: ReadTxn>(shared: &Q, txn: &T, index: u32, length: u32, len: u32) -> Result<WeakPrelim<Q>> {
    if length == 0 {
        return Err(Error::OutOfBounds);
    }
    check_range(index, length, len)?;
    shared.quote(txn, index..=index + length - 1).map_err(|_| Error::OutOfBounds)
}

/// What a link quotes: a map entry, or a range of a sequence (an array, a text, an XML text).
#[derive(PartialEq)]
pub(crate) enum Quoted {
    MapEntry,
    XmlText,
    Sequence,
}

/// The kind of type `link` quotes from, found through its first quoted item.
pub(crate) fn quoted<T: ReadTxn>(link: &WeakRef<BranchPtr>, txn: &T) -> Quoted {
    let parent = link.start_id().and_then(|id| StickyIndex::from_id(*id, Assoc::Before).get_offset(txn)).map(|offset| offset.branch);
    match parent.as_ref().map(|branch| branch.type_ref()) {
        Some(TypeRef::Map) => Quoted::MapEntry,
        Some(TypeRef::XmlText) => Quoted::XmlText,
        _ => Quoted::Sequence,
    }
}

/// The linked map entry's value, or a quote's first value; `None` when it is gone.
fn deref<T: ReadTxn>(link: &WeakRef<BranchPtr>, txn: &T) -> Option<Out> {
    let sequence: WeakRef<ArrayRef> = WeakRef::from(link.clone());
    match sequence.unquote(txn).next() {
        Some(first) => Some(first),
        // A map entry isn't in its map's sequence of items: yrs's unquote gives nothing for it.
        None if quoted(link, txn) == Quoted::MapEntry => WeakRef::<MapRef>::from(link.clone()).try_deref_value(txn),
        None => None,
    }
}

#[uniffi::export]
impl YWeakLink {
    /// Whether this is a preliminary link (a quote, not in the document yet).
    pub fn prelim(&self) -> bool {
        is_prelim(&self.0)
    }

    /// Whether the link is in its document, read in `txn`: `false` for a preliminary link or a
    /// deleted one.
    pub fn alive(&self, txn: &YTransaction) -> Result<bool> {
        is_alive(&self.0, txn)
    }

    /// The link's identity in its document, the same on every replica (see `YText.id`).
    /// `NotIntegrated` for a preliminary link.
    pub fn id(&self) -> Result<YBranchId> {
        Ok(self.integrated()?.id().into())
    }

    /// The values it quotes, in order, as they are now: an array range's values (inserted inside it
    /// included, deleted ones gone), a text range's characters (`toText` gives the string), a map
    /// entry's value (none once it is deleted). `NotIntegrated` for a preliminary link.
    #[uniffi::method(default(txn = None))]
    pub fn unquote(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<YOutput>> {
        let c = self.integrated()?;
        c.read(txn, |link, view| {
            let sequence: WeakRef<ArrayRef> = WeakRef::from(link.clone());
            let mut values: Vec<Out> = sequence.unquote(view).collect();
            if values.is_empty() && quoted(link, view) == Quoted::MapEntry {
                values.extend(deref(link, view));
            }
            Ok(values.into_iter().map(|value| YOutput::from_out(value, &c.doc, view)).collect())
        })
    }

    /// The quoted text as it is now, as its type's `toText` gives it — a text's string (formatting
    /// left out), an XML text's as XML — what was inserted inside the range included; empty once it
    /// is deleted, and for a link to a map entry or to array values. `NotIntegrated` for a
    /// preliminary link.
    #[uniffi::method(name = "toText", default(txn = None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        let c = self.integrated()?;
        c.read(txn, |link, view| {
            Ok(match quoted(link, view) {
                Quoted::MapEntry => String::new(),
                Quoted::XmlText => WeakRef::<XmlTextRef>::from(link.clone()).get_string(view),
                Quoted::Sequence => WeakRef::<TextRef>::from(link.clone()).get_string(view),
            })
        })
    }

    /// The linked map entry's value now (as its map's `getOutput` gives it), or a quote's first
    /// value; `null` once it is deleted. `NotIntegrated` for a preliminary link.
    #[uniffi::method(default(txn = None))]
    pub fn deref(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YOutput>> {
        let c = self.integrated()?;
        c.read(txn, |link, view| Ok(deref(link, view).map(|value| YOutput::from_out(value, &c.doc, view))))
    }

    /// Calls `observer` after every committed transaction that changed what the link quotes — a
    /// value inserted, deleted or replaced in its range, its map entry set or deleted — with the
    /// transaction's origin. The rules of `YText.observe` hold: read the link after the callback.
    /// `NotIntegrated` for a preliminary link.
    pub fn observe(&self, observer: Arc<dyn YWeakLinkObserver>) -> ObserveResult {
        let c = self.integrated()?;
        let link: WeakRef<BranchPtr> = c.read(None, |link, _| Ok(link.clone()))?;
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        link.observe(key.clone(), move |txn, event: &WeakEvent| {
            if running.load(Ordering::SeqCst) {
                observer.on_change(YWeakLinkEvent {
                    target: Arc::new(YWeakLink::from_ref(event.as_target(), txn.doc().clone())),
                    origin: origin_bytes(txn),
                });
            }
        });
        // The handle keeps the document alive: `link` points into its store.
        let doc = c.doc.clone();
        Ok(YSubscription::new(active, move || {
            let removed = link.unobserve(key);
            drop(doc);
            removed
        }))
    }

    /// As `observe`, with the changes of the shared types it links to as well — a linked map's
    /// keys, a quoted text's… — an event per changed type (Yjs's `observeDeep`): `WeakLinkChange`
    /// for the link itself. `NotIntegrated` for a preliminary link.
    pub fn observe_deep(&self, observer: Arc<dyn YDeepObserver>) -> ObserveResult {
        let c = self.integrated()?;
        let link: WeakRef<BranchPtr> = c.read(None, |link, _| Ok(link.clone()))?;
        Ok(crate::events::observe_deep(link, c.doc.clone(), observer))
    }
}
