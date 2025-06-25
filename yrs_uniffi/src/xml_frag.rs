use crate::collection::{Integrated, SharedCollection};
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::mem;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, RwLock};
use yrs::types::TYPE_REFS_XML_FRAGMENT;
use yrs::{GetString, TransactionMut, XmlElementRef, XmlFragment, XmlFragmentRef};

/// Represents a list of `YXmlElement` and `YXmlText` types.
/// A `YXmlFragment` is similar to a `YXmlElement`, but it does not have a
/// nodeName and it does not have attributes. Though it can be bound to a DOM
/// element - in this case the attributes and the nodeName are not shared
#[derive(uniffi::Object)]
#[repr(transparent)]
pub struct YXmlFragment(pub(crate) RwLock<SharedCollection<Vec<YXmlChild>, XmlFragmentRef>>);

impl Clone for YXmlFragment {
    fn clone(&self) -> Self {
        let cloned = self.0.read().unwrap().clone();
        YXmlFragment(RwLock::new(cloned))
    }
}

unsafe impl Sync for YXmlFragment {}
unsafe impl Send for YXmlFragment {}


impl YXmlFragment {
    pub fn new_with_collection(init: SharedCollection<Vec<YXmlChild>, XmlFragmentRef>) -> Self {
        YXmlFragment(RwLock::new(init))
    }

    pub fn integrate(&self, txn: &mut TransactionMut, xml_fragment: XmlFragmentRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.write().unwrap();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(
                xml_fragment.clone(),
                doc,
            )))
        };

        if let SharedCollection::Prelim(raw) = old_value
        {
            for child in raw {
                xml_fragment.push_back(txn, child.clone());
            }
        };
    }
}


#[uniffi::export]
impl YXmlFragment {
    #[uniffi::constructor]
    pub fn new(children: Vec<YXmlChild>) -> crate::tools::Result<Self> {
        let mut nodes = Vec::with_capacity(children.len());
        for xml_node in children {
            xml_node.assert_xml_prelim()?;
            nodes.push(xml_node);
        }
        Ok(YXmlFragment::new_with_collection(SharedCollection::prelim(nodes)))
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_FRAGMENT
    }

    /// Returns true if this is a preliminary instance of `YXmlFragment`.
    ///
    /// Preliminary instances can be nested into other shared data types.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.read().unwrap().is_prelim()
    }

    /// Checks if current shared type reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.read().unwrap().is_alive(txn)
    }

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<u32> {
        match self.0.read().unwrap().deref() {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn insert(
        &self,
        index: u32,
        xml_node: YXmlChild,
        txn: Option<Arc<YTransaction>>,
    ) -> crate::tools::Result<()> {
        xml_node.assert_xml_prelim()?;

        match self.0.write().unwrap().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.insert(index as usize, xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert(txn, index, xml_node);
                Ok(())
            }),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn push(&self, xml_node: YXmlChild,
                txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        xml_node.assert_xml_prelim()?;

        match self.0.write().unwrap().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.push(xml_node);
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
        txn: Option<Arc<YTransaction>>,
    ) -> crate::tools::Result<()> {
        let length = length.unwrap_or(1);
        match self.0.write().unwrap().deref_mut() {
            SharedCollection::Prelim(c) => {
                c.drain((index as usize)..((index + length) as usize));
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
        match self.0.read().unwrap().deref() {
            SharedCollection::Prelim(c) => Ok(match c.first() {
                None => None,
                Some(found) => Some(found.clone())
            }),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.first_child() {
                None => Ok(None),
                Some(xml) => Ok(Some(YXmlChild::from_xml(xml, txn.doc().clone())))
            }),
        }
    }

    /// Returns a string representation of this XML node.
    #[uniffi::method(name = "getText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        match &self.0.read().unwrap().deref() {
            SharedCollection::Prelim(c) => {
                let mut str = String::new();
                for child in c.iter() {
                    let res = match child {
                        YXmlChild::Element(c) => c.clone().to_string(txn.clone()),
                        YXmlChild::Fragment(c) => c.clone().to_string(txn.clone()),
                        YXmlChild::Text(c) => c.clone().to_string(txn.clone()),
                    };
                    str.push_str(&res?);
                }
                Ok(str)
            }
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }
}

