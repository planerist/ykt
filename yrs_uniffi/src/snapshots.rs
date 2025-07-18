use crate::doc::YDoc;
use crate::tools;
use crate::tools::Error;
use crate::tools::Result;
use std::ops::Deref;
use std::sync::Arc;
use yrs::updates::decoder::{Decode, DecoderV1};
use yrs::updates::encoder::{Encode, Encoder, EncoderV1, EncoderV2};
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
    let txn = doc.0.try_transact().map_err(|_| Error::AnotherRwTx)?;
    let bytes = txn.state_vector().encode_v1();
    Ok(bytes)
}

#[uniffi::export]
pub fn encode_state_vector2(doc: &YDoc) -> Result<Vec<u8>> {
    let txn = doc.0.try_transact().map_err(|_| Error::AnotherRwTx)?;
    let bytes = txn.state_vector().encode_v2();
    Ok(bytes)
}

#[uniffi::export(default(vector=None))]
fn decode_state_vector(vector: Option<Vec<u8>>) -> Result<YStateVector> {
    if let Some(v) = vector {
        match StateVector::decode_v1(v.as_slice()) {
            Ok(sv) => Ok(YStateVector(sv)),
            Err(e) => Err(Error::InvalidData(e.to_string())),
        }
    } else {
        Ok(YStateVector(StateVector::default()))
    }
}

#[uniffi::export(default(vector=None))]
fn decode_state_vector2(vector: Option<Vec<u8>>) -> Result<YStateVector> {
    if let Some(v) = vector {
        match StateVector::decode_v2(v.as_slice()) {
            Ok(sv) => Ok(YStateVector(sv)),
            Err(e) => Err(Error::InvalidData(e.to_string())),
        }
    } else {
        Ok(YStateVector(StateVector::default()))
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v1 encoding.
#[uniffi::export]
pub fn debug_update_v1(update: &[u8]) -> Result<String> {
    let mut decoder = DecoderV1::from(update);
    match Update::decode(&mut decoder) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(Error::InvalidData(e.to_string())),
    }
}

/// Returns a string dump representation of a given `update` encoded using lib0 v2 encoding.
#[uniffi::export]
pub fn debug_update_v2(update: &[u8]) -> Result<String> {
    match Update::decode_v2(update) {
        Ok(update) => Ok(format!("{:#?}", update)),
        Err(e) => Err(Error::InvalidData(e.to_string())),
    }
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
    let txn = doc.0.try_transact().map_err(|_| Error::AnotherRwTx)?;
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
    let txn = doc.0.try_transact().map_err(|_| Error::AnotherRwTx)?;
    let sv = if let Some(vector) = vector {
        &vector.clone().0
    } else {
        &StateVector::default()
    };
    let bytes = txn.encode_state_as_update_v2(sv);
    Ok(bytes)
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
    .map_err(|_| Error::AnotherRwTx)?;

    match Update::decode_v1(update) {
        Ok(update) => txn
            .apply_update(update)
            .map_err(|e| Error::InvalidData(e.to_string())),
        Err(e) => Err(Error::InvalidData(e.to_string())),
    }
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
    .map_err(|_| Error::AnotherRwTx)?;

    match Update::decode_v2(update) {
        Ok(update) => txn
            .apply_update(update)
            .map_err(|e| tools::Error::InvalidData(e.to_string())),
        Err(e) => Err(tools::Error::InvalidData(e.to_string())),
    }
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

#[uniffi::export]
pub fn snapshot(doc: &YDoc) -> Arc<YSnapshot> {
    let snapshot = doc.0.transact().snapshot();
    Arc::new(YSnapshot(snapshot))
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
    let snap = yrs::Snapshot::decode_v1(snapshot).map_err(|_| {
        Error::InvalidData("failed to deserialize snapshot using lib0 v2 decoding".into())
    })?;
    Ok(YSnapshot(snap))
}

#[uniffi::export]
pub fn encode_snapshot_v2(snapshot: &YSnapshot) -> Vec<u8> {
    snapshot.0.encode_v2()
}

#[uniffi::export]
pub fn decode_snapshot_v2(snapshot: &[u8]) -> Result<YSnapshot> {
    let snap = yrs::Snapshot::decode_v2(snapshot).map_err(|_| {
        Error::InvalidData("failed to deserialize snapshot using lib0 v2 decoding".into())
    })?;
    Ok(YSnapshot(snap))
}

#[uniffi::export]
pub fn encode_state_from_snapshot_v1(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>> {
    let mut encoder = EncoderV1::new();
    match doc
        .0
        .transact()
        .encode_state_from_snapshot(&snapshot.0, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(Error::InvalidData(e.to_string())),
    }
}

#[uniffi::export]
pub fn encode_state_from_snapshot_v2(doc: &YDoc, snapshot: &YSnapshot) -> Result<Vec<u8>> {
    let mut encoder = EncoderV2::new();
    match doc
        .0
        .transact()
        .encode_state_from_snapshot(&snapshot.0, &mut encoder)
    {
        Ok(_) => Ok(encoder.to_vec()),
        Err(e) => Err(Error::InvalidData(e.to_string())),
    }
}
