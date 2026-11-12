use crate::doc::YDoc;
use crate::tools::Error;
use crate::tools::Result;
use crate::updates::{decoding, state_as_update_v2, DocEncoderV2};
use std::ops::Deref;
use std::sync::Arc;
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1};
use yrs::{ReadTxn, StateVector, Transact, Update};

#[derive(uniffi::Object)]
#[repr(transparent)]
pub struct YStateVector(pub(crate) StateVector);

/// Encodes a state vector of a given ywasm document into its binary representation using lib0 v1
/// encoding. State vector is a compact representation of updates performed on a given document and
/// can be used by `encode_state_as_update` on remote peer to generate a delta update payload to
/// synchronize changes between peers.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[uniffi::export]
pub fn encode_state_vector(doc: &YDoc) -> Result<Vec<u8>> {
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    let bytes = txn.state_vector().encode_v1();
    Ok(bytes)
}

#[uniffi::export]
pub fn encode_state_vector2(doc: &YDoc) -> Result<Vec<u8>> {
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    let bytes = txn.state_vector().encode_v2();
    Ok(bytes)
}

#[uniffi::export(default(vector=None))]
pub fn decode_state_vector(vector: Option<Vec<u8>>) -> Result<YStateVector> {
    if let Some(v) = vector {
        decoding(|| StateVector::decode_v1(&v)).map(YStateVector)
    } else {
        Ok(YStateVector(StateVector::default()))
    }
}

#[uniffi::export(default(vector=None))]
pub fn decode_state_vector2(vector: Option<Vec<u8>>) -> Result<YStateVector> {
    if let Some(v) = vector {
        decoding(|| StateVector::decode_v2(&v)).map(YStateVector)
    } else {
        Ok(YStateVector(StateVector::default()))
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v1 encoding.
#[uniffi::export]
pub fn debug_update_v1(update: &[u8]) -> Result<String> {
    let update = decoding(|| Update::decode(&mut DecoderV1::from(update)))?;
    Ok(format!("{:#?}", update))
}

/// Returns a string dump representation of a given `update` encoded using lib0 v2 encoding.
#[uniffi::export]
pub fn debug_update_v2(update: &[u8]) -> Result<String> {
    let update = decoding(|| Update::decode_v2(update))?;
    Ok(format!("{:#?}", update))
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v1 encoding. If `vector` parameter has not been provided, generated
/// delta payload will contain all changes of a current ywasm document, working effectivelly as its
/// state snapshot.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[uniffi::export(default(vector=None))]
pub fn encode_state_as_update(doc: &YDoc, vector: Option<Arc<YStateVector>>) -> Result<Vec<u8>> {
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    let sv = if let Some(vector) = vector {
        &vector.clone().0
    } else {
        &StateVector::default()
    };
    let bytes = txn.encode_state_as_update_v1(sv);
    Ok(bytes)
}

/// Encodes all updates that have happened since a given version `vector` into a compact delta
/// representation using lib0 v2 encoding. If `vector` parameter has not been provided, generated
/// delta payload will contain all changes of a current ywasm document, working effectivelly as its
/// state snapshot.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdate(localDoc, remoteDelta)
/// ```
#[uniffi::export(default(vector=None))]
pub fn encode_state_as_update_v2(doc: &YDoc, vector: Option<Arc<YStateVector>>) -> Result<Vec<u8>> {
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    let sv = if let Some(vector) = vector {
        &vector.clone().0
    } else {
        &StateVector::default()
    };
    // yrs's own writer fails on a document with updates out of order (`DocEncoderV2`).
    state_as_update_v2(&txn, sv)
}


/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v1 encoding format.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdate(remoteDoc, localSV)
///
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[uniffi::export(default(origin=None))]
pub fn apply_update(doc: &YDoc, update: &[u8], origin: Option<Vec<u8>>) -> Result<()> {
    let mut txn = if let Some(origin) = origin {
        doc.0.try_transact_mut_with(origin.as_slice())
    } else {
        doc.0.try_transact_mut()
    }
    .map_err(|_| Error::Locked)?;

    let update = decoding(|| Update::decode_v1(update))?;
    txn.apply_update(update).map_err(|e| Error::InvalidData(e.to_string()))
}

/// Applies delta update generated by the remote document replica to a current document. This
/// method assumes that a payload maintains lib0 v2 encoding format.
///
/// Example:
///
/// ```javascript
/// import {YDoc, encodeStateVector, encodeStateAsUpdate, applyUpdate} from 'ywasm'
///
/// /// document on machine A
/// const localDoc = new YDoc()
/// const localSV = encodeStateVector(localDoc)
///
/// // document on machine B
/// const remoteDoc = new YDoc()
/// const remoteDelta = encodeStateAsUpdateV2(remoteDoc, localSV)
///
/// applyUpdateV2(localDoc, remoteDelta)
/// ```
#[uniffi::export(default(origin=None))]
pub fn apply_update_v2(doc: &YDoc, update: &[u8], origin: Option<Vec<u8>>) -> Result<()> {
    let mut txn = if let Some(origin) = origin {
        doc.0.try_transact_mut_with(origin.as_slice())
    } else {
        doc.0.try_transact_mut()
    }
    .map_err(|_| Error::Locked)?;

    let update = decoding(|| Update::decode_v2(update))?;
    txn.apply_update(update).map_err(|e| Error::InvalidData(e.to_string()))
}

#[derive(uniffi::Object)]
pub struct YSnapshot(yrs::Snapshot);

impl Deref for YSnapshot {
    type Target = yrs::Snapshot;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// The document's state now, to read a text as it was (`YText.toDelta(snapshot)`) or to encode that
/// state (`encodeStateFromSnapshot`; both need a document made with `gc = false`). Throws `Locked`
/// while the document is locked.
#[uniffi::export]
pub fn snapshot(doc: &YDoc) -> Result<Arc<YSnapshot>> {
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    Ok(Arc::new(YSnapshot(txn.snapshot())))
}

#[uniffi::export]
pub fn equal_snapshots(snap1: &YSnapshot, snap2: &YSnapshot) -> bool {
    snap1.0 == snap2.0
}

#[uniffi::export]
pub fn encode_snapshot_v1(snapshot: &YSnapshot) -> Vec<u8> {
    snapshot.0.encode_v1()
}

#[uniffi::export]
pub fn decode_snapshot_v1(snapshot: &[u8]) -> Result<YSnapshot> {
    decoding(|| yrs::Snapshot::decode_v1(snapshot)).map(YSnapshot)
}

#[uniffi::export]
pub fn encode_snapshot_v2(snapshot: &YSnapshot) -> Vec<u8> {
    snapshot.0.encode_v2()
}

#[uniffi::export]
pub fn decode_snapshot_v2(snapshot: &[u8]) -> Result<YSnapshot> {
    decoding(|| yrs::Snapshot::decode_v2(snapshot)).map(YSnapshot)
}

/// What `encode_state_from_snapshot` fails with: a document that collects garbage (the usual case),
/// or one that doesn't have the snapshot's state.
fn snapshot_error(e: yrs::error::Error) -> Error {
    match e {
        yrs::error::Error::Gc => Error::GcEnabled,
        e => Error::InvalidData(e.to_string()),
    }
}

/// The document's state at `snapshot` as an update (lib0 v1): what it was then. Needs a document
/// made with `gc = false` (else `GcEnabled`).
#[uniffi::export]
pub fn encode_state_from_snapshot_v1(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>> {
    let mut encoder = EncoderV1::new();
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    txn.encode_state_from_snapshot(&snapshot.0, &mut encoder).map_err(snapshot_error)?;
    Ok(encoder.to_vec())
}

/// As `encode_state_from_snapshot_v1`, in lib0 v2 encoding.
#[uniffi::export]
pub fn encode_state_from_snapshot_v2(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>> {
    let mut encoder = DocEncoderV2::new();
    let txn = doc.0.try_transact().map_err(|_| Error::Locked)?;
    txn.encode_state_from_snapshot(&snapshot.0, &mut encoder).map_err(snapshot_error)?;
    Ok(encoder.to_vec())
}
