use crate::tools::Error;
use crate::tools::Result;
use crate::updates::{decoding, DocEncoderV2};
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::{Encode, Encoder};
use yrs::{ReadTxn, StateVector, TransactionMut, Update};

pub struct YTransactionInner {
    // SAFETY NOTE: We erase the lifetime of TransactionMut to 'static below and rely on the
    // embedding application's guarantee that there are no concurrent calls into this library,
    // and that any given YTransaction is only used from one thread at a time (although the
    // thread may change between calls). Misuse outside those guarantees can cause UB.
    pub inner: ManuallyDrop<TransactionMut<'static>>,
    pub committed: bool,
}

impl ReadTxn for YTransactionInner {
    fn store(&self) -> &yrs::Store {
        self.deref().store()
    }
}

impl Deref for YTransactionInner {
    type Target = TransactionMut<'static>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for YTransactionInner {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl Drop for YTransactionInner {
    fn drop(&mut self) {
        if !self.committed {
            let _ = self.commit();
        }
    }
}

impl YTransactionInner {
    pub fn new(txn: TransactionMut<'static>) -> Self {
        YTransactionInner {
            inner: ManuallyDrop::new(txn),
            committed: false,
        }
    }

    /// Commits the transaction (its observers run now) and releases the document's lock.
    pub fn commit(&mut self) -> Result<()> {
        if !self.committed {
            self.deref_mut().commit();
            self.committed = true;
            unsafe { ManuallyDrop::drop(&mut self.inner) };

            Ok(())
        } else {
            Err(Error::TransactionCommitted)
        }
    }
}

/// A read-write transaction of a document (`YDoc.transaction(origin)`): every change made in it
/// reaches the observers at once, as one event, when it commits — `commit()`, or when it is
/// released. Every method throws `TransactionCommitted` after the commit, and `Locked` while the
/// transaction is committing (inside its own observers' callbacks).
#[derive(uniffi::Object)]
pub struct YTransaction {
    inner: Arc<RefCell<YTransactionInner>>,
}

// SAFETY: The outer Arc<RefCell<...>> is not Send nor Sync by default. We provide
// these impls relying on the embedding application's guarantees:
// - Calls into this library are not concurrent.
// - A given object is only used by one thread at a time (though the thread may change between calls).
// Violating these guarantees can cause undefined behavior.
unsafe impl Sync for YTransaction {}
unsafe impl Send for YTransaction {}

impl YTransaction {
    /// Runs `f` with the open transaction: `TransactionCommitted` after the commit, `Locked` while
    /// it is in use (committing: inside its callbacks).
    pub(crate) fn read<R>(&self, f: impl FnOnce(&TransactionMut<'static>) -> Result<R>) -> Result<R> {
        let inner = self.inner.try_borrow().map_err(|_| Error::Locked)?;
        if inner.committed {
            return Err(Error::TransactionCommitted);
        }
        f(&inner)
    }

    /// As `read`, to write in the transaction.
    pub(crate) fn write<R>(&self, f: impl FnOnce(&mut TransactionMut<'static>) -> Result<R>) -> Result<R> {
        let mut inner = self.inner.try_borrow_mut().map_err(|_| Error::Locked)?;
        if inner.committed {
            return Err(Error::TransactionCommitted);
        }
        f(&mut inner)
    }

    fn apply(&self, update: Update) -> Result<()> {
        self.write(|txn| txn.apply_update(update).map_err(|e| Error::InvalidData(e.to_string())))
    }
}

#[uniffi::export]
impl YTransaction {
    /// The document's state vector (client id → clock) when the transaction began.
    pub fn before_state(&self) -> Result<HashMap<u64, u32>> {
        self.read(|txn| Ok(txn.before_state().iter().map(|(x, y)| (x.get(), *y)).collect()))
    }

    /// The document's state vector now, with the transaction's changes so far.
    pub fn after_state(&self) -> Result<HashMap<u64, u32>> {
        self.read(|txn| Ok(txn.after_state().iter().map(|(x, y)| (x.get(), *y)).collect()))
    }

    /// The origin the transaction was opened with (`YDoc.transaction(origin)`), as bytes.
    pub fn origin(&self) -> Result<Option<Vec<u8>>> {
        self.read(|txn| Ok(txn.origin().map(|origin| origin.as_ref().to_vec())))
    }

    /// Commits the transaction: its changes reach the observers now, and the document is unlocked.
    /// Throws `TransactionCommitted` the second time.
    #[uniffi::method]
    pub fn commit(&self) -> Result<()> {
        self.inner.try_borrow_mut().map_err(|_| Error::Locked)?.commit()
    }

    /// The document's state vector (lib0 v1): what a replica sends to ask for what it lacks
    /// (`diffV1` / `diffV2` on the other side).
    pub fn state_vector_v1(&self) -> Result<Vec<u8>> {
        self.read(|txn| Ok(txn.state_vector().encode_v1()))
    }

    /// The document's state vector in lib0 v2 encoding.
    pub fn state_vector_v2(&self) -> Result<Vec<u8>> {
        self.read(|txn| Ok(txn.state_vector().encode_v2()))
    }

    /// What a replica with state `vector` (lib0 v1) lacks, as an update in lib0 v1 encoding.
    /// Malformed bytes throw `InvalidData`.
    pub fn diff_v1(&self, vector: Vec<u8>) -> Result<Vec<u8>> {
        let sv = decoding(|| StateVector::decode_v1(&vector))?;
        self.read(|txn| Ok(txn.encode_diff_v1(&sv)))
    }

    /// As `diff_v1`, the update in lib0 v2 encoding (the state vector is lib0 v1).
    pub fn diff_v2(&self, vector: Vec<u8>) -> Result<Vec<u8>> {
        let sv = decoding(|| StateVector::decode_v1(&vector))?;
        self.read(|txn| {
            let mut encoder = DocEncoderV2::new();
            txn.encode_diff(&sv, &mut encoder);
            Ok(encoder.to_vec())
        })
    }

    /// Applies a replica's update (lib0 v1) in this transaction. Malformed bytes throw
    /// `InvalidData`.
    pub fn apply_v1(&self, diff: Vec<u8>) -> Result<()> {
        self.apply(decoding(|| Update::decode_v1(&diff))?)
    }

    /// Applies a replica's update (lib0 v2) in this transaction. Malformed bytes throw
    /// `InvalidData`.
    pub fn apply_v2(&self, diff: Vec<u8>) -> Result<()> {
        self.apply(decoding(|| Update::decode_v2(&diff))?)
    }

    /// The transaction's changes so far as an update (lib0 v1).
    pub fn encode_update(&self) -> Result<Vec<u8>> {
        self.read(|txn| Ok(txn.encode_update_v1()))
    }

    /// The transaction's changes so far as an update (lib0 v2).
    pub fn encode_update_v2(&self) -> Result<Vec<u8>> {
        self.read(|txn| {
            let mut encoder = DocEncoderV2::new();
            txn.encode_update(&mut encoder);
            Ok(encoder.to_vec())
        })
    }

    /// Force garbage collection of the deleted elements, regardless of a parent doc was created
    /// with `gc` option turned on or off.
    pub fn gc(&self) -> Result<()> {
        self.write(|txn| {
            txn.gc(None);
            Ok(())
        })
    }
}

impl<'doc> From<TransactionMut<'doc>> for YTransaction {
    fn from(value: TransactionMut<'doc>) -> Self {
        // SAFETY: We extend the lifetime of TransactionMut here based on external guarantees.
        // See the note above YTransactionInner and the unsafe Send/Sync impls for details.
        let txn: TransactionMut<'static> = unsafe { std::mem::transmute(value) };
        YTransaction {
            inner: Arc::new(RefCell::new(YTransactionInner::new(txn))),
        }
    }
}
