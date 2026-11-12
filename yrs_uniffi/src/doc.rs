use crate::collection::SharedCollection;
use crate::observers::{
    encode_delete_set, origin_bytes, state_map, subscription_key, ObserveResult, YSubscription,
    YTransactionCleanupEvent, YTransactionCleanupObserver, YUpdateEvent, YUpdateObserver,
};
use crate::text::YText;
use crate::tools::Error;
use crate::tools::Result;
use crate::transaction::YTransaction;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::block::ClientID;
use yrs::types::TYPE_REFS_DOC;
use yrs::{Doc, OffsetKind, Options, Transact};
use crate::xml_frag::YXmlFragment;
use crate::array::YArray;
use crate::collection::Integrated;
use crate::map::YMap;
use yrs::{ArrayRef, MapRef, RootRef, TextRef, XmlFragmentRef};

/// A ywasm document type. Documents are most important units of collaborative resources management.
/// All shared collections live within a scope of their corresponding documents. All updates are
/// generated on per-document basis (rather than individual shared type). All operations on shared
/// collections happen via [YTransaction], which lifetime is also bound to a document.
///
/// Document manages so-called root types, which are top-level shared types definitions (as opposed
/// to recursively nested types).
///
/// A basic workflow sample:
///
/// ```javascript
/// import YDoc from 'ywasm'
///
/// const doc = new YDoc()
/// const txn = doc.beginTransaction()
/// try {
///     const text = txn.getText('name')
///     text.push(txn, 'hello world')
///     const output = text.toString(txn)
///     console.log(output)
/// } finally {
///     txn.free()
/// }
/// ```
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
#[repr(transparent)]
pub struct YDoc(pub(crate) Doc);

impl Deref for YDoc {
    type Target = Doc;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<Doc> for YDoc {
    fn from(doc: Doc) -> Self {
        YDoc(doc)
    }
}

#[uniffi::export]
impl YDoc {
    /// Creates a new ywasm document. If `id` parameter was passed it will be used as this document
    /// globally unique identifier (it's up to caller to ensure that requirement). Otherwise it will
    /// be assigned a randomly generated number.
    #[uniffi::constructor(default(options=None))]
    pub fn new(options: Option<YDocOptions>) -> Self {
        let mut opt = Options::default();

        opt.offset_kind = crate::tools::offset_kind();

        if let Some(o) = options {
            o.fill(&mut opt);
        }

        Doc::with_options(opt).into()
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_DOC
    }

    /// Checks if a document is a preliminary type. It returns false, if current document
    /// is already a sub-document of another document.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.parent_doc().is_none()
    }

    /// Returns a parent document of this document or null if current document is not sub-document.
    pub fn parent_doc(&self) -> Option<Arc<YDoc>> {
        let doc = self.0.parent_doc()?;
        Some(Arc::new(YDoc(doc)))
    }

    /// Gets unique peer identifier of this `YDoc` instance.
    pub fn id(&self) -> f64 {
        self.client_id().get() as f64
    }

    /// Gets globally unique identifier of this `YDoc` instance.
    pub fn guid(&self) -> String {
        self.0.guid().to_string()
    }

    pub fn should_load(&self) -> bool {
        self.0.should_load()
    }

    pub fn auto_load(&self) -> bool {
        self.0.auto_load()
    }

    /// Returns a new transaction for this document. Ywasm shared data types execute their
    /// operations in a context of a given transaction. Each document can have only one active
    /// transaction at the time - subsequent attempts will cause exception to be thrown.
    ///
    /// Transactions started with `doc.beginTransaction` can be released using `transaction.free`
    /// method.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// // helper function used to simplify transaction
    /// // create/release cycle
    /// YDoc.prototype.transact = callback => {
    ///     const txn = this.transaction()
    ///     try {
    ///         return callback(txn)
    ///     } finally {
    ///         txn.free()
    ///     }
    /// }
    ///
    /// const doc = new YDoc()
    /// const text = doc.getText('name')
    /// doc.transact(txn => text.insert(txn, 0, 'hello world'))
    /// ```
    #[uniffi::method(default(origin=None))]
    pub fn transaction(&self, origin: Option<String>) -> Result<YTransaction> {
        let inner = if let Some(origin) = origin {
            self.try_transact_mut_with(yrs::Origin::from(origin))
        } else {
            self.try_transact_mut()
        }
        .map_err(|_| Error::Locked)?;

        Ok(YTransaction::from(inner))
    }

    /// Returns the `YText` root type of this `name`, creating it if the document hasn't got it (Yjs's
    /// `doc.getText`). The same name gives the same text, whichever handle is used. Throws while the
    /// document is locked (an open transaction, a callback) instead of waiting for it: yrs's
    /// `get_or_insert_text` would wait for good.
    ///
    /// A root of this name that is of another type is read as a text.
    pub fn get_text(&self, name: &str) -> Result<YText> {
        let mut txn = self.0.try_transact_mut().map_err(|_| Error::Locked)?;
        let text = TextRef::root(name).get_or_create(&mut txn);
        drop(txn);
        Ok(YText::new(SharedCollection::integrated(text, self.0.clone())))
    }

    /// Returns the `YXmlFragment` root type of this `name`, creating it if the document hasn't got it
    /// (Yjs's `doc.getXmlFragment`). Throws while the document is locked (an open transaction, a
    /// callback) instead of waiting for it.
    pub fn get_xml_fragment(&self, name: &str) -> Result<YXmlFragment> {
        let mut txn = self.0.try_transact_mut().map_err(|_| Error::Locked)?;
        let fragment = XmlFragmentRef::root(name).get_or_create(&mut txn);
        drop(txn);
        Ok(YXmlFragment::new_with_collection(SharedCollection::integrated(fragment, self.0.clone())))
    }

    /// Returns the `YMap` root type of this `name`, creating it if the document hasn't got it (Yjs's
    /// `doc.getMap`). Throws while the document is locked (an open transaction, a callback) instead
    /// of waiting for it.
    pub fn get_map(&self, name: &str) -> Result<YMap> {
        let mut txn = self.0.try_transact_mut().map_err(|_| Error::Locked)?;
        let map = MapRef::root(name).get_or_create(&mut txn);
        drop(txn);
        Ok(YMap(Integrated::new(map, self.0.clone())))
    }

    /// Returns the `YArray` root type of this `name`, creating it if the document hasn't got it
    /// (Yjs's `doc.getArray`). Throws while the document is locked (an open transaction, a
    /// callback) instead of waiting for it.
    pub fn get_array(&self, name: &str) -> Result<YArray> {
        let mut txn = self.0.try_transact_mut().map_err(|_| Error::Locked)?;
        let array = ArrayRef::root(name).get_or_create(&mut txn);
        drop(txn);
        Ok(YArray(Integrated::new(array, self.0.clone())))
    }

    /// Calls `observer` with every committed transaction's changes as one update, lib0 v1 encoding,
    /// and the transaction's origin (Yjs's `doc.on('update')`). Fails while a transaction is open.
    /// The callback runs on the committing thread while the document is still locked: it must not
    /// read or write the document.
    pub fn observe_update_v1(&self, observer: Arc<dyn YUpdateObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_update_v1(key.clone(), move |txn, event| {
                if running.load(Ordering::SeqCst) {
                    observer.on_update(YUpdateEvent { update: event.update.clone(), origin: origin_bytes(txn) });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_update_v1(key).unwrap_or(false)))
    }

    /// As `observe_update_v1`, in lib0 v2 encoding (Yjs's `doc.on('updateV2')`).
    pub fn observe_update_v2(&self, observer: Arc<dyn YUpdateObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_update_v2(key.clone(), move |txn, event| {
                if running.load(Ordering::SeqCst) {
                    observer.on_update(YUpdateEvent { update: event.update.clone(), origin: origin_bytes(txn) });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_update_v2(key).unwrap_or(false)))
    }

    /// Calls `observer` after every committed transaction with the state vectors before and after
    /// it and what it deleted (Yjs's `doc.on('afterTransaction')`, as yrs's transaction cleanup):
    /// e.g. to store or send only what changed. Fails while a transaction is open.
    pub fn observe_transaction_cleanup(&self, observer: Arc<dyn YTransactionCleanupObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_transaction_cleanup(key.clone(), move |txn, event| {
                if running.load(Ordering::SeqCst) {
                    observer.on_cleanup(YTransactionCleanupEvent {
                        before_state: state_map(&event.before_state),
                        after_state: state_map(&event.after_state),
                        delete_set: encode_delete_set(&event.delete_set),
                        origin: origin_bytes(txn),
                    });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_transaction_cleanup(key).unwrap_or(false)))
    }
}

impl YDoc {
    /// A subscription's key and its on/off switch (the callback's copy and the handle's). It is
    /// off until the callback is registered: yrs registers it in a transaction of its own, whose
    /// commit would otherwise reach the new callback (an empty cleanup event nobody made).
    fn new_subscription() -> (yrs::Origin, Arc<AtomicBool>, Arc<AtomicBool>) {
        let active = Arc::new(AtomicBool::new(false));
        (subscription_key(), active.clone(), active)
    }
}

#[derive(uniffi::Record)]
pub struct YDocOptions {
    #[uniffi(default = None)]
    pub client_id: Option<u64>,

    #[uniffi(default = None)]
    pub guid: Option<String>,

    #[uniffi(default = None)]
    pub collection_id: Option<String>,

    #[uniffi(default = None)]
    pub gc: Option<bool>,

    #[uniffi(default = None)]
    pub auto_load: Option<bool>,

    #[uniffi(default = None)]
    pub should_load: Option<bool>,

    #[uniffi(default = None)]
    pub bytes_offset: Option<bool>,
}

impl YDocOptions {
    fn fill(self, options: &mut Options) {
        if let Some(value) = self.client_id {
            options.client_id = ClientID::new(value);
        }
        if let Some(value) = self.guid {
            options.guid = value.into();
        }
        if let Some(value) = self.collection_id {
            options.collection_id = Some(value.into());
        }
        if let Some(value) = self.gc {
            options.skip_gc = !value;
        }
        if let Some(bytes_offset) = self.bytes_offset {
            options.offset_kind = if bytes_offset {
                OffsetKind::Bytes
            } else {
                OffsetKind::Utf16
            };
        }
        if let Some(value) = self.auto_load {
            options.auto_load = value;
        }
        if let Some(value) = self.should_load {
            options.should_load = value;
        }
    }
}
