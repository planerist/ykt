use std::collections::HashMap;
use yrs::XmlTextRef;
use crate::collection::SharedCollection;


pub(crate) struct PrelimXmlText {
    pub attributes: HashMap<String, String>,
    pub text: String,
}

#[derive(uniffi::Object)]
pub struct YXmlText(pub(crate) SharedCollection<PrelimXmlText, XmlTextRef>);
