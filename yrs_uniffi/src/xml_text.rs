use crate::attrs::{into_yattrs, into_yvalue, YValue};
use crate::collection::{Integrated, SharedCollection};
use crate::delta::YDelta;
use crate::delta::YDelta::YInsert;
use crate::snapshots::YSnapshot;
use crate::text::YText;
use crate::tools::Error;
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::types::TYPE_REFS_XML_TEXT;
use yrs::{GetString, Out, Snapshot, Text, TransactionMut, Xml, XmlTextRef};

#[derive(Clone)]
pub(crate) struct PrelimXmlText {
    pub attributes: HashMap<String, String>,
    pub text: String,
}

#[derive(uniffi::Object)]
#[repr(transparent)]
pub struct YXmlText(pub(crate) Arc<RefCell<SharedCollection<PrelimXmlText, XmlTextRef>>>);

unsafe impl Sync for YXmlText {}
unsafe impl Send for YXmlText {}


impl YXmlText {
    pub fn integrate(&self, txn: &mut TransactionMut, xml_text: XmlTextRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.borrow_mut();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(
                xml_text.clone(),
                doc,
            )))
        };

        if let SharedCollection::Prelim(raw) = old_value {
            xml_text.insert(txn, 0, &raw.text);
            for (name, value) in &raw.attributes {
                xml_text.insert_attribute(txn, name.clone(), value);
            }
        }
    }
}

#[uniffi::export]
impl YXmlText {
    #[uniffi::constructor(default(attributes=None))]
    pub fn new(text: String, attributes: Option<HashMap<String, String>>) -> Self {
        YXmlText(Arc::new(RefCell::new(SharedCollection::prelim(PrelimXmlText {
            text: text,
            attributes: attributes.unwrap_or_default(),
        }))))
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_TEXT
    }

    /// Returns true if this is a preliminary instance of `YXmlText`.
    ///
    /// Preliminary instances can be nested into other shared data types.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.borrow().is_prelim()
    }

    /// Checks if current shared type reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.borrow().is_alive(txn)
    }

    /// Returns length of an underlying string stored in this `YXmlText` instance,
    /// understood as a number of UTF-8 encoded bytes.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<u32> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.text.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    /// Inserts a given `chunk` of text into this `YXmlText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[uniffi::method(default(txn=None))]
    pub fn insert(
        &self,
        index: u32,
        chunk: &str,
        attributes: Option<HashMap<String, String>>,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                if let None = attributes {
                    c.text.insert_str(index as usize, chunk);
                    Ok(())
                } else {
                    Err(Error::InvalidPrelimOp)
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if let None = attributes {
                    c.insert(txn, index, chunk);
                    Ok(())
                } else if let Some(attrs) = attributes {
                    c.insert_with_attributes(txn, index, chunk, YText::convert_attrs(attrs));
                    Ok(())
                } else {
                    Err(Error::InvalidFmt)
                }
            }),
        }
    }

    /// Inserts a given `embed` object into this `YXmlText` instance, starting at a given `index`.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided `embed`
    /// with a formatting blocks.`attributes` are only supported for a `YXmlText` instance which
    /// already has been integrated into document store.
    #[uniffi::method(default(txn=None))]
    pub fn insert_embed(
        &self,
        index: u32,
        embed: YXmlChild,
        attributes: Option<HashMap<String, String>>,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if attributes.is_none() {
                    c.insert_embed(txn, index, embed);
                    Ok(())
                } else if let Some(attrs) = attributes {
                    c.insert_embed_with_attributes(txn, index, embed, YText::convert_attrs(attrs));
                    Ok(())
                } else {
                    Err(Error::InvalidFmt)
                }
            }),
        }
    }

    /// Formats text within bounds specified by `index` and `len` with a given formatting
    /// attributes.
    #[uniffi::method(default(txn=None))]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: Option<HashMap<String, String>>,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        let attrs = match attributes {
            Some(attrs) => attrs,
            None => return Err(Error::InvalidFmt)
        };
        let attrs = YText::convert_attrs(attrs);

        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.format(txn, index, length, attrs);
                Ok(())
            }),
        }
    }

    /// Appends a given `chunk` of text at the end of `YXmlText` instance.
    ///
    /// Optional object with defined `attributes` will be used to wrap provided text `chunk`
    /// with a formatting blocks.
    #[uniffi::method(default(txn=None))]
    pub fn push(
        &self,
        chunk: &str,
        attributes: Option<HashMap<String, String>>,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                if let None = attributes {
                    c.text.push_str(chunk);
                    Ok(())
                } else {
                    Err(Error::InvalidPrelimOp)
                }
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                if let None = attributes {
                    c.push(txn, chunk);
                    Ok(())
                } else if let Some(attrs) = attributes {
                    let len = c.len(txn);
                    c.insert_with_attributes(txn, len, chunk, YText::convert_attrs(attrs));
                    Ok(())
                } else {
                    Err(Error::InvalidFmt)
                }
            }),
        }
    }

    /// Deletes a specified range of characters, starting at a given `index`.
    /// Both `index` and `length` are counted in terms of a number of UTF-8 character bytes.
    #[uniffi::method(default(txn=None))]
    pub fn delete(
        &self,
        index: u32,
        length: u32,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.text.drain((index as usize)..((index + length) as usize));
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_range(txn, index, length);
                Ok(())
            }),
        }
    }

    /// Returns a next XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a last child of
    /// parent XML node.
    #[uniffi::method(default(txn=None))]
    pub fn next_sibling(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YXmlChild>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let next = c.siblings(txn).next();
                match next {
                    Some(node) => Ok(YXmlChild::from_xml(node, txn.doc().clone()).into()),
                    None => Ok(None.into()),
                }
            }),
        }
    }

    /// Returns a previous XML sibling node of this XMl node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node is a first child
    /// of parent XML node.
    #[uniffi::method(default(txn=None))]
    pub fn prev_sibling(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YXmlChild>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let next = c.siblings(txn).next_back();
                match next {
                    Some(node) => Ok(YXmlChild::from_xml(node, txn.doc().clone()).into()),
                    None => Ok(None),
                }
            }),
        }
    }

    /// Returns a parent `YXmlElement` node or `undefined` if current node has no parent assigned.
    #[uniffi::method(default(txn=None))]
    pub fn parent(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YXmlChild>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.parent() {
                None => Ok(None),
                Some(node) => Ok(YXmlChild::from_xml(node, txn.doc().clone()).into()),
            }),
        }
    }

    #[uniffi::method(name = "toText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.text.to_string()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[uniffi::method(default(txn=None))]
    pub fn set_attribute(
        &self,
        name: &str,
        value: &str,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.attributes.insert(name.to_string(), value.to_string());
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert_attribute(txn, name, value);
                Ok(())
            }),
        }
    }

    /// Returns a value of an attribute given its `name`. If no attribute with such name existed,
    /// `null` will be returned.
    #[uniffi::method(default(txn=None))]
    pub fn get_attribute(&self, name: &str, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<String>> {
        let value = match &self.0.borrow().deref() {
            SharedCollection::Integrated(c) => {
                c.readonly(txn, |c, txn| Ok(c.get_attribute(txn, name)))?
            }
            SharedCollection::Prelim(c) => c.attributes.get(name).cloned(),
        };

        Ok(value)
    }

    /// Removes an attribute from this XML node, given its `name`.
    #[uniffi::method(default(txn=None))]
    pub fn remove_attribute(
        &self,
        name: String,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.attributes.remove(&name);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_attribute(txn, &name);
                Ok(())
            }),
        }
    }

    /// Returns an iterator that enables to traverse over all attributes of this XML node in
    /// unspecified order.
    #[uniffi::method(default(txn=None))]
    pub fn attributes(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<HashMap<String, String>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.attributes.clone()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let mut map = HashMap::new();
                for (name, value) in c.attributes(txn) {
                    map.insert(name.to_string(), value);
                };

                Ok(map)
            }),
        }
    }

    /// Returns the Delta representation of this YXmlText type.
    #[uniffi::method(default(snapshot=None,prev_snapshot=None,txn=None))]
    pub fn to_delta(
        &self,
        snapshot: Option<Arc<YSnapshot>>,
        prev_snapshot: Option<Arc<YSnapshot>>,
        txn: Option<Arc<YTransaction>>,
    ) -> crate::tools::Result<Vec<YDelta>> {
        match self.0.borrow().deref() {
            SharedCollection::Prelim(_) => {
                Err(Error::InvalidPrelimOp)
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                let hi: Option<Snapshot> = if let Some(snap) = snapshot {
                    let snap = snap.clone().deref().deref().clone();
                    Some(snap)
                } else {
                    None
                };
                let lo: Option<Snapshot> = if let Some(snap) = prev_snapshot {
                    let snap = snap.clone().deref().deref().clone();
                    Some(snap)
                } else {
                    None
                };

                let delta = c.diff_range(txn, hi.as_ref(), lo.as_ref(), |change| change);

                let mut array: Vec<YDelta> = vec![];
                for d in delta {
                    let attrs = match d.attributes {
                        None => None,
                        Some(attrs) => Some(into_yattrs(*attrs))
                    };

                    if let Out::Any(any) = d.insert {
                        let d = YInsert(into_yvalue(&any), attrs);
                        array.push(d);
                    } else if let Out::YXmlText(textRef) = d.insert {
                        let mut map = HashMap::new();
                        for (name, value) in textRef.attributes(txn) {
                            map.insert(name.to_string(), YValue::String(value));
                        }

                        let d = YInsert(YValue::String(textRef.get_string(txn)), Some(map));
                        array.push(d);
                    } else if let Out::YXmlElement(elementRef) = d.insert {
                        let mut map = HashMap::new();
                        for (name, value) in elementRef.attributes(txn) {
                            map.insert(name.to_string(), YValue::String(value));
                        }

                        let d = YInsert(YValue::String(elementRef.get_string(txn)), Some(map));
                        array.push(d);
                    } else if let Out::YXmlFragment(fragmentRef) = d.insert {
                        let d = YInsert(YValue::String(fragmentRef.get_string(txn)), None);
                        array.push(d);
                    } else {
                        return Err(Error::InvalidData(d.insert.to_string(txn)));
                    }
                }
                Ok(array)
            }),
        }
    }
}