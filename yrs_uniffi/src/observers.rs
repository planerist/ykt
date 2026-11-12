//! Observers: callbacks from Rust into the host language (Kotlin, Swift) when a document or a
//! shared type changes, as Yjs's `observe` / `on('update')`.
//!
//! - [YText::observe](crate::text::YText::observe): the text's change as a delta, per transaction.
//! - [YMap::observe](crate::map::YMap::observe): the changed keys; [YArray::observe](crate::array::YArray::observe):
//!   the array's change as a delta.
//! - [YDoc::observe_update_v1](crate::doc::YDoc::observe_update_v1) / `_v2`: every committed
//!   transaction's update, to store or send.
//! - [YDoc::observe_transaction_cleanup](crate::doc::YDoc::observe_transaction_cleanup): the state
//!   vectors before and after every committed transaction.
//!
//! Callbacks run synchronously, on the thread that commits the transaction, while the document is
//! still locked by it: a callback must not read or write the same document (that fails with
//! `AnotherTx` / `AnotherRwTx`); it gets everything it needs in the event, and can defer more work.
//!
//! Each subscription is a [YSubscription]: `unsubscribe()` stops it, and so does releasing it (the
//! host's GC / ARC, Kotlin's `close()`). Unsubscribing is safe anywhere, inside a callback too: the
//! callback is switched off at once, and removed from the document when the document allows it.

use crate::attrs::{into_yattrs, into_yvalue, YAttributes};
use crate::delta::YDelta;
use crate::tools::Result;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use yrs::types::Delta;
use yrs::{Origin, Out, StateVector, TransactionMut};

/// What a text's observer gets for a committed transaction that changed the text.
#[derive(uniffi::Record)]
pub struct YTextEvent {
    /// The change, as Yjs's `event.delta`: retain / insert / delete in UTF-16 units, with the
    /// formatting attributes of inserts and retains (a `null` attribute value removes it).
    pub delta: Vec<YDelta>,
    /// The origin the transaction was opened with (`YDoc.transaction(origin)`), as bytes; none for
    /// a transaction without one.
    pub origin: Option<Vec<u8>>,
}

/// What an update observer gets for a committed transaction that changed the document.
#[derive(uniffi::Record)]
pub struct YUpdateEvent {
    /// The transaction's changes as one update (lib0 v1 or v2 encoding, as subscribed), ready to be
    /// applied to another replica (`applyUpdate`) or stored.
    pub update: Vec<u8>,
    pub origin: Option<Vec<u8>>,
}

/// What a transaction-cleanup observer gets for every committed transaction.
#[derive(uniffi::Record)]
pub struct YTransactionCleanupEvent {
    /// The document's state vector (client id → clock) before the transaction.
    pub before_state: HashMap<u64, u32>,
    /// Its state vector after the transaction.
    pub after_state: HashMap<u64, u32>,
    /// What the transaction deleted, encoded as a lib0 v1 delete set.
    pub delete_set: Vec<u8>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YTextObserver: Send + Sync {
    fn on_change(&self, event: YTextEvent);
}

#[uniffi::export(with_foreign)]
pub trait YUpdateObserver: Send + Sync {
    fn on_update(&self, event: YUpdateEvent);
}

#[uniffi::export(with_foreign)]
pub trait YTransactionCleanupObserver: Send + Sync {
    fn on_cleanup(&self, event: YTransactionCleanupEvent);
}

/// An active subscription. `unsubscribe()` stops it; releasing it does too.
#[derive(uniffi::Object)]
pub struct YSubscription {
    active: Arc<AtomicBool>,
    /// Removes the callback from the document; `false` while the document is locked (the callback
    /// stays registered, switched off by `active`).
    remove: Mutex<Option<Box<dyn FnOnce() -> bool + Send>>>,
}

impl YSubscription {
    pub(crate) fn new(active: Arc<AtomicBool>, remove: impl FnOnce() -> bool + Send + 'static) -> Arc<Self> {
        Arc::new(YSubscription {
            active,
            remove: Mutex::new(Some(Box::new(remove))),
        })
    }

    fn stop(&self) -> bool {
        let was_active = self.active.swap(false, Ordering::SeqCst);
        if let Some(remove) = self.remove.lock().unwrap_or_else(|e| e.into_inner()).take() {
            remove();
        }
        was_active
    }
}

#[uniffi::export]
impl YSubscription {
    /// Stops the callbacks of this subscription. Returns `false` if it was already stopped.
    pub fn unsubscribe(&self) -> bool {
        self.stop()
    }

    /// Whether the callbacks still run.
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

impl Drop for YSubscription {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A key of its own for every subscription (yrs keys its observers by an origin).
pub(crate) fn subscription_key() -> Origin {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    Origin::from(format!("ykt-observer-{}", NEXT.fetch_add(1, Ordering::Relaxed)))
}

pub(crate) fn origin_bytes(txn: &TransactionMut) -> Option<Vec<u8>> {
    txn.origin().map(|origin| origin.as_ref().to_vec())
}

pub(crate) fn state_map(state: &StateVector) -> HashMap<u64, u32> {
    state.iter().map(|(client, clock)| (client.get(), *clock)).collect()
}

/// A yrs text delta in ykt's form. Inserted embeds that aren't plain values (a shared type
/// embedded in the text) come as their string form.
pub(crate) fn text_delta(delta: &[Delta<Out>], txn: &TransactionMut) -> Vec<YDelta> {
    let attrs = |a: &Option<Box<yrs::types::Attrs>>| -> Option<YAttributes> { a.as_ref().map(|a| into_yattrs((**a).clone())) };
    delta
        .iter()
        .map(|d| match d {
            Delta::Inserted(Out::Any(any), a) => YDelta::YInsert(into_yvalue(any), attrs(a)),
            Delta::Inserted(other, a) => YDelta::YInsert(crate::attrs::YValue::String(other.clone().to_string(txn)), attrs(a)),
            Delta::Deleted(len) => YDelta::YDelete(*len),
            Delta::Retain(len, a) => YDelta::YRetain(*len, attrs(a)),
        })
        .collect()
}

/// Encodes a delete set (lib0 v1).
pub(crate) fn encode_delete_set(set: &yrs::IdSet) -> Vec<u8> {
    use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
    let mut encoder = EncoderV1::new();
    set.encode(&mut encoder);
    encoder.to_vec()
}

pub(crate) type ObserveResult = Result<Arc<YSubscription>>;
