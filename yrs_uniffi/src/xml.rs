use crate::attrs::{YAttributes, YValue};
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
use yrs::{Doc, TransactionMut, XmlElementRef, XmlFragmentRef, XmlOut, XmlTextRef};

#[derive(uniffi::Enum)]
#[derive(Clone, Debug)]
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

#[derive(uniffi::Enum)]
#[derive(Clone)]
pub enum YDeltaXmlChild {
    Embed(YValue, Option<YAttributes>),
    Element(Arc<YXmlElement>),
    Fragment(Arc<YXmlFragment>),
    Text(Arc<YXmlText>),
}


#[derive(uniffi::Enum)]
pub enum YXmlDelta {
    YInsert(YDeltaXmlChild),
    YDelete(u32),
    YRetain(u32, Option<YAttributes>),
}

impl Prelim for YXmlChild {
    type Return = XmlOut;

    fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = self.type_ref();
        let branch = Branch::new(type_ref);
        (ItemContent::Type(branch), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        match self {
            YXmlChild::Text(cell) => {
                let xml_text = XmlTextRef::from(inner_ref);
                cell.clone().integrate(txn, xml_text);
            }
            YXmlChild::Element(cell) => {
                let xml_element = XmlElementRef::from(inner_ref);
                cell.clone().integrate(txn, xml_element);
            }
            YXmlChild::Fragment(cell) => {
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
            XmlOut::Text(v) => YXmlChild::Text(Arc::new(YXmlText(Arc::new(RefCell::new(SharedCollection::integrated(v, doc)))))),
        }
    }

    /// The node as XML text.
    pub fn to_text(&self, txn: Option<Arc<crate::transaction::YTransaction>>) -> crate::tools::Result<String> {
        match self {
            YXmlChild::Element(c) => c.to_string(txn),
            YXmlChild::Fragment(c) => c.to_string(txn),
            YXmlChild::Text(c) => c.to_string(txn),
        }
    }

    /// `AlreadyIntegrated` unless this node and every node in it are preliminary: a node already in
    /// a document can't be inserted again.
    pub fn assert_xml_prelim(&self) -> crate::tools::Result<()> {
        let children = match self {
            YXmlChild::Element(e) => match &*e.0.try_borrow().map_err(|_| Error::Locked)? {
                SharedCollection::Prelim(p) => p.children.clone(),
                SharedCollection::Integrated(_) => return Err(Error::AlreadyIntegrated),
            },
            YXmlChild::Fragment(f) => match &*f.0.try_borrow().map_err(|_| Error::Locked)? {
                SharedCollection::Prelim(children) => children.clone(),
                SharedCollection::Integrated(_) => return Err(Error::AlreadyIntegrated),
            },
            YXmlChild::Text(t) => match &*t.0.try_borrow().map_err(|_| Error::Locked)? {
                SharedCollection::Prelim(_) => vec![],
                SharedCollection::Integrated(_) => return Err(Error::AlreadyIntegrated),
            },
        };
        children.iter().try_for_each(|child| child.assert_xml_prelim())
    }

    fn type_ref(&self) -> TypeRef {
        match self {
            YXmlChild::Element(v) => {
                let name = match &v.0.borrow().deref() {
                    SharedCollection::Integrated(_) => unreachable!("checked by assert_xml_prelim"),
                    SharedCollection::Prelim(p) => Arc::from(p.name.as_str()),
                };
                TypeRef::XmlElement(name)
            }
            YXmlChild::Fragment(_) => TypeRef::XmlFragment,
            YXmlChild::Text(_) => TypeRef::XmlText,
        }
    }
}
