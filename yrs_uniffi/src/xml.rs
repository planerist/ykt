use crate::collection::SharedCollection;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use std::sync::Arc;
use yrs::{Doc, XmlOut};

#[derive(uniffi::Object)]
pub enum YXml {
    Element(Arc<YXmlElement>),
    Fragment(Arc<YXmlFragment>),
    Text(Arc<YXmlText>),
}


impl YXml {
    pub fn from_xml(value: XmlOut, doc: Doc) -> Self {
        match value {
            XmlOut::Element(v) => YXml::Element(Arc::new(YXmlElement(SharedCollection::integrated(v, doc)))),
            XmlOut::Fragment(v) => YXml::Fragment(Arc::new(YXmlFragment(SharedCollection::integrated(v, doc)))),
            XmlOut::Text(v) => YXml::Text(Arc::new(YXmlText(SharedCollection::integrated(v, doc)))),
        }
    }
}