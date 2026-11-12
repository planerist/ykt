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
use yrs::encoding::write::Write;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::{Encode, Encoder, EncoderV2};
use yrs::{ReadTxn, StateVector};

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

/// yrs's lib0 v2 encoder, for writing a document's blocks: every v2 encoding of a document
/// (`encodeStateAsUpdateV2`, a transaction's update and diff, update events, snapshots) goes through
/// it.
///
/// A client's update that arrives before that client's earlier ones is integrated by yrs 0.28 when
/// nothing else it needs is missing, with a gap (a skip) in the client's blocks. yrs's document
/// writer puts a skip's length in v2's length column, where every v2 reader — yrs's, Yjs's — looks
/// for it in the rest, so such a document's v2 bytes didn't read (`EndOfBuffer`, `UnexpectedValue`),
/// and yrs's `encode_state_as_update_v2` panicked merging them with its pending updates. This writes
/// a skip's length where readers look, as yrs's update writer and Yjs do; everything else is
/// `EncoderV2`'s.
pub(crate) struct DocEncoderV2 {
    inner: EncoderV2,
    /// The last struct written was a skip: its length comes next.
    skip: bool,
}

impl DocEncoderV2 {
    pub(crate) fn new() -> Self {
        DocEncoderV2 { inner: EncoderV2::new(), skip: false }
    }
}

impl Write for DocEncoderV2 {
    fn write_all(&mut self, buf: &[u8]) {
        self.skip = false;
        self.inner.write_all(buf)
    }

    fn write_u8(&mut self, value: u8) {
        self.skip = false;
        self.inner.write_u8(value)
    }

    fn write_string(&mut self, str: &str) {
        self.skip = false;
        self.inner.write_string(str)
    }
}

impl Encoder for DocEncoderV2 {
    fn to_vec(self) -> Vec<u8> {
        self.inner.to_vec()
    }

    fn reset_ds_cur_val(&mut self) {
        self.skip = false;
        self.inner.reset_ds_cur_val()
    }

    fn write_ds_clock(&mut self, clock: u32) {
        self.skip = false;
        self.inner.write_ds_clock(clock)
    }

    fn write_ds_len(&mut self, len: u32) {
        self.skip = false;
        self.inner.write_ds_len(len)
    }

    fn write_left_id(&mut self, id: &yrs::ID) {
        self.skip = false;
        self.inner.write_left_id(id)
    }

    fn write_right_id(&mut self, id: &yrs::ID) {
        self.skip = false;
        self.inner.write_right_id(id)
    }

    fn write_client(&mut self, client: yrs::block::ClientID) {
        self.skip = false;
        self.inner.write_client(client)
    }

    fn write_info(&mut self, info: u8) {
        self.inner.write_info(info);
        self.skip = info == yrs::block::BLOCK_SKIP_REF_NUMBER;
    }

    fn write_parent_info(&mut self, is_y_key: bool) {
        self.skip = false;
        self.inner.write_parent_info(is_y_key)
    }

    fn write_type_ref(&mut self, info: u8) {
        self.skip = false;
        self.inner.write_type_ref(info)
    }

    fn write_len(&mut self, len: u32) {
        if std::mem::take(&mut self.skip) {
            // A skip's length: in the rest, where readers look for it.
            self.inner.write_var(len)
        } else {
            self.inner.write_len(len)
        }
    }

    fn write_any(&mut self, any: &yrs::Any) {
        self.skip = false;
        self.inner.write_any(any)
    }

    fn write_json(&mut self, any: &yrs::Any) {
        self.skip = false;
        self.inner.write_json(any)
    }

    fn write_key(&mut self, string: &str) {
        self.skip = false;
        self.inner.write_key(string)
    }
}

/// A document's state from `sv` on in lib0 v2, its pending updates included, as yrs's
/// `encode_state_as_update_v2` — through `DocEncoderV2`.
pub(crate) fn state_as_update_v2<T: ReadTxn>(txn: &T, sv: &StateVector) -> Result<Vec<u8>> {
    let mut encoder = DocEncoderV2::new();
    txn.encode_state_as_update(sv, &mut encoder);
    let state = encoder.to_vec();
    if !txn.has_missing_updates() {
        return Ok(state);
    }
    // The pending updates, by yrs: from the document's own state vector on it writes no blocks of
    // its own, so no skip, only its delete set and what is pending.
    let pending = txn.encode_state_as_update_v2(&txn.state_vector());
    let merged = decoding(|| Ok(yrs::Update::merge_updates([yrs::Update::decode_v2(&state)?, yrs::Update::decode_v2(&pending)?])))?;
    Ok(merged.encode_v2())
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
