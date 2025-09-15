use crate::tools;
use crate::tools::Error;
use crate::tools::Result;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::block::ClientID;
use yrs::error::UpdateError;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{ReadTxn, StateVector, TransactionMut, Update};

pub struct YTransactionInner {
    // SAFETY NOTE: We erase the lifetime of TransactionMut to 'static below and rely on the
    // embedding application's guarantee that there are no concurrent calls into this library,
    // and that any given YTransaction is only used from one thread at a time (although the
    // thread may change between calls). Misuse outside those guarantees can cause UB.
    pub inner: ManuallyDrop<TransactionMut<'static>>,
    // pub cached_before_state: Option<PyObject>,
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
            self.commit();
        }
    }
}

impl YTransactionInner {
    pub fn new(txn: TransactionMut<'static>) -> Self {
        YTransactionInner {
            inner: ManuallyDrop::new(txn),
            // cached_before_state: None,
            committed: false,
        }
    }
}

impl YTransactionInner {
    // pub fn before_state(&mut self) -> PyObject {
    //     if self.cached_before_state.is_none() {
    //         let before_state = Python::with_gil(|py| {
    //             let txn = (*self).deref();
    //             let state_map: HashMap<u64, u32> =
    //                 txn.before_state().iter().map(|(x, y)| (*x, *y)).collect();
    //             state_map.into_py(py)
    //         });
    //         self.cached_before_state = Some(before_state);
    //     }
    //     return self.cached_before_state.as_ref().unwrap().clone();
    // }

    /// Triggers a post-update series of operations without `free`ing the transaction. This includes
    /// compaction and optimization of internal representation of updates, triggering events etc.
    /// Ypy transactions are auto-committed when they are `free`d.
    pub fn commit(&mut self) -> Result<()> {
        if !self.committed {
            self.deref_mut().commit();
            self.committed = true;
            unsafe { ManuallyDrop::drop(&mut self.inner) };

            Ok(())
        } else {
            Err(Error::TxnCommitted)
        }
    }
}

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
    pub fn get_inner(&self) -> Arc<RefCell<YTransactionInner>> {
        self.inner.clone()
    }

    fn try_apply(&self, update: Update) -> Result<()> {
        self.get_inner()
            .borrow_mut()
            .apply_update(update)
            .map_err(|e| match e {
                UpdateError::InvalidParent(_, _) => tools::Error::InvalidParent,
            })
    }
}

#[uniffi::export]
impl YTransaction {
    /// Returns state vector describing the state of the document
    /// at the moment when the transaction began.
    pub fn before_state(&self) -> HashMap<ClientID, u32> {
        self.get_inner()
            .borrow()
            .before_state()
            .iter()
            .map(|(x, y)| (*x, *y))
            .collect()
    }

    /// Returns state vector describing the current state of
    /// the document.
    pub fn after_state(&self) -> HashMap<ClientID, u32> {
        let state_map: HashMap<u64, u32> = self
            .get_inner()
            .borrow()
            .after_state()
            .iter()
            .map(|(x, y)| (*x, *y))
            .collect();

        state_map
    }

    pub fn origin(&self) -> Option<Vec<u8>> {
        let inner = self.get_inner();
        let inner = inner.borrow();
        let origin = inner.origin()?;
        Some(origin.as_ref().to_vec())
    }

    /// Triggers a post-update series of operations without `free`ing the transaction. This includes
    /// compaction and optimization of internal representation of updates, triggering events etc.
    /// ywasm transactions are auto-committed when they are `free`d.
    #[uniffi::method]
    pub fn commit(&self) -> Result<()> {
        self.get_inner().borrow_mut().commit()?;
        Ok(())
    }

    /// Encodes a state vector of a given transaction document into its binary representation using
    /// lib0 v1 encoding. State vector is a compact representation of updates performed on a given
    /// document and can be used by `encode_state_as_update` on remote peer to generate a delta
    /// update payload to synchronize changes between peers.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    pub fn state_vector_v1(&self) -> Vec<u8> {
        let sv = self.get_inner().borrow().state_vector();
        sv.encode_v1()
    }

    pub fn state_vector_v2(&self) -> Vec<u8> {
        let sv = self.get_inner().borrow().state_vector();
        sv.encode_v2()
    }

    /// Encodes all updates that have happened since a given version `vector` into a compact delta
    /// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
    /// delta payload will contain all changes of a current ywasm document, working effectively as
    /// its state snapshot.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    pub fn diff_v1(&self, vector: Vec<u8>) -> Result<Vec<u8>> {
        match StateVector::decode_v1(vector.to_vec().as_slice()) {
            Ok(sv) => Ok(self.get_inner().borrow().encode_diff_v1(&sv)),
            Err(e) => Err(tools::Error::InvalidData(e.to_string())),
        }
    }

    /// Encodes all updates that have happened since a given version `vector` into a compact delta
    /// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
    /// delta payload will contain all changes of a current ywasm document, working effectively as
    /// its state snapshot.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    pub fn diff_v2(&self, vector: Vec<u8>) -> Result<Vec<u8>> {
        match StateVector::decode_v1(vector.to_vec().as_slice()) {
            Ok(sv) => Ok(self.get_inner().borrow().encode_diff_v1(&sv)),
            Err(e) => Err(tools::Error::InvalidData(e.to_string())),
        }
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v1 encoding format.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV1(localSv)
    ///     localTxn.applyV1(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    pub fn apply_v1(&self, diff: Vec<u8>) -> Result<()> {
        match Update::decode_v1(diff.as_slice()) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(tools::Error::InvalidData(e.to_string())),
        }
    }

    /// Applies delta update generated by the remote document replica to a current transaction's
    /// document. This method assumes that a payload maintains lib0 v2 encoding format.
    ///
    /// Example:
    ///
    /// ```javascript
    /// import YDoc from 'ywasm'
    ///
    /// /// document on machine A
    /// const localDoc = new YDoc()
    /// const localTxn = localDoc.beginTransaction()
    ///
    /// // document on machine B
    /// const remoteDoc = new YDoc()
    /// const remoteTxn = localDoc.beginTransaction()
    ///
    /// try {
    ///     const localSV = localTxn.stateVectorV1()
    ///     const remoteDelta = remoteTxn.diffV2(localSv)
    ///     localTxn.applyV2(remoteDelta)
    /// } finally {
    ///     localTxn.free()
    ///     remoteTxn.free()
    /// }
    /// ```
    pub fn apply_v2(&self, diff: Vec<u8>) -> Result<()> {
        match Update::decode_v2(diff.as_slice()) {
            Ok(update) => self.try_apply(update),
            Err(e) => Err(tools::Error::InvalidData(e.to_string())),
        }
    }

    pub fn encode_update(&self) -> Vec<u8> {
        self.get_inner().borrow().encode_update_v1()
    }

    pub fn encode_update_v2(&self) -> Vec<u8> {
        self.get_inner().borrow().encode_update_v2()
    }

    /// Force garbage collection of the deleted elements, regardless of a parent doc was created
    /// with `gc` option turned on or off.
    pub fn gc(&self) -> Result<()> {
        self.get_inner().borrow_mut().gc(None);
        Ok(())
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
