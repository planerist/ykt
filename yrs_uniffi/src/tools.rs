use thiserror::Error;
use yrs::OffsetKind;

#[cfg(target_arch = "wasm32")]
pub(crate) fn offset_kind() -> OffsetKind {
    OffsetKind::Bytes
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn offset_kind() -> OffsetKind {
    OffsetKind::Utf16
}

/// Everything ykt's API throws: Swift `YError`, Kotlin `YException`. Each case carries a message
/// (Swift `message:`, Kotlin `message`), the text below plus details where there are any. See the
/// README's "Errors" for when each one happens.
#[derive(uniffi::Error, Error, Debug)]
#[uniffi(flat_error)]
pub enum YError {
    /// The document is locked by work in progress on this thread: a transaction is open, or a
    /// callback runs while one commits (also an undo manager or an awareness in the middle of a
    /// call or a callback of its own). ykt throws instead of waiting for a lock that can't come.
    #[error("the document is locked: a transaction is open or a callback is running")]
    Locked,
    /// The transaction was committed (`commit()`, or the end of `transact { }`): it can't be used
    /// any more.
    #[error("the transaction is already committed")]
    TransactionCommitted,
    /// A transaction, a shared type or an undo scope of another document was passed.
    #[error("the transaction or the shared type belongs to another document")]
    WrongDocument,
    /// The shared type was deleted from its document (a nested type its parent removed).
    #[error("the shared type was deleted from its document")]
    TypeDeleted,
    /// The shared type is preliminary — made with its constructor, not in a document yet — and the
    /// operation needs it in one (formatting, observing, positions, undo).
    #[error("the shared type isn't in a document yet: insert it into one first")]
    NotIntegrated,
    /// A shared type that is already in a document was given where a new (preliminary) one is
    /// needed: it can't be inserted a second time.
    #[error("the shared type is already in a document: make a new one to insert")]
    AlreadyIntegrated,
    /// An index or a range past the end of a text, an array or an XML node's children.
    #[error("index outside the bounds of the collection")]
    OutOfBounds,
    /// Formatting attributes that aren't a JSON object.
    #[error("formatting attributes must be a JSON object: {0}")]
    InvalidAttributes(String),
    /// Bytes that aren't what they should be: a malformed update, state vector, snapshot, relative
    /// position or awareness update, or one in the other encoding (v1 / v2).
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// The operation needs the document's history, which garbage collection drops: snapshots of a
    /// past state need a document made with `gc = false`.
    #[error("the document collects garbage: this needs a document made with gc = false")]
    GcEnabled,
}

/// ykt's error, as the Rust code names it.
pub(crate) type Error = YError;

pub(crate) type Result<T> = std::result::Result<T, YError>;

/// The byte offset of `index` UTF-16 units into `s` (a preliminary text's content: indexes count
/// UTF-16 units, as in an integrated one). `OutOfBounds` past the end or inside a character.
pub(crate) fn utf16_offset(s: &str, index: u32) -> Result<usize> {
    let mut units = 0u32;
    for (offset, c) in s.char_indices() {
        if units == index {
            return Ok(offset);
        }
        units += c.len_utf16() as u32;
        if units > index {
            return Err(Error::OutOfBounds);
        }
    }
    if units == index {
        Ok(s.len())
    } else {
        Err(Error::OutOfBounds)
    }
}

/// The length of `s` in UTF-16 units.
pub(crate) fn utf16_len(s: &str) -> u32 {
    s.encode_utf16().count() as u32
}

/// `OutOfBounds` unless `index .. index + length` lies within `0 .. len`.
pub(crate) fn check_range(index: u32, length: u32, len: u32) -> Result<()> {
    match index.checked_add(length) {
        Some(end) if end <= len => Ok(()),
        _ => Err(Error::OutOfBounds),
    }
}
