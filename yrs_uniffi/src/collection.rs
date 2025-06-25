use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::{BranchID, Doc, Hook, ReadTxn, SharedRef, Transact, Transaction, TransactionMut};

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

    pub fn id(&self) -> Result<BranchID> {
        match self {
            SharedCollection::Prelim(_) => Err(Error::InvalidPrelimOp),
            SharedCollection::Integrated(c) => Ok(c.hook.id().clone()),
        }
    }

    pub fn try_integrated(&self) -> Result<(&BranchID, &Doc)> {
        match self {
            SharedCollection::Integrated(i) => {
                let branch_id = i.hook.id();
                let doc = &i.doc;
                Ok((branch_id, doc))
            }
            SharedCollection::Prelim(_) => Err(Error::InvalidPrelimOp),
        }
    }

    #[inline]
    pub fn is_prelim(&self) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(_) => false,
        }
    }

    pub fn is_alive(&self, txn: &YTransaction) -> bool {
        match self {
            SharedCollection::Prelim(_) => true,
            SharedCollection::Integrated(col) => {
                let desc = &col.hook;
                desc.get(txn.get_inner().borrow().deref()).is_some()
            }
        }
    }

    #[inline]
    pub fn branch_id(&self) -> Option<&BranchID> {
        match self {
            SharedCollection::Prelim(_) => None,
            SharedCollection::Integrated(v) => Some(v.hook.id()),
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

    pub fn readonly<F, R>(&self, txn: Option<Arc<YTransaction>>, f: F) -> Result<R>
    where
        F: FnOnce(&S, &TransactionMut<'_>) -> Result<R>,
    {
        match txn {
            Some(txn) => {
                let inner = txn.get_inner();
                let txn = inner.borrow();
                let txn = txn.deref();
                let txn = txn.deref();
                let shared_ref= self.resolve(txn)?;
                f(&shared_ref, txn)
            }
            None => {
                let txn = self.transact_mut()?; // :( rust type inference problem
                let shared_ref= self.resolve(&txn)?;
                f(&shared_ref, &txn)
            }
        }
    }

    pub fn mutably<F, T>(&self, txn: Option<Arc<YTransaction>>, f: F) -> Result<T>
    where
        F: FnOnce(&S, &mut TransactionMut<'_>) -> Result<T>,
    {
        match txn {
            Some(txn) => {
                let inner = txn.get_inner();
                let mut txn = inner.borrow_mut();
                let txn = txn.deref_mut();
                let shared_ref = self.resolve(txn)?;
                f(&shared_ref, txn)
            }
            None => {
                let mut txn = self.transact_mut()?;
                let shared_ref = self.resolve(&mut txn)?;
                f(&shared_ref, &mut txn)
            }
        }
    }

    pub fn resolve<T: ReadTxn>(&self, txn: &T) -> Result<S> {
        match self.hook.get(txn) {
            Some(shared_ref) => Ok(shared_ref),
            None => Err(Error::RefDisposed),
        }
    }

    pub fn transact(&self) -> Result<Transaction> {
        match self.doc.try_transact() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(Error::AnotherRwTx),
        }
    }

    pub fn transact_mut(&self) -> Result<TransactionMut> {
        match self.doc.try_transact_mut() {
            Ok(tx) => Ok(tx),
            Err(_) => Err(Error::AnotherTx),
        }
    }
}
