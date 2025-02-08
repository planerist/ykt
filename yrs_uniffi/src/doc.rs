use crate::collection::SharedCollection;
use crate::text::YText;
use crate::transaction::YTransaction;
use std::ops::Deref;
use std::sync::Arc;
use yrs::types::TYPE_REFS_DOC;
use yrs::{Doc, OffsetKind, Options, Transact};
use crate::tools::Error;
use crate::tools::Result;

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

        if crate::tools::is_wasm() {
            opt.offset_kind = OffsetKind::Utf16;
        }

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
        self.client_id() as f64
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
        }.map_err(|_| Error::AnotherRwTx)?;

        Ok(YTransaction::from(inner))
    }

    /// Returns a `YText` shared data type, that's accessible for subsequent accesses using given
    /// `name`.
    ///
    /// If there was no instance with this name before, it will be created and then returned.
    ///
    /// If there was an instance with this name, but it was of different type, it will be projected
    /// onto `YText` instance.
    pub fn get_text(&self, name: &str) -> YText {
        let shared_ref = self.get_or_insert_text(name);
        YText::new(SharedCollection::integrated(shared_ref, self.0.clone()))
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
}

impl YDocOptions {
    fn fill(self, options: &mut Options) {
        if let Some(value) = self.client_id {
            options.client_id = value;
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
        if let Some(value) = self.auto_load {
            options.auto_load = value;
        }
        if let Some(value) = self.should_load {
            options.should_load = value;
        }
    }
}
