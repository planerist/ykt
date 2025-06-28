use crate::collection::SharedCollection;
use crate::tools::Error;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use std::cell::RefCell;
use std::ops::Deref;
use std::sync::Arc;
use yrs::block::{EmbedPrelim, ItemContent, Prelim};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::xml::XmlPrelim;
use yrs::types::TypeRef;
use yrs::{Doc, Text, TransactionMut, XmlElementRef, XmlFragmentRef, XmlOut, XmlTextRef};

#[derive(uniffi::Enum)]
#[derive(Clone)]
pub enum YXmlChild {
    Element(Arc<YXmlElement>),
    Fragment(Arc<YXmlFragment>),
    Text(Arc<YXmlText>),
}


impl XmlPrelim for YXmlChild {}

impl Into<EmbedPrelim<YXmlChild>> for YXmlChild {
    fn into(self) -> EmbedPrelim<YXmlChild> {
        EmbedPrelim::Shared(self)
    }
}

impl Prelim for YXmlChild {
    type Return = XmlOut;

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = self.type_ref(txn);
        let branch = Branch::new(type_ref);
        (ItemContent::Type(branch), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let doc = txn.doc().clone();
        match self {
            YXmlChild::Text(cell) => {
                let xml_text = XmlTextRef::from(inner_ref);
                cell.clone().integrate(txn, xml_text);
            }
            YXmlChild::Element(mut cell) => {
                let xml_element = XmlElementRef::from(inner_ref);
                cell.clone().integrate(txn, xml_element);
            }
            YXmlChild::Fragment(mut cell) => {
                let xml_fragment = XmlFragmentRef::from(inner_ref);
                cell.clone().integrate(txn, xml_fragment);
            }
        }
    }
}

impl YXmlChild {
    pub fn from_xml(value: XmlOut, doc: Doc) -> Self {
        match value {
            XmlOut::Element(v) => YXmlChild::Element(Arc::new(YXmlElement(Arc::new(RefCell::new(SharedCollection::integrated(v, doc)))))),
            XmlOut::Fragment(v) => YXmlChild::Fragment(Arc::new(YXmlFragment::new_with_collection(SharedCollection::integrated(v, doc)))),
            XmlOut::Text(v) => YXmlChild::Text(Arc::new(YXmlText(Arc::new(RefCell::new((SharedCollection::integrated(v, doc))))))),
        }
    }

    pub fn assert_xml_prelim(&self) -> crate::tools::Result<()> {
        let prelim = match self {
            YXmlChild::Element(e) => e.prelim(),
            YXmlChild::Fragment(e) => e.prelim(),
            YXmlChild::Text(e) => e.prelim(),
        };

        if !prelim {
            return Err(Error::NotPrelim);
        }

        Ok(())
    }

    fn type_ref(&self, txn: &TransactionMut) -> TypeRef {
        match self {
            YXmlChild::Element(v) => {
                let name = match &v.0.borrow().deref() {
                    SharedCollection::Integrated(_) => panic!("{}", Error::NotPrelim),
                    SharedCollection::Prelim(p) => Arc::from(p.name.as_str()),
                };
                TypeRef::XmlElement(name)
            }
            YXmlChild::Fragment(_) => TypeRef::XmlFragment,
            YXmlChild::Text(_) => TypeRef::XmlText,
        }
    }
}
