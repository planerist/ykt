use crate::collection::SharedCollection;
use crate::tools::Result;
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, RwLock};
use yrs::{GetString, Xml, XmlElementRef};
use crate::xml_frag::YXmlFragment;

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
pub struct YXmlElement(pub(crate) RwLock<SharedCollection<PrelimXmElement, XmlElementRef>>);

impl Clone for YXmlElement {
    fn clone(&self) -> Self {
        let cloned = self.0.read().unwrap().clone();
        YXmlElement(RwLock::new(cloned))
    }
}


#[uniffi::export]
impl YXmlElement {
    #[uniffi::constructor]
    pub fn new(name: String, attributes: HashMap<String, String>, children: Vec<YXmlChild>) -> Result<YXmlElement> {
        for child in children.iter() {
            child.assert_xml_prelim()?;
        }
        Ok(YXmlElement(RwLock::new(SharedCollection::prelim(PrelimXmElement {
            name,
            attributes,
            children,
        }))))
    }

    /// Returns true if this is a preliminary instance of `YXmlElement`.
    ///
    /// Preliminary instances can be nested into other shared data types.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[inline]
    pub fn prelim(&self) -> bool {
        self.0.read().unwrap().is_prelim()
    }

    #[uniffi::method(name = "toString", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        match &self.0.read().unwrap().deref() {
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
        value: &str,
        txn: Option<Arc<YTransaction>>)
        -> crate::tools::Result<()> {
        match &mut self.0.write().unwrap().deref_mut() {
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
        let value = match &self.0.read().unwrap().deref() {
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
        match &mut self.0.write().unwrap().deref_mut() {
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
        match &self.0.read().unwrap().deref() {
            SharedCollection::Prelim(c) => Ok(c.clone().attributes),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| {
                let mut map =  HashMap::new();
                for (name, value) in c.attributes(txn) {
                    map.insert(name.to_string(), value);
                }
                Ok(map)
            }),
        }
    }
}