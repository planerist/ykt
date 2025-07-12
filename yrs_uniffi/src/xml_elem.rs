use crate::attrs::{into_yvalue, YValue};
use crate::collection::{Integrated, SharedCollection};
use crate::tools::{Error, Result};
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;
use yrs::{Doc, GetString, Out, TransactionMut, Xml, XmlElementRef, XmlFragment};

impl Clone for PrelimXmElement {
    fn clone(&self) -> Self {
        PrelimXmElement {
            name: self.name.clone(),
            attributes: self.attributes.clone(),
            children: self.children.clone(),
        }
    }
}

pub(crate) struct PrelimXmElement {
    pub name: String,
    pub attributes: HashMap<String, YValue>,
    pub children: Vec<YXmlChild>,
}

impl PrelimXmElement {
    fn to_string(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        let mut str = String::new();

        for child in self.children.iter() {
            let res = match child.clone() {
                YXmlChild::Element(c) => c.clone().to_string(txn.clone()),
                YXmlChild::Fragment(c) => c.clone().to_string(txn.clone()),
                YXmlChild::Text(c) => c.clone().to_string(txn.clone()),
            };
            str.push_str(&res?);
        }

        Ok(str)
    }
}

/// XML element data type. It represents an XML node, which can contain key-value attributes
/// (interpreted as strings) as well as other nested XML elements or rich text (represented by
/// `YXmlText` type).
///
/// In terms of conflict resolution, `YXmlElement` uses following rules:
///
/// - Attribute updates use logical last-write-wins principle, meaning the past updates are
///   automatically overridden and discarded by newer ones, while concurrent updates made by
///   different peers are resolved into a single value using document id seniority to establish
///   an order.
/// - Child node insertion uses sequencing rules from other Yrs collections - elements are inserted
///   using interleave-resistant algorithm, where order of concurrent inserts at the same index
///   is established using peer's document id seniority.
#[derive(uniffi::Object)]
pub struct YXmlElement(pub(crate) Arc<RefCell<SharedCollection<PrelimXmElement, XmlElementRef>>>);

unsafe impl Sync for YXmlElement {}
unsafe impl Send for YXmlElement {}


impl YXmlElement {
    pub fn from_ref(elem_ref: XmlElementRef, doc: Doc) -> Self {
        YXmlElement(Arc::new(RefCell::new(SharedCollection::integrated(elem_ref, doc))))
    }
    
    pub fn integrate(&self, txn: &mut TransactionMut, xml_element: XmlElementRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.borrow_mut();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(
                xml_element.clone(),
                doc,
            )))
        };

        if let SharedCollection::Prelim(raw) = old_value {
            for child in raw.children.clone() {
                xml_element.push_back(txn, child);
            }
            for (name, value) in &raw.attributes {
                xml_element.insert_attribute(txn, name.clone(), value);
            }
        };
    }
}


#[uniffi::export]
impl YXmlElement {
    #[uniffi::constructor(default(attributes=None, children=None))]
    pub fn new(name: String, attributes: Option<HashMap<String, YValue>>, children: Option<Vec<YXmlChild>>) -> Result<YXmlElement> {
        let c = children.unwrap_or_default();

        for child in c.iter() {
            child.assert_xml_prelim()?;
        }
        Ok(YXmlElement(Arc::new(RefCell::new(SharedCollection::prelim(PrelimXmElement {
            name,
            attributes: attributes.unwrap_or_default(),
            children: c,
        })))))
    }

    /// Returns true if this is a preliminary instance of `YXmlElement`.
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

    /// Returns a tag name of this XML node.
    #[uniffi::method(default(txn=None))]
    pub fn name(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.name.clone()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, _| Ok(c.tag().to_string())),
        }
    }

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<u32> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.children.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn insert(
        &self,
        index: u32,
        xml_node: YXmlChild,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        xml_node.assert_xml_prelim()?;

        match self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.children.insert(index as usize, xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert(txn, index, xml_node);
                Ok(())
            }),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn push(&self, xml_node: YXmlChild, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        xml_node.assert_xml_prelim()?;

        match self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.children.push(xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.push_back(txn, xml_node);
                Ok(())
            }),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn delete(
        &self,
        index: u32,
        length: Option<u32>,
        txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        let length = length.unwrap_or(1);
        match self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.children
                    .drain((index as usize)..((index + length) as usize));
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.remove_range(txn, index, length);
                Ok(())
            }),
        }
    }

    /// Returns a first child of this XML node.
    /// It can be either `YXmlElement`, `YXmlText` or `undefined` if current node has not children.
    #[uniffi::method(default(txn=None))]
    pub fn first_child(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YXmlChild>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => {
                Ok(c.children.first().cloned())
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.first_child() {
                None => Ok(None),
                Some(xml) => Ok(YXmlChild::from_xml(xml, txn.doc().clone()).into()),
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
                    None => Ok(None),
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
            SharedCollection::Prelim(c) => c.to_string(txn),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }

    /// Sets a `name` and `value` as new attribute for this XML node. If an attribute with the same
    /// `name` already existed on that node, its value with be overridden with a provided one.
    #[uniffi::method(default(txn=None))]
    pub fn set_attribute(
        &self,
        name: &str,
        value: YValue,
        txn: Option<Arc<YTransaction>>)
        -> crate::tools::Result<()> {
        match &mut self.0.borrow_mut().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.attributes.insert(name.to_string(), value);
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
    pub fn get_attribute(&self, name: &str, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YValue>> {
       match &self.0.borrow().deref() {
            SharedCollection::Integrated(c) => {
                c.readonly(txn, |c, txn| {
                    let out = c.get_attribute(txn, name);
                    return match out {
                        None => Ok(None),
                        Some(out) => {
                            if let Out::Any(attr) = out {
                                Ok(Some(into_yvalue(&attr)))
                            } else {
                                return Err(Error::InvalidData("attr value".to_string()));
                            }
                        }
                    }
                })
            }
            SharedCollection::Prelim(c) => {
                let attr = c.attributes.get(name);
                match attr {
                    None =>  Ok(None),
                    Some(attr) =>  Ok(Some(attr.clone()))
                }
            }
        }
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
    pub fn attributes(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<HashMap<String, YValue>> {
        match &self.0.borrow().deref() {
            SharedCollection::Prelim(c) => Ok(c.clone().attributes),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let mut map = HashMap::new();
                for (name, value) in c.attributes(txn) {
                    if let Out::Any(attr) = value {
                        map.insert(name.to_string(), into_yvalue(&attr));
                    } else {
                        return Err(Error::InvalidData("attr value".to_string()));
                    }
                };

                Ok(map)
            }),
        }
    }
}