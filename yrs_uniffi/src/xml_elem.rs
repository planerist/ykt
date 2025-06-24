use crate::tools::Result;
use std::collections::HashMap;
use yrs::block::{ItemContent, Prelim, Unused};
use yrs::types::xml::XmlPrelim;
use yrs::{TransactionMut, XmlElementRef};
use yrs::branch::BranchPtr;
use crate::collection::SharedCollection;
use crate::transaction::YTransaction;

impl XmlPrelim for PrelimXmElement {}

impl Prelim for PrelimXmElement {
    type Return = Unused;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        todo!()
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        todo!()
    }
}

impl Clone for PrelimXmElement {
    fn clone(&self) -> Self {
        PrelimXmElement {
            name: self.name.clone(),
            attributes: self.attributes.clone(),
            children: self.children.clone()
        }
    }
}

pub(crate) struct PrelimXmElement {
    pub name: String,
    pub attributes: HashMap<String, String>,
    pub children: Vec<PrelimXmElement>,
}

impl PrelimXmElement {
    // fn to_string(&self, txn: &YTransaction) -> Result<String> {
    //     let mut str = String::new();
    //     for js in self.children.iter() {
    //         let res = match Shared::from_ref(js)? {
    //             Shared::XmlText(c) => c.to_string(txn),
    //             Shared::XmlElement(c) => c.to_string(txn),
    //             Shared::XmlFragment(c) => c.to_string(txn),
    //             _ => return Err(JsValue::from_str(crate::js::errors::NOT_XML_TYPE)),
    //         };
    //         str.push_str(&res?);
    //     }
    //     Ok(str)
    // }
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
pub struct YXmlElement(pub(crate) SharedCollection<PrelimXmElement, XmlElementRef>);
