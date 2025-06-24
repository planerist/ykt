use crate::collection::SharedCollection;
use crate::tools::Result;
use crate::xml::YXmlChild;
use std::collections::HashMap;
use std::ops::Deref;
use std::sync::Arc;
use yrs::{GetString, XmlElementRef};
use crate::transaction::YTransaction;

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
    pub attributes: HashMap<String, String>,
    pub children: Vec<YXmlChild>,
}

impl PrelimXmElement {
    fn to_string(&self, txn: Option<Arc<YTransaction>>)  ->  crate::tools::Result<String> {
        let mut str = String::new();
        
        for child in  self.children.iter() {
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
#[derive(Clone)]
pub struct YXmlElement(pub(crate) SharedCollection<PrelimXmElement, XmlElementRef>);


#[uniffi::export]
impl YXmlElement {
    #[uniffi::constructor]
    pub fn new(name: String, attributes: HashMap<String, String>, children: Vec<YXmlChild>) -> Result<YXmlElement> {
        for child in children.iter() {
            child.assert_xml_prelim()?;
        }
        Ok(YXmlElement(SharedCollection::prelim(PrelimXmElement {
            name,
            attributes,
            children,
        })))
    }

    /// Returns true if this is a preliminary instance of `YXmlElement`.
    ///
    /// Preliminary instances can be nested into other shared data types.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.is_prelim()
    }

    #[uniffi::method(name = "toString", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>)  ->  crate::tools::Result<String> {
        match &self.0 {
            SharedCollection::Prelim(c) => c.to_string(txn),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }
}