use crate::tools;
use thiserror::Error;

#[cfg(target_arch = "wasm32")]
pub(crate) fn is_wasm() -> bool {
    true
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn is_wasm() -> bool {
    false
}

#[derive(uniffi::Error, Error, Debug)]
pub(crate) enum Error {
    #[error("cannot modify transaction in this context")]
    InvalidTransactionCtx,
    #[error("shared collection has been destroyed")]
    RefDisposed,
    #[error("transaction is already committed")]
    TxnCommitted,
    #[error("another transaction is in progress")]
    AnotherTx,
    #[error("another read-write transaction is in progress")]
    AnotherRwTx,
    // OutOfBounds, //"index outside of the bounds of an array";
    // KeyNotFound, //"key was not found in a map";
    #[error("preliminary type doesn't support this operation")]
    InvalidPrelimOp,
    #[error("given object cannot be used as formatting attributes")]
    InvalidFmt,
    // InvalidXmlAttrs, //"given object cannot be used as XML attributes";
    // NotXmlType, //"provided object is not a valid XML shared type";
    // NotPrelim, //"this operation only works on preliminary types";

    #[error("invalid delta format")]
    InvalidDelta,
    #[error("{0}")]
    Custom(String),

    #[error("Invalid data: {0}")]
    InvalidData(String),

    #[error("Invalid parent")]
    InvalidParent,
}

pub(crate) type Result<T> = std::result::Result<T, tools::Error>;
