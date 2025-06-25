use std::collections::HashMap;
use crate::collection::{Integrated, SharedCollection};
use crate::tools::Error;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, RwLock};
use yrs::block::{ItemContent, Prelim};
use yrs::branch::{Branch, BranchPtr};
use yrs::types::xml::XmlPrelim;
use yrs::types::TypeRef;
use yrs::{Doc, Text, TransactionMut, Xml, XmlElementRef, XmlFragment, XmlFragmentRef, XmlOut, XmlTextRef};

#[derive(uniffi::Enum)]
#[derive(Clone)]
pub enum YXmlChild {
    Element(Arc<YXmlElement>),
    Fragment(Arc<YXmlFragment>),
    Text(Arc<YXmlText>),
}


impl XmlPrelim for YXmlChild {}

impl Prelim for YXmlChild {
    type Return = XmlOut;

    // fn into_content(self, _txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
    //     let type_ref = match &self {
    //         YXmlChild::Text(_) => TypeRef::XmlText,
    //         YXmlChild::Element(prelim) => {
    //             match prelim.0 {
    //                 SharedCollection::Integrated(integrated) => TypeRef::XmlElement(integrated.tag.clone()),
    //                 SharedCollection::Prelim(p) => TypeRef::XmlElement(p.name),
    //             }
    //             
    //         },
    //         YXmlChild::Fragment(_) => TypeRef::XmlFragment,
    //     };
    //     
    //     (ItemContent::Type(Branch::new(type_ref)), Some(self))
    // }

    fn into_content(self, txn: &mut TransactionMut) -> (ItemContent, Option<Self>) {
        let type_ref = self.type_ref(txn);
        let branch = Branch::new(type_ref);
        (ItemContent::Type(branch), Some(self))
    }

    fn integrate(self, txn: &mut TransactionMut, inner_ref: BranchPtr) {
        let doc = txn.doc().clone();
        match self {
            YXmlChild::Text(mut cell) => {
                println!("Integrating: Text");

                let xml_text = XmlTextRef::from(inner_ref);
                let old_value = std::mem::replace(
                    &mut cell,
                    Arc::new(YXmlText(RwLock::new(SharedCollection::Integrated(Integrated::new(
                        xml_text.clone(),
                        doc,
                    ))))),
                );

                if let SharedCollection::Prelim(raw) = old_value.clone().0.read().unwrap().deref() {
                    xml_text.insert(txn, 0, &raw.text);
                    for (name, value) in &raw.attributes {
                        xml_text.insert_attribute(txn, name.clone(), value);
                    }
                }
            }
            YXmlChild::Element(mut cell) => {
                println!("Integrating: Element");

                let xml_element = XmlElementRef::from(inner_ref);
                let old_value = std::mem::replace(
                    &mut cell,
                    Arc::new(YXmlElement(RwLock::new(SharedCollection::Integrated(Integrated::new(
                        xml_element.clone(),
                        doc,
                    ))))),
                );

                if let SharedCollection::Prelim(raw) = old_value.0.read().unwrap().deref() {
                    for child in raw.children.clone() {
                        xml_element.push_back(txn, child);
                    }
                    for (name, value) in &raw.attributes {
                        xml_element.insert_attribute(txn, name.clone(), value);
                    }
                };
            }
            YXmlChild::Fragment(mut cell) => {
                println!("Integrating: Fragment");

                let xml_fragment = XmlFragmentRef::from(inner_ref);
                let old_value = std::mem::replace(
                    &mut cell,
                    Arc::new(YXmlFragment(RwLock::new(SharedCollection::Integrated(Integrated::new(
                        xml_fragment.clone(),
                        doc,
                    ))))),
                );

                if let SharedCollection::Prelim(raw) = old_value.0.read().unwrap().deref()
                {
                    for child in raw {
                        xml_fragment.push_back(txn, child.clone());
                    }
                };
            }
        }
    }
}

impl YXmlChild {
    pub fn from_xml(value: XmlOut, doc: Doc) -> Self {
        match value {
            XmlOut::Element(v) => YXmlChild::Element(Arc::new(YXmlElement(RwLock::new(SharedCollection::integrated(v, doc))))),
            XmlOut::Fragment(v) => YXmlChild::Fragment(Arc::new(YXmlFragment::new_with_collection(SharedCollection::integrated(v, doc)))),
            XmlOut::Text(v) => YXmlChild::Text(Arc::new(YXmlText(RwLock::new(SharedCollection::integrated(v, doc))))),
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
                let name = match &v.0.read().unwrap().deref() {
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
