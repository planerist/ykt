use crate::collection::SharedCollection;
use crate::transaction::YTransaction;
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::TYPE_REFS_XML_TEXT;
use yrs::{GetString, XmlTextRef};

#[derive(Clone)]
pub(crate) struct PrelimXmlText {
    pub attributes: HashMap<String, String>,
    pub text: String,
}

#[derive(uniffi::Object)]
#[derive(Clone)]
pub struct YXmlText(pub(crate) SharedCollection<PrelimXmlText, XmlTextRef>);


#[uniffi::export]
impl YXmlText {
    #[uniffi::constructor]
    pub fn new(text: Option<String>, attributes: HashMap<String, String>) -> crate::tools::Result<YXmlText> {
        Ok(YXmlText(SharedCollection::prelim(PrelimXmlText {
            text: text.unwrap_or_default(),
            attributes,
        })))
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
        self.0.is_prelim()
    }

    #[uniffi::method(name = "toString", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> crate::tools::Result<String> {
        match &self.0 {
            SharedCollection::Prelim(c) => Ok(c.text.to_string()),
            SharedCollection::Integrated(c) => c.readonly(txn, |c, txn| Ok(c.get_string(txn))),
        }
    }
}