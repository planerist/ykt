//! Update utilities that work on encoded updates alone, without a document (Yjs's `mergeUpdates`,
//! `diffUpdate`, `encodeStateVectorFromUpdate`): a server or a client's outbox can compact stored
//! updates, or answer a replica's sync request, without loading the document.
//!
//! State vectors here are always in lib0 v1 encoding, whatever the update's encoding — as in Yjs,
//! where both encodings of a state vector are the same bytes. They are what `encodeStateVector(doc)`
//! and `YTransaction.stateVectorV1()` return, and what `decodeStateVector` reads.
//!
//! Every function checks its input: bytes that aren't a valid update or state vector fail with
//! `InvalidData`, they never crash the process.

use crate::tools::{Error, Result};
use std::panic::{catch_unwind, AssertUnwindSafe};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::StateVector;

/// Merges updates (lib0 v1) into one update (lib0 v1), as Yjs's `mergeUpdates`: applying the result
/// to a document is the same as applying every update in turn, in any order. An empty list gives
/// an empty update.
#[uniffi::export]
pub fn merge_updates_v1(updates: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    decoding(|| yrs::merge_updates_v1(&updates))
}

/// As `merge_updates_v1`, for updates in lib0 v2 encoding (Yjs's `mergeUpdatesV2`).
#[uniffi::export]
pub fn merge_updates_v2(updates: Vec<Vec<u8>>) -> Result<Vec<u8>> {
    decoding(|| yrs::merge_updates_v2(&updates))
}

/// What of `update` (lib0 v1) a replica with `state_vector` (lib0 v1) doesn't have yet, as an
/// update (lib0 v1): Yjs's `diffUpdate`. Deletions are always included (a state vector doesn't
/// cover them).
#[uniffi::export]
pub fn diff_update_v1(update: Vec<u8>, state_vector: Vec<u8>) -> Result<Vec<u8>> {
    decoding(|| yrs::diff_updates_v1(&update, &state_vector))
}

/// As `diff_update_v1`, for an update in lib0 v2 encoding (Yjs's `diffUpdateV2`): the result is
/// lib0 v2 too; the state vector is lib0 v1, as everywhere here.
#[uniffi::export]
pub fn diff_update_v2(update: Vec<u8>, state_vector: Vec<u8>) -> Result<Vec<u8>> {
    decoding(|| {
        // yrs reads the state vector of a v2 diff in its own v2 form: hand it that.
        let sv = StateVector::decode_v1(&state_vector)?;
        yrs::diff_updates_v2(&update, &sv.encode_v2())
    })
}

/// The state vector (lib0 v1) of a document that has exactly `update` (lib0 v1): Yjs's
/// `encodeStateVectorFromUpdate`. It equals `encodeStateVector` of a fresh document after applying
/// the update.
#[uniffi::export]
pub fn encode_state_vector_from_update_v1(update: Vec<u8>) -> Result<Vec<u8>> {
    decoding(|| yrs::encode_state_vector_from_update_v1(&update))
}

/// As `encode_state_vector_from_update_v1`, for an update in lib0 v2 encoding (Yjs's
/// `encodeStateVectorFromUpdateV2`); the state vector is lib0 v1, as everywhere here.
#[uniffi::export]
pub fn encode_state_vector_from_update_v2(update: Vec<u8>) -> Result<Vec<u8>> {
    decoding(|| {
        let update = yrs::Update::decode_v2(&update)?;
        Ok(update.state_vector().encode_v1())
    })
}

/// Runs a decoding step on bytes from outside: a decoding error, or a panic inside yrs on malformed
/// input (its v2 decoder has some), becomes `InvalidData`.
pub(crate) fn decoding<T, F>(f: F) -> Result<T>
where
    F: FnOnce() -> std::result::Result<T, yrs::encoding::read::Error>,
{
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(e)) => Err(Error::InvalidData(e.to_string())),
        Err(_) => Err(Error::InvalidData("malformed input".into())),
    }
}
