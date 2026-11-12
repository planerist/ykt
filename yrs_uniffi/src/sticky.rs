//! Relative positions (yrs's `StickyIndex`, Yjs's `RelativePosition`): a place in a text that keeps
//! its spot through local and remote edits — a caret, the ends of a selection, a comment's range.
//!
//! `YText.stickyIndex(index, assoc)` makes one; `YStickyIndex.offset(doc)` turns it back into an
//! index in the document as it is now. `encode()` / `YStickyIndex.decode(bytes)` carry it to another
//! replica or into storage, in Yjs's encoding (`Y.encodeRelativePosition`, lib0 v1), so positions
//! made by Yjs clients can be read here and the other way round.

use crate::collection::{same_doc, ReadView};
use crate::doc::YDoc;
use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use crate::updates::decoding;
use std::fmt::{Display, Formatter};
use std::ops::Deref;
use std::sync::Arc;
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Assoc, IndexScope, StickyIndex, Transact};

/// Which neighbour a position sticks to, so where an insert made exactly at the position goes.
#[derive(uniffi::Enum, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YAssoc {
    /// Sticks to what is before it (Yjs's `assoc < 0`): an insert exactly there goes after the
    /// position, which stays where it was.
    Before,
    /// Sticks to what is after it (Yjs's `assoc >= 0`, its default): an insert exactly there goes
    /// before the position, which moves on with the text after it.
    After,
}

impl From<YAssoc> for Assoc {
    fn from(value: YAssoc) -> Self {
        match value {
            YAssoc::Before => Assoc::Before,
            YAssoc::After => Assoc::After,
        }
    }
}

impl From<Assoc> for YAssoc {
    fn from(value: Assoc) -> Self {
        match value {
            Assoc::Before => YAssoc::Before,
            Assoc::After => YAssoc::After,
        }
    }
}

/// Where a relative position is now: an index (UTF-16 units in a text) and the position's `assoc`.
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct YStickyOffset {
    pub index: u32,
    pub assoc: YAssoc,
}

/// A relative position: a place between two items of a collection, tied to the items rather than
/// to an index (see the module docs). Immutable; equal positions compare equal.
#[derive(uniffi::Object, Clone, Debug, PartialEq, Eq, Hash)]
#[uniffi::export(Debug, Display, Eq, Hash)]
pub struct YStickyIndex(pub(crate) StickyIndex);

impl YStickyIndex {
    /// The position at `index` of `branch`, as Yjs's `createRelativePositionFromTypeIndex`: `None`
    /// past the end. At the end with `After` yrs gives none either; Yjs gives a position tied to
    /// the collection itself, which stays at its end: so does this.
    pub(crate) fn at<B: AsRef<yrs::branch::Branch> + yrs::IndexedSequence>(
        branch: &B,
        view: &ReadView<'_>,
        index: u32,
        len: u32,
        assoc: YAssoc,
    ) -> Result<Self> {
        if index > len {
            return Err(Error::OutOfBounds);
        }
        match branch.sticky_index(view, index, assoc.into()) {
            Some(sticky) => Ok(YStickyIndex(sticky)),
            None if index == len && assoc == YAssoc::After => {
                let scope = IndexScope::from_branch(yrs::branch::BranchPtr::from(branch.as_ref()));
                Ok(YStickyIndex(StickyIndex::new(scope, Assoc::After)))
            }
            None => Err(Error::OutOfBounds),
        }
    }
}

#[uniffi::export]
impl YStickyIndex {
    /// Reads a position written by `encode()` (or by Yjs's `encodeRelativePosition`). Bytes that
    /// aren't one throw `InvalidData`.
    #[uniffi::constructor]
    pub fn decode(bytes: Vec<u8>) -> Result<Self> {
        decoding(|| StickyIndex::decode_v1(&bytes)).map(YStickyIndex)
    }

    /// The position in Yjs's encoding (`encodeRelativePosition`, lib0 v1), to send or store.
    pub fn encode(&self) -> Vec<u8> {
        self.0.encode_v1()
    }

    pub fn assoc(&self) -> YAssoc {
        self.0.assoc.into()
    }

    /// Where the position is in `doc` now (Yjs's `createAbsolutePositionFromRelativePosition`),
    /// read in `txn` if given (a transaction of `doc`), else in a read-only transaction of its
    /// own. `null` when `doc` doesn't have what the position refers to (yet: it was made on a
    /// replica whose changes haven't arrived). A position whose neighbour was deleted collapses to
    /// where the deleted part was. The index is in the collection the position was made in.
    #[uniffi::method(default(txn = None))]
    pub fn offset(&self, doc: &YDoc, txn: Option<Arc<YTransaction>>) -> Result<Option<YStickyOffset>> {
        let offset = match txn {
            Some(txn) => {
                let inner = txn.get_inner();
                let txn = inner.borrow();
                if txn.committed {
                    return Err(Error::TxnCommitted);
                }
                if !same_doc(txn.doc(), doc.deref()) {
                    return Err(Error::Custom("the transaction belongs to another document".into()));
                }
                self.0.get_offset(&ReadView(yrs::ReadTxn::store(txn.deref())))
            }
            None => {
                let txn = doc.try_transact().map_err(|_| Error::AnotherRwTx)?;
                self.0.get_offset(&ReadView(yrs::ReadTxn::store(&txn)))
            }
        };
        Ok(offset.map(|o| YStickyOffset { index: o.index, assoc: o.assoc.into() }))
    }
}

impl Display for YStickyIndex {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let assoc = match self.0.assoc {
            Assoc::Before => "before",
            Assoc::After => "after",
        };
        match self.0.scope() {
            IndexScope::Relative(id) => write!(f, "YStickyIndex({assoc} {}#{})", id.client, id.clock),
            IndexScope::Nested(id) => write!(f, "YStickyIndex({assoc}, in {}#{})", id.client, id.clock),
            IndexScope::Root(name) => write!(f, "YStickyIndex({assoc}, in \"{name}\")"),
        }
    }
}
