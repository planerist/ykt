use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use std::cell::RefCell;
use std::sync::Arc;
use yrs::{BranchID, Doc, Hook, ReadTxn, SharedRef, Store, Transact, Transaction, TransactionMut};

#[derive(Clone)]
pub enum SharedCollection<P, S> {
    Integrated(Integrated<S>),
    Prelim(P),
}

impl<P, S: SharedRef + 'static> SharedCollection<P, S> {
    #[inline]
    pub fn prelim(prelim: P) -> Self {
        SharedCollection::Prelim(prelim)
    }

    #[inline]
    pub fn integrated(shared_ref: S, doc: Doc) -> Self {
        SharedCollection::Integrated(Integrated::new(shared_ref, doc))
    }

    #[inline]
    pub fn is_prelim(&self) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(_) => false,
        }
    }
}

/// A preliminary type's content, or an integrated type's handle: what a ykt handle's cell holds.
pub(crate) type SharedCell<P, S> = RefCell<SharedCollection<P, S>>;

/// Runs `prelim` on a preliminary type's content, or `integrated` on an integrated type. The cell
/// isn't kept borrowed while `integrated` runs: the commit's callbacks may use the same handle (and
/// get `Locked` from the document rather than a panic from the cell).
pub(crate) fn with_cell<P, S: Clone, R>(
    cell: &SharedCell<P, S>,
    prelim: impl FnOnce(&mut P) -> Result<R>,
    integrated: impl FnOnce(&Integrated<S>) -> Result<R>,
) -> Result<R> {
    let found = match &*cell.try_borrow().map_err(|_| Error::Locked)? {
        SharedCollection::Integrated(c) => Some(c.clone()),
        SharedCollection::Prelim(_) => None,
    };
    match found {
        Some(c) => integrated(&c),
        None => match &mut *cell.try_borrow_mut().map_err(|_| Error::Locked)? {
            SharedCollection::Prelim(p) => prelim(p),
            SharedCollection::Integrated(_) => Err(Error::Locked),
        },
    }
}

/// The integrated type of a cell; `NotIntegrated` for a preliminary one.
pub(crate) fn integrated_of<P, S: Clone>(cell: &SharedCell<P, S>) -> Result<Integrated<S>> {
    match &*cell.try_borrow().map_err(|_| Error::Locked)? {
        SharedCollection::Integrated(c) => Ok(c.clone()),
        SharedCollection::Prelim(_) => Err(Error::NotIntegrated),
    }
}

/// Whether a cell holds a preliminary type.
pub(crate) fn is_prelim<P, S: SharedRef + 'static>(cell: &SharedCell<P, S>) -> bool {
    cell.try_borrow().map(|c| c.is_prelim()).unwrap_or(false)
}

/// Whether a cell's type is in its document, read in `txn` (of that document): `false` for a
/// preliminary type or a deleted one.
pub(crate) fn is_alive<P, S: SharedRef + Clone + 'static>(cell: &SharedCell<P, S>, txn: &YTransaction) -> Result<bool> {
    match integrated_of(cell) {
        Ok(c) => txn.read(|txn| {
            c.check_doc(txn.doc())?;
            Ok(c.hook.get(txn).is_some())
        }),
        Err(Error::NotIntegrated) => Ok(false),
        Err(e) => Err(e),
    }
}

/// A shared type's identity in its document, the same on every replica (yrs's `BranchID`): a root
/// type's name, or the id of the item that holds a nested one.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq, Hash)]
pub enum YBranchId {
    Root { name: String },
    Nested { client: u64, clock: u32 },
}

impl From<&BranchID> for YBranchId {
    fn from(id: &BranchID) -> Self {
        match id {
            BranchID::Root(name) => YBranchId::Root { name: name.to_string() },
            BranchID::Nested(id) => YBranchId::Nested { client: id.client.get(), clock: id.clock },
        }
    }
}

#[derive(Clone)]
pub struct Integrated<S> {
    pub hook: Hook<S>,
    pub doc: Doc,
}

impl<S: SharedRef + 'static> Integrated<S> {
    pub fn new(shared_ref: S, doc: Doc) -> Self {
        let desc = shared_ref.hook();
        Integrated { hook: desc, doc }
    }

    pub fn id(&self) -> &BranchID {
        self.hook.id()
    }

    /// Runs `f` with the type, in `txn` if given (a transaction of this type's document), else in a
    /// read-write transaction of its own.
    pub fn readonly<F, R>(&self, txn: Option<Arc<YTransaction>>, f: F) -> Result<R>
    where
        F: FnOnce(&S, &TransactionMut<'_>) -> Result<R>,
    {
        match txn {
            Some(txn) => txn.read(|txn| {
                self.check_doc(txn.doc())?;
                f(&self.resolve(txn)?, txn)
            }),
            None => {
                let txn = self.transact_mut()?;
                f(&self.resolve(&txn)?, &txn)
            }
        }
    }

    /// Runs `f` with the type and a transaction to write in: `txn` if given (of this type's
    /// document), else one of its own, committed when `f` returns.
    pub fn mutably<F, T>(&self, txn: Option<Arc<YTransaction>>, f: F) -> Result<T>
    where
        F: FnOnce(&S, &mut TransactionMut<'_>) -> Result<T>,
    {
        match txn {
            Some(txn) => txn.write(|txn| {
                self.check_doc(txn.doc())?;
                let shared_ref = self.resolve(txn)?;
                f(&shared_ref, txn)
            }),
            None => {
                let mut txn = self.transact_mut()?;
                let shared_ref = self.resolve(&txn)?;
                f(&shared_ref, &mut txn)
            }
        }
    }

    /// As `readonly`, without a transaction of its own when none is given: a read-only one, which
    /// commits nothing (no empty transaction reaching the document's observers).
    pub fn read<F, R>(&self, txn: Option<Arc<YTransaction>>, f: F) -> Result<R>
    where
        F: FnOnce(&S, &ReadView<'_>) -> Result<R>,
    {
        match txn {
            Some(txn) => txn.read(|txn| {
                self.check_doc(txn.doc())?;
                let view = ReadView(txn.store());
                f(&self.resolve(&view)?, &view)
            }),
            None => {
                let txn = self.transact()?;
                let view = ReadView(txn.store());
                f(&self.resolve(&view)?, &view)
            }
        }
    }

    pub fn resolve<T: ReadTxn>(&self, txn: &T) -> Result<S> {
        self.hook.get(txn).ok_or(Error::TypeDeleted)
    }

    /// `WrongDocument` unless `doc` is this type's document.
    pub fn check_doc(&self, doc: &Doc) -> Result<()> {
        if same_doc(doc, &self.doc) {
            Ok(())
        } else {
            Err(Error::WrongDocument)
        }
    }

    pub fn transact(&self) -> Result<Transaction<'_>> {
        self.doc.try_transact().map_err(|_| Error::Locked)
    }

    pub fn transact_mut(&self) -> Result<TransactionMut<'_>> {
        self.doc.try_transact_mut().map_err(|_| Error::Locked)
    }
}

/// Read access to a document's store, whichever transaction holds it (a read-only one or a
/// read-write one).
pub struct ReadView<'a>(pub &'a Store);

impl ReadTxn for ReadView<'_> {
    fn store(&self) -> &Store {
        self.0
    }
}

/// Whether two handles are the same document (yrs's `==` compares guids, which replicas share).
pub fn same_doc(a: &Doc, b: &Doc) -> bool {
    a.clone().as_raw() == b.clone().as_raw()
}
