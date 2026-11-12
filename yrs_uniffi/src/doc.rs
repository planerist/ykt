use crate::collection::SharedCollection;
use crate::observers::{
    encode_delete_set, origin_bytes, state_map, subscription_key, ObserveResult, YSubscription,
    YTransactionCleanupEvent, YTransactionCleanupObserver, YUpdateEvent, YUpdateObserver,
};
use crate::events::{
    YAfterTransactionEvent, YAfterTransactionObserver, YDestroyEvent, YDestroyObserver, YSubdocsEvent, YSubdocsObserver,
};
use crate::text::YText;
use crate::tools::Error;
use crate::tools::Result;
use crate::updates::DocEncoderV2;
use crate::transaction::YTransaction;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::block::ClientID;
use yrs::types::{TypeRef, TYPE_REFS_DOC};
use yrs::updates::encoder::Encoder;
use yrs::{Doc, OffsetKind, Options, ReadTxn, Transact};
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

    /// Whether the document isn't a subdocument of another one (it can be inserted as one). Throws
    /// `Locked` while it is locked (yrs would wait for it).
    pub fn prelim(&self) -> Result<bool> {
        Ok(self.parent()?.is_none())
    }

    /// The document this one is a subdocument of, `null` for none. Throws `Locked` while it is
    /// locked.
    pub fn parent_doc(&self) -> Result<Option<Arc<YDoc>>> {
        Ok(self.parent()?.map(|doc| Arc::new(YDoc(doc))))
    }

    /// Asks to load this subdocument (Yjs's `doc.load()`): `shouldLoad` turns on, and the parent's
    /// subdocument observers hear it among `loaded`. Nothing for a document that isn't a
    /// subdocument. Throws `Locked` while it or its parent is locked.
    pub fn load(&self) -> Result<()> {
        match self.parent()? {
            Some(parent) => {
                let mut parent_txn = parent.try_transact_mut().map_err(|_| Error::Locked)?;
                self.0.load(&mut parent_txn);
                Ok(())
            }
            None => Ok(()),
        }
    }

    /// Destroys the document (Yjs's `doc.destroy()`; `destroyDoc` here, as `destroy` frees a
    /// handle in Kotlin): its destroy observers run and its
    /// subscriptions end; a subdocument is replaced in its parent by a new, unloaded instance (the
    /// parent's subdocument observers hear it removed and added). Throws `Locked` while it, its
    /// parent or one of its subdocuments is locked (yrs would wait for them).
    #[uniffi::method(name = "destroyDoc")]
    pub fn destroy(&self) -> Result<()> {
        let parent = {
            let txn = self.0.try_transact_mut().map_err(|_| Error::Locked)?;
            for subdoc in txn.subdocs() {
                drop(subdoc.try_transact_mut().map_err(|_| Error::Locked)?);
            }
            txn.parent_doc()
        };
        match parent {
            Some(parent) => {
                let mut parent_txn = parent.try_transact_mut().map_err(|_| Error::Locked)?;
                self.0.destroy(Some(&mut parent_txn));
            }
            None => self.0.destroy(None),
        }
        Ok(())
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
    /// `doc.getText`; only creating it is a transaction). The same name gives the same text, whichever handle is used. Throws while the
    /// document is locked (an open transaction, a callback) instead of waiting for it: yrs's
    /// `get_or_insert_text` would wait for good.
    ///
    /// A root of this name that is of another type is read as a text.
    pub fn get_text(&self, name: &str) -> Result<YText> {
        // A root another client made (an update) has no type yet: making it one takes a write
        // transaction, as does a new root. The read transaction ends here.
        let existing = self.0.try_transact().map_err(|_| Error::Locked)?.get_text(name).filter(|r| matches!(AsRef::<yrs::branch::Branch>::as_ref(r).type_ref(), TypeRef::Text));
        let text = match existing {
            Some(text) => text,
            None => TextRef::root(name).get_or_create(&mut self.0.try_transact_mut().map_err(|_| Error::Locked)?),
        };
        Ok(YText::new(SharedCollection::integrated(text, self.0.clone())))
    }

    /// Returns the `YXmlFragment` root type of this `name`, creating it if the document hasn't got it
    /// (Yjs's `doc.getXmlFragment`). Throws while the document is locked (an open transaction, a
    /// callback) instead of waiting for it.
    pub fn get_xml_fragment(&self, name: &str) -> Result<YXmlFragment> {
        // A root another client made (an update) has no type yet: making it one takes a write
        // transaction, as does a new root. The read transaction ends here.
        let existing = self.0.try_transact().map_err(|_| Error::Locked)?.get_xml_fragment(name).filter(|r| matches!(AsRef::<yrs::branch::Branch>::as_ref(r).type_ref(), TypeRef::XmlFragment));
        let fragment = match existing {
            Some(fragment) => fragment,
            None => XmlFragmentRef::root(name).get_or_create(&mut self.0.try_transact_mut().map_err(|_| Error::Locked)?),
        };
        Ok(YXmlFragment::new_with_collection(SharedCollection::integrated(fragment, self.0.clone())))
    }

    /// Returns the `YMap` root type of this `name`, creating it if the document hasn't got it (Yjs's
    /// `doc.getMap`). Throws while the document is locked (an open transaction, a callback) instead
    /// of waiting for it.
    pub fn get_map(&self, name: &str) -> Result<YMap> {
        // A root another client made (an update) has no type yet: making it one takes a write
        // transaction, as does a new root. The read transaction ends here.
        let existing = self.0.try_transact().map_err(|_| Error::Locked)?.get_map(name).filter(|r| matches!(AsRef::<yrs::branch::Branch>::as_ref(r).type_ref(), TypeRef::Map));
        let map = match existing {
            Some(map) => map,
            None => MapRef::root(name).get_or_create(&mut self.0.try_transact_mut().map_err(|_| Error::Locked)?),
        };
        Ok(YMap(Integrated::new(map, self.0.clone())))
    }

    /// Returns the `YArray` root type of this `name`, creating it if the document hasn't got it
    /// (Yjs's `doc.getArray`). Throws while the document is locked (an open transaction, a
    /// callback) instead of waiting for it.
    pub fn get_array(&self, name: &str) -> Result<YArray> {
        // A root another client made (an update) has no type yet: making it one takes a write
        // transaction, as does a new root. The read transaction ends here.
        let existing = self.0.try_transact().map_err(|_| Error::Locked)?.get_array(name).filter(|r| matches!(AsRef::<yrs::branch::Branch>::as_ref(r).type_ref(), TypeRef::Array));
        let array = match existing {
            Some(array) => array,
            None => ArrayRef::root(name).get_or_create(&mut self.0.try_transact_mut().map_err(|_| Error::Locked)?),
        };
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
            .observe_update_v2(key.clone(), move |txn, _event| {
                if running.load(Ordering::SeqCst) {
                    // The update written again, by `DocEncoderV2`: yrs's doesn't read after updates
                    // out of order.
                    let mut encoder = DocEncoderV2::new();
                    txn.encode_update(&mut encoder);
                    observer.on_update(YUpdateEvent { update: encoder.to_vec(), origin: origin_bytes(txn) });
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

    /// Calls `observer` after every committed transaction (an empty one too), after the types'
    /// observers and before the cleanup, with the state vectors before and after it, what it
    /// deleted and its origin (Yjs's `doc.on('afterTransaction')`). Fails while the document is
    /// locked.
    pub fn observe_after_transaction(&self, observer: Arc<dyn YAfterTransactionObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_after_transaction(key.clone(), move |txn| {
                if running.load(Ordering::SeqCst) {
                    observer.on_after_transaction(YAfterTransactionEvent {
                        before_state: state_map(txn.before_state()),
                        after_state: state_map(txn.after_state()),
                        delete_set: encode_delete_set(txn.delete_set()),
                        origin: origin_bytes(txn),
                    });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_after_transaction(key).unwrap_or(false)))
    }

    /// Calls `observer` after a committed transaction that added, removed or asked to load
    /// subdocuments of this document (Yjs's `doc.on('subdocs')`). Fails while the document is
    /// locked.
    pub fn observe_subdocs(&self, observer: Arc<dyn YSubdocsObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_subdocs(key.clone(), move |txn, event| {
                if running.load(Ordering::SeqCst) {
                    let docs = |iter: yrs::SubdocsEventIter| iter.map(|d| Arc::new(YDoc(d.clone()))).collect();
                    observer.on_subdocs(YSubdocsEvent {
                        added: docs(event.added()),
                        removed: docs(event.removed()),
                        loaded: docs(event.loaded()),
                        origin: origin_bytes(txn),
                    });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_subdocs(key).unwrap_or(false)))
    }

    /// Calls `observer` when the document is destroyed (`destroy()`, Yjs's `doc.on('destroy')`).
    /// Fails while the document is locked.
    pub fn observe_destroy(&self, observer: Arc<dyn YDestroyObserver>) -> ObserveResult {
        let (key, active, running) = Self::new_subscription();
        self.0
            .observe_destroy(key.clone(), move |_txn, doc| {
                if running.load(Ordering::SeqCst) {
                    observer.on_destroy(YDestroyEvent { guid: doc.guid().to_string() });
                }
            })
            .map_err(|_| Error::Locked)?;
        active.store(true, Ordering::SeqCst);
        let doc = self.0.clone();
        Ok(YSubscription::new(active, move || doc.unobserve_destroy(key).unwrap_or(false)))
    }
}

impl YDoc {
    /// The parent document, read without waiting for the lock.
    fn parent(&self) -> Result<Option<Doc>> {
        Ok(self.0.try_transact().map_err(|_| Error::Locked)?.parent_doc())
    }

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
