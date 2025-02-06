use crate::attrs::parse_attrs;
use crate::collection::SharedCollection;
use crate::tools::Error;
use crate::tools::Result;
use crate::transaction::YTransaction;
use std::cell::RefCell;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::types::{Delta, TYPE_REFS_TEXT};
use yrs::{GetString, Text, TextRef};

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during `YTransaction.commit`, which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// `YText` structure internally uses UTF-8 encoding and its length is described in a number of
/// bytes rather than individual characters (a single UTF-8 code point can consist of many bytes).
///
/// Like all Yrs shared data types, `YText` is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[derive(uniffi::Object)]
#[repr(transparent)]
pub struct YText {
    inner: Arc<RefCell<SharedCollection<String, TextRef>>>,
}

unsafe impl Sync for YText {}
unsafe impl Send for YText {}

impl YText {
    pub fn new(init: SharedCollection<String, TextRef>) -> Self {
        YText {
            inner: Arc::new(RefCell::new(init)),
        }
    }

    pub fn get_inner(&self) -> Arc<RefCell<SharedCollection<String, TextRef>>> {
        self.inner.clone()
    }
}

#[uniffi::export]
impl YText {
    /// Creates a new preliminary instance of a `YText` shared data type, with its state initialized
    /// to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[uniffi::constructor]
    pub fn new_with_text(init: Option<String>) -> Self {
        YText {
            inner: Arc::new(RefCell::new(SharedCollection::prelim(
                init.unwrap_or_default(),
            ))),
        }
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_TEXT
    }

    /// Returns true if this is a preliminary instance of `YArray`.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.get_inner().borrow().is_prelim()
    }

    /// Checks if current YArray reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.get_inner().borrow().is_alive(txn)
    }

    /// Returns length of an underlying string stored in this `YText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        match self.get_inner().borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Returns an underlying shared string stored in this data type.
    #[uniffi::method(default(txn=None))]
    pub fn get_text(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        match self.get_inner().borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.clone()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Inserts a given `chunk` of text into this `YText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
    #[uniffi::method(default(attributes=None, txn=None))]
    pub fn insert(
        &self,
        index: u32,
        chunk: &str,
        attributes: Option<String>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        let attributes = parse_attrs(attributes)?;

        match self.get_inner().borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                if let None = attributes {
                    c.insert_str(index as usize, chunk);
                    Ok(())
                } else {
                    Err(Error::InvalidPrelimOp)
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if let Some(attrs) = attributes {
                    c.insert_with_attributes(txn, index, chunk, attrs);
                    Ok(())
                } else {
                    c.insert(txn, index, chunk);
                    Ok(())
                }
            }),
        }
    }

    /// Wraps an existing piece of text within a range described by `index`-`length` parameters with
    /// formatting blocks containing provided `attributes` metadata. This method only works for
    /// `YText` instances that already have been integrated into document store.
    #[uniffi::method(default(txn=None))]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: String,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        let attrs = match parse_attrs(Some(attributes))? {
            Some(attrs) => attrs,
            None => return Err(Error::InvalidFmt),
        };

        match &self.get_inner().borrow_mut().deref_mut() {
            SharedCollection::Prelim(_) => Err(Error::InvalidPrelimOp),
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.format(txn, index, length, attrs);
                Ok(())
            }),
        }
    }

    /// Appends a given `chunk` of text at the end of current `YText` instance.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.`attributes` are only supported for a `YText` instance which
    /// already has been integrated into document store.
    #[uniffi::method(default(attributes=None, txn=None))]
    pub fn push(
        &self,
        chunk: &str,
        attributes: Option<String>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        let attributes = parse_attrs(attributes)?;

        match self.get_inner().borrow_mut().deref_mut() {
            SharedCollection::Prelim(ref mut c) => {
                if let Some(_) = attributes {
                    Err(Error::InvalidPrelimOp)
                } else {
                    c.push_str(chunk);
                    Ok(())
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if let Some(attrs) = attributes {
                    let len = c.len(txn);
                    c.insert_with_attributes(txn, len, chunk, attrs);
                    Ok(())
                } else {
                    c.push(txn, chunk);
                    Ok(())
                }
            }),
        }
    }

    /// Deletes a specified range of of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[uniffi::method(default(txn=None))]
    pub fn delete(&self, index: u32, length: u32, txn: Option<Arc<YTransaction>>) -> Result<()> {
        match self.get_inner().borrow_mut().deref_mut() {
            SharedCollection::Prelim(ref mut c) => {
                c.drain((index as usize)..((index + length) as usize));
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_range(txn, index, length);
                Ok(())
            }),
        }
    }
}
