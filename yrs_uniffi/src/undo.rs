//! Undo / redo over a document's shared types (yrs's `UndoManager`, Yjs's `Y.UndoManager`).
//!
//! A `YUndoManager` records the changes of the transactions it tracks — by their origin — to the
//! types in its scope, as stack items: `undo()` reverts the last one and moves it to the redo stack,
//! `redo()` brings it back. Changes from other origins (remote updates, other editors) are never
//! undone, and an undo keeps them: it reverts only its own items. Transactions that come within the
//! capture timeout of each other merge into one item; `stopCapturing()` ends the item at once.
//!
//! Stack items carry a meta map (`YAttributes`), as Yjs's `stackItem.meta`: an item-added observer
//! returns what to store (e.g. the selection before the change), an item-popped observer gets it back
//! on undo or redo to restore it.
//!
//! The manager follows the document's rules: one call at a time; its callbacks run on the
//! committing thread with the document locked, and inside them neither the manager nor the document
//! may be used (that throws `AnotherTx`). `undo()`, `redo()` and `clear()` need the document
//! unlocked: inside a transaction or a callback they throw instead of waiting for a lock that
//! can't come.

use crate::attrs::YAttributes;
use crate::collection::same_doc;
use crate::doc::YDoc;
use crate::observers::{subscription_key, ObserveResult, YSubscription};
use crate::text::YText;
use crate::tools::{Error, Result};
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use yrs::undo::{EventKind, Options, UndoManager};
use yrs::{Doc, Origin, Transact};

/// Options of a `YUndoManager`.
#[derive(uniffi::Record)]
pub struct YUndoOptions {
    /// Changes closer than this to the previous tracked change merge into its stack item (Yjs's
    /// `captureTimeout`). 0 makes every transaction an item of its own.
    #[uniffi(default = 500)]
    pub capture_timeout_millis: u64,
    /// The origins (`YDoc.transaction(origin)`, or the origin an update is applied with) whose
    /// transactions are tracked, as Yjs's `trackedOrigins`. Empty: transactions without an origin
    /// are tracked; given: only those of these origins (and none without an origin).
    #[uniffi(default)]
    pub tracked_origins: Vec<String>,
}

/// Which stack an item is on, or comes off.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum YUndoKind {
    /// The undo stack: a tracked change was recorded, or `undo()` takes it back.
    Undo,
    /// The redo stack: `undo()` recorded how to redo, or `redo()` does it.
    Redo,
}

/// A stack item added, extended or popped.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct YUndoEvent {
    pub kind: YUndoKind,
    /// The origin of the transaction behind it: the tracked one for a recorded change, the
    /// manager's own (`YUndoManager.origin()`) for undo and redo.
    pub origin: Option<Vec<u8>>,
    /// The item's meta: what the item-added (or -updated) observers stored with it.
    pub meta: YAttributes,
}

/// Called when a stack item is added (or extended, for `observeItemUpdated`). Returns the meta to
/// keep with the item — e.g. the selection to restore when it is undone — or `null` to leave it.
#[uniffi::export(with_foreign)]
pub trait YUndoItemObserver: Send + Sync {
    fn on_item(&self, event: YUndoEvent) -> Option<YAttributes>;
}

/// Called when `undo()` or `redo()` pops a stack item, with the meta stored with it.
#[uniffi::export(with_foreign)]
pub trait YUndoPopObserver: Send + Sync {
    fn on_pop(&self, event: YUndoEvent);
}

/// Undo / redo of the tracked changes to a document's texts (see the module docs).
#[derive(uniffi::Object)]
pub struct YUndoManager {
    /// `None` only while it is dropped. A lock, not a cell: a second call at once (another thread,
    /// or a callback of the manager calling it again) fails instead of racing.
    manager: Mutex<Option<UndoManager<YAttributes>>>,
    doc: Doc,
    origin: Vec<u8>,
    /// Above 0 while one of the manager's callbacks runs (the manager is in the middle of its work).
    in_callback: Arc<AtomicUsize>,
}

// SAFETY: as YText / YTransaction: the embedding application calls into a document (and this
// manager) one call at a time, from any thread. The manager's own state is behind a lock.
unsafe impl Send for YUndoManager {}
unsafe impl Sync for YUndoManager {}

#[uniffi::export]
impl YUndoManager {
    /// A manager of the changes to `scope`, a text of `doc` (`expandScope` adds more), with
    /// `options` (by default: a 500 ms capture timeout, tracking transactions without an origin).
    /// Throws while `doc` is locked (an open transaction, a callback).
    #[uniffi::constructor(default(options = None))]
    pub fn new(doc: &YDoc, scope: &YText, options: Option<YUndoOptions>) -> Result<Self> {
        let (text, text_doc) = scope.integrated_ref()?;
        if !same_doc(&text_doc, doc.deref()) {
            return Err(Error::Custom("the text belongs to another document".into()));
        }
        // Registering with the document takes its lock, and yrs unwraps that: check it's free.
        drop(doc.try_transact().map_err(|_| Error::AnotherTx)?);
        let mut opts: Options<YAttributes> = Options::default();
        if let Some(o) = options {
            opts.capture_timeout_millis = o.capture_timeout_millis;
            for origin in o.tracked_origins {
                opts.tracked_origins.insert(Origin::from(origin.as_str()));
            }
        }
        let mut manager = UndoManager::with_options(opts);
        manager.expand_scope(doc.deref(), &text);
        let origin = manager.as_origin().as_ref().to_vec();
        Ok(YUndoManager {
            manager: Mutex::new(Some(manager)),
            doc: doc.deref().clone(),
            origin,
            in_callback: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Adds a text of the same document to the scope.
    pub fn expand_scope(&self, text: &YText) -> Result<()> {
        let (text, text_doc) = text.integrated_ref()?;
        if !same_doc(&text_doc, &self.doc) {
            return Err(Error::Custom("the text belongs to another document".into()));
        }
        self.with(|m| m.expand_scope(&text_doc, &text))
    }

    /// Reverts the last undo stack item and moves it to the redo stack. Returns whether anything
    /// changed (`false` with an empty stack). Throws inside a transaction or a callback.
    pub fn undo(&self) -> Result<bool> {
        self.with_unlocked_doc(|m| m.undo_blocking())
    }

    /// Re-applies the last undone item. Returns whether anything changed. Throws inside a
    /// transaction or a callback.
    pub fn redo(&self) -> Result<bool> {
        self.with_unlocked_doc(|m| m.redo_blocking())
    }

    pub fn can_undo(&self) -> Result<bool> {
        self.with(|m| m.can_undo())
    }

    pub fn can_redo(&self) -> Result<bool> {
        self.with(|m| m.can_redo())
    }

    /// Ends the current stack item: the next tracked change starts a new one, however soon it
    /// comes (Yjs's `stopCapturing`).
    pub fn stop_capturing(&self) -> Result<()> {
        self.with(|m| m.reset())
    }

    /// Empties both stacks. Throws inside a transaction or a callback.
    pub fn clear(&self) -> Result<()> {
        self.with_unlocked_doc(|m| m.clear_all())
    }

    /// Tracks the transactions of `origin` too.
    pub fn add_tracked_origin(&self, origin: String) -> Result<()> {
        self.with(|m| m.include_origin(origin.as_str()))
    }

    /// Stops tracking the transactions of `origin`.
    pub fn remove_tracked_origin(&self, origin: String) -> Result<()> {
        self.with(|m| m.exclude_origin(origin.as_str()))
    }

    /// The origin of the manager's own undo and redo transactions, as observers see it (Yjs's
    /// `transaction.origin === undoManager`): e.g. to tell an undo from a remote change in a text
    /// observer.
    pub fn origin(&self) -> Vec<u8> {
        self.origin.clone()
    }

    /// Calls `observer` when a stack item is added — a tracked change recorded (`Undo`), or an undo
    /// recorded for redo (`Redo`) — and keeps the meta it returns with the item.
    pub fn observe_item_added(self: Arc<Self>, observer: Arc<dyn YUndoItemObserver>) -> ObserveResult {
        let (key, active, callback) = self.item_callback(observer);
        self.with(|m| m.observe_item_added(key.clone(), callback))?;
        Ok(self.subscription(active, move |m| m.unobserve_item_added(key)))
    }

    /// Calls `observer` when the last stack item is extended by a change within the capture
    /// timeout, and keeps the meta it returns with the item.
    pub fn observe_item_updated(self: Arc<Self>, observer: Arc<dyn YUndoItemObserver>) -> ObserveResult {
        let (key, active, callback) = self.item_callback(observer);
        self.with(|m| m.observe_item_updated(key.clone(), callback))?;
        Ok(self.subscription(active, move |m| m.unobserve_item_updated(key)))
    }

    /// Calls `observer` when `undo()` (`Undo`) or `redo()` (`Redo`) pops a stack item, with the meta
    /// stored with it. It runs before `undo()` / `redo()` returns, with the document still locked:
    /// keep what it needs and apply it after.
    pub fn observe_item_popped(self: Arc<Self>, observer: Arc<dyn YUndoPopObserver>) -> ObserveResult {
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        let in_callback = self.in_callback.clone();
        self.with(|m| {
            m.observe_item_popped(key.clone(), move |_txn, event| {
                if running.load(Ordering::SeqCst) {
                    let _busy = Busy::enter(&in_callback);
                    observer.on_pop(undo_event(event));
                }
            })
        })?;
        Ok(self.subscription(active, move |m| m.unobserve_item_popped(key)))
    }
}

type Manager = UndoManager<YAttributes>;

impl YUndoManager {
    /// Runs `f` on the manager; fails while it is busy (inside its callbacks, or another call).
    fn with<R>(&self, f: impl FnOnce(&mut Manager) -> R) -> Result<R> {
        if self.in_callback.load(Ordering::SeqCst) > 0 {
            return Err(Error::AnotherTx);
        }
        let mut guard = match self.manager.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(Error::AnotherTx),
        };
        let manager = guard.as_mut().ok_or(Error::RefDisposed)?;
        Ok(f(manager))
    }

    /// As `with`, for what takes the document's lock (yrs then waits for it, forever if this thread
    /// holds it): fails at once while the document is locked.
    fn with_unlocked_doc<R>(&self, f: impl FnOnce(&mut Manager) -> R) -> Result<R> {
        drop(self.doc.try_transact().map_err(|_| Error::AnotherTx)?);
        self.with(f)
    }

    fn item_callback(
        &self,
        observer: Arc<dyn YUndoItemObserver>,
    ) -> (Origin, Arc<AtomicBool>, impl FnMut(&yrs::TransactionMut<'_>, &mut yrs::undo::Event<YAttributes>) + 'static) {
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        let in_callback = self.in_callback.clone();
        let callback = move |_txn: &yrs::TransactionMut<'_>, event: &mut yrs::undo::Event<YAttributes>| {
            if running.load(Ordering::SeqCst) {
                let meta = {
                    let _busy = Busy::enter(&in_callback);
                    observer.on_item(undo_event(event))
                };
                if let Some(meta) = meta {
                    *event.meta_mut() = meta;
                }
            }
        };
        (key, active, callback)
    }

    /// A subscription to one of the manager's observers. It doesn't keep the manager alive; when
    /// the manager is busy (unsubscribing inside its callback) the callback is only switched off,
    /// and goes with the manager.
    fn subscription(
        self: &Arc<Self>,
        active: Arc<AtomicBool>,
        remove: impl FnOnce(&mut Manager) -> bool + Send + 'static,
    ) -> Arc<YSubscription> {
        let manager = Arc::downgrade(self);
        YSubscription::new(active, move || match manager.upgrade() {
            Some(manager) => manager.with(remove).unwrap_or(false),
            None => false,
        })
    }
}

impl Drop for YUndoManager {
    fn drop(&mut self) {
        let manager = match self.manager.get_mut() {
            Ok(m) => m.take(),
            Err(e) => e.into_inner().take(),
        };
        let Some(manager) = manager else { return };
        // yrs's own drop unregisters the manager from the document with an unwrap, which panics
        // while the document is locked (released inside a callback or a transaction, or by a GC
        // thread at such a moment) and would leave the document calling into freed memory.
        // Unregister first; if the document is locked, the manager is left as it is (leaked, with
        // the document it holds), which is safe.
        let origin = manager.as_origin();
        let detached = self.doc.unobserve_after_transaction(origin.clone()).is_ok()
            && self.doc.unobserve_destroy(origin).is_ok();
        if detached {
            drop(manager);
        } else {
            std::mem::forget(manager);
        }
    }
}

fn undo_event(event: &yrs::undo::Event<YAttributes>) -> YUndoEvent {
    YUndoEvent {
        kind: match event.kind() {
            EventKind::Undo => YUndoKind::Undo,
            EventKind::Redo => YUndoKind::Redo,
        },
        origin: event.origin().map(|o| o.as_ref().to_vec()),
        meta: event.meta().clone(),
    }
}

/// Marks a manager's callback as running, until dropped (also if the callback panics).
struct Busy<'a>(&'a AtomicUsize);

impl<'a> Busy<'a> {
    fn enter(count: &'a AtomicUsize) -> Self {
        count.fetch_add(1, Ordering::SeqCst);
        Busy(count)
    }
}

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}
