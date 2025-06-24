use crate::collection::SharedCollection;
use crate::transaction::YTransaction;
use std::iter::FromIterator;
use std::sync::Arc;
use yrs::types::xml::XmlEvent;
use yrs::types::TYPE_REFS_XML_FRAGMENT;
use yrs::{BranchID, DeepObservable, Doc, GetString, Observable, TransactionMut, XmlFragment, XmlFragmentRef, XmlOut};
use crate::tools::Error;
use crate::xml::YXml;
use crate::xml_elem::{PrelimXmElement, YXmlElement};
use crate::xml_text::YXmlText;

/// Represents a list of `YXmlElement` and `YXmlText` types.
/// A `YXmlFragment` is similar to a `YXmlElement`, but it does not have a
/// nodeName and it does not have attributes. Though it can be bound to a DOM
/// element - in this case the attributes and the nodeName are not shared
#[derive(uniffi::Object)]
#[repr(transparent)]
pub struct YXmlFragment(pub(crate) SharedCollection<Vec<Arc<YXmlElement>>, XmlFragmentRef>);

#[uniffi::export]
impl YXmlFragment {
    #[uniffi::constructor]
    pub fn new(children: Vec<Arc<YXmlElement>>) -> Self {
        let mut nodes = Vec::with_capacity(children.len());
        for xml_node in children {
            // TODO: Js::assert_xml_prelim(&xml_node)?;
            nodes.push(xml_node);
        }
        YXmlFragment(SharedCollection::prelim(nodes))
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
        self.0.is_prelim()
    }

    /// Checks if current shared type reference is alive and has not been deleted by its parent collection.
    /// This method only works on already integrated shared types and will return false is current
    /// type is preliminary (has not been integrated into document).
    #[inline]
    pub fn alive(&self, txn: &YTransaction) -> bool {
        self.0.is_alive(txn)
    }

    /// Returns a number of child XML nodes stored within this `YXMlElement` instance.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<u32> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.len() as u32),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.len(txn))),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn insert(
        &mut self,
        index: u32,
        xml_node: Arc<YXmlElement>,
        txn: Option<Arc<YTransaction>>
    ) -> crate::tools::Result<()> {
        let xml_node_prelim = match &xml_node.0 {
            SharedCollection::Prelim(c) => c,
            SharedCollection::Integrated(_) => return Err(Error::NotPrelim),
        };
        
        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.insert(index as usize, xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.insert(txn, index, xml_node_prelim.clone());
                Ok(())
            }),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn push(&mut self,  xml_node: Arc<YXmlElement>,
                txn: Option<Arc<YTransaction>>) -> crate::tools::Result<()> {
        let xml_node_prelim = match &xml_node.0 {
            SharedCollection::Prelim(c) => c,
            SharedCollection::Integrated(_) => return Err(Error::NotPrelim),
        };

        match &mut self.0 {
            SharedCollection::Prelim(c) => {
                c.push(xml_node);
                Ok(())
            }
            SharedCollection::Integrated(c) => c.mutably(txn, |c, txn| {
                c.push_back(txn, xml_node_prelim.clone());
                Ok(())
            }),
        }
    }

    #[uniffi::method(default(txn=None))]
    pub fn delete(
        &mut self,
        index: u32,
        length: Option<u32>,
        txn: Option<Arc<YTransaction>>
    ) -> crate::tools::Result<()> {
        let length = length.unwrap_or(1);
        match &mut self.0 {
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
    pub fn first_child(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<Option<YXml>> {
        match &self.0 {
            SharedCollection::Prelim(c) =>  Ok(match c.first() {
                None => None,
                Some(found) => Some(YXml::Element(found.clone()))
            }),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| match c.first_child() {
                None => Ok(None),
                Some(xml) => Ok(Some(YXml::from_xml(xml, txn.doc().clone())))
            }),
        }
    }
}

