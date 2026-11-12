use crate::attrs::{from_yattrs, into_yattrs3, into_yvalue, out_into_yvalue, YValue};
use crate::collection::{integrated_of, is_alive, is_prelim, with_cell, Integrated, SharedCell, SharedCollection};
use crate::delta::{y_into_delta, YDelta};
use crate::snapshots::YSnapshot;
use crate::tools::{check_range, utf16_len, utf16_offset, Error, Result};
use crate::transaction::YTransaction;
use crate::xml::{YDeltaXmlChild, YXmlChild, YXmlDelta};
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::ops::Deref;
use std::sync::Arc;
use yrs::types::TYPE_REFS_XML_TEXT;
use yrs::{Doc, GetString, Out, Snapshot, Text, TransactionMut, Xml, XmlTextRef};

#[derive(Clone)]
pub(crate) struct PrelimXmlText {
    pub attributes: HashMap<String, YValue>,
    pub text: String,
}

/// Rich text in an XML tree (Yjs's `Y.XmlText`): a text with formatting, embeds and XML
/// attributes. Indexes count UTF-16 units, in a preliminary one too.
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
#[repr(transparent)]
pub struct YXmlText(pub(crate) Arc<SharedCell<PrelimXmlText, XmlTextRef>>);

unsafe impl Sync for YXmlText {}
unsafe impl Send for YXmlText {}

impl YXmlText {
    pub fn from_ref(xml_text_ref: XmlTextRef, doc: Doc) -> Self {
        YXmlText(Arc::new(RefCell::new(SharedCollection::integrated(xml_text_ref, doc))))
    }

    pub fn integrate(&self, txn: &mut TransactionMut, xml_text: XmlTextRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.borrow_mut();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(xml_text.clone(), doc)))
        };

        if let SharedCollection::Prelim(raw) = old_value {
            xml_text.insert(txn, 0, &raw.text);
            for (name, value) in raw.attributes {
                xml_text.insert_attribute(txn, name.clone(), value);
            }
        }
    }
}

#[uniffi::export]
impl YXmlText {
    /// A preliminary XML text with `text` and XML `attributes`.
    #[uniffi::constructor(default(attributes=None))]
    pub fn new(text: String, attributes: Option<HashMap<String, YValue>>) -> Self {
        YXmlText(Arc::new(RefCell::new(SharedCollection::prelim(PrelimXmlText {
            text,
            attributes: attributes.unwrap_or_default(),
        }))))
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_TEXT
    }

    /// Whether this is a preliminary XML text, not in a document yet.
    #[inline]
    pub fn prelim(&self) -> bool {
        is_prelim(&self.0)
    }

    /// Whether the XML text is in its document, read in `txn`: `false` for a preliminary one or a
    /// deleted one.
    pub fn alive(&self, txn: &YTransaction) -> Result<bool> {
        is_alive(&self.0, txn)
    }

    /// The length of the text (UTF-16 units).
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        with_cell(&self.0, |c| Ok(utf16_len(&c.text)), |c| c.readonly(txn, |c, txn| Ok(c.len(txn))))
    }

    /// Inserts `chunk` at `index` (0 ..= `length()`, else `OutOfBounds`) with formatting
    /// `attributes`. A preliminary XML text takes no attributes (`NotIntegrated`).
    #[uniffi::method(default(txn=None))]
    pub fn insert(
        &self,
        index: u32,
        chunk: &str,
        attributes: Option<HashMap<String, YValue>>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        with_cell(
            &self.0,
            |c| match &attributes {
                None => {
                    let at = utf16_offset(&c.text, index)?;
                    c.text.insert_str(at, chunk);
                    Ok(())
                }
                Some(_) => Err(Error::NotIntegrated),
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    if index > c.len(txn) {
                        return Err(Error::OutOfBounds);
                    }
                    match &attributes {
                        None => c.insert(txn, index, chunk),
                        Some(attrs) => c.insert_with_attributes(txn, index, chunk, from_yattrs(attrs)),
                    }
                    Ok(())
                })
            },
        )
    }

    /// Inserts a preliminary XML node (`AlreadyIntegrated` otherwise) as an embed at `index`, with
    /// formatting `attributes`. `NotIntegrated` for a preliminary XML text.
    #[uniffi::method(default(txn=None))]
    pub fn insert_embed(
        &self,
        index: u32,
        embed: YXmlChild,
        attributes: Option<HashMap<String, YValue>>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        embed.assert_xml_prelim()?;
        integrated_of(&self.0)?.mutably(txn, |c, txn| {
            if index > c.len(txn) {
                return Err(Error::OutOfBounds);
            }
            match attributes {
                None => {
                    c.insert_embed(txn, index, embed);
                }
                Some(attrs) => {
                    c.insert_embed_with_attributes(txn, index, embed, from_yattrs(&attrs));
                }
            }
            Ok(())
        })
    }

    /// Formats `length` units from `index` on with `attributes` (a `null` value removes that
    /// attribute). `OutOfBounds` past the end; `NotIntegrated` for a preliminary XML text.
    #[uniffi::method(default(txn=None))]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: HashMap<String, YValue>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        let attrs = from_yattrs(&attributes);
        integrated_of(&self.0)?.mutably(txn, |c, txn| {
            check_range(index, length, c.len(txn))?;
            c.format(txn, index, length, attrs);
            Ok(())
        })
    }

    /// Appends `chunk` at the end, with formatting `attributes` as in `insert`.
    #[uniffi::method(default(txn=None))]
    pub fn push(&self, chunk: &str, attributes: Option<HashMap<String, YValue>>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        with_cell(
            &self.0,
            |c| match &attributes {
                None => {
                    c.text.push_str(chunk);
                    Ok(())
                }
                Some(_) => Err(Error::NotIntegrated),
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    match &attributes {
                        None => c.push(txn, chunk),
                        Some(attrs) => {
                            let len = c.len(txn);
                            c.insert_with_attributes(txn, len, chunk, from_yattrs(attrs))
                        }
                    }
                    Ok(())
                })
            },
        )
    }

    /// Deletes `length` units from `index` on (`OutOfBounds` if the text hasn't got them all).
    #[uniffi::method(default(txn=None))]
    pub fn delete(&self, index: u32, length: u32, txn: Option<Arc<YTransaction>>) -> Result<()> {
        with_cell(
            &self.0,
            |c| {
                let end = index.checked_add(length).ok_or(Error::OutOfBounds)?;
                let (from, to) = (utf16_offset(&c.text, index)?, utf16_offset(&c.text, end)?);
                c.text.drain(from..to);
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    check_range(index, length, c.len(txn))?;
                    if length > 0 {
                        c.remove_range(txn, index, length);
                    }
                    Ok(())
                })
            },
        )
    }

    /// Applies a delta (Yjs's `applyDelta`), as `YText.applyDelta`: retains (formatting with their
    /// attributes), inserts of strings and embeds with attributes, deletes. A retain past the end
    /// stops there; a delete past the end throws `OutOfBounds`, before anything is applied.
    /// `NotIntegrated` for a preliminary XML text.
    #[uniffi::method(default(txn=None))]
    pub fn apply_delta(&self, delta: Vec<YDelta>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        integrated_of(&self.0)?.mutably(txn, |c, txn| {
            crate::text::check_delta(&delta, c.len(txn), txn.doc().offset_kind())?;
            c.apply_delta(txn, delta.iter().map(y_into_delta));
            Ok(())
        })
    }

    /// The next sibling node, or `null` for the last child. `NotIntegrated` for a preliminary XML
    /// text.
    #[uniffi::method(default(txn=None))]
    pub fn next_sibling(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| {
            Ok(c.siblings(txn).next().map(|node| YXmlChild::from_xml(node, txn.doc().clone())))
        })
    }

    /// The previous sibling node, or `null` for the first child. `NotIntegrated` for a preliminary
    /// XML text.
    #[uniffi::method(default(txn=None))]
    pub fn prev_sibling(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| {
            Ok(c.siblings(txn).next_back().map(|node| YXmlChild::from_xml(node, txn.doc().clone())))
        })
    }

    /// The parent node, or `null` for none. `NotIntegrated` for a preliminary XML text.
    #[uniffi::method(default(txn=None))]
    pub fn parent(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| Ok(c.parent().map(|node| YXmlChild::from_xml(node, txn.doc().clone()))))
    }

    /// The text as XML (formatting as tags).
    #[uniffi::method(name = "toText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        with_cell(&self.0, |c| Ok(c.text.clone()), |c| c.readonly(txn, |c, txn| Ok(c.get_string(txn))))
    }

    /// Sets XML attribute `name` to `value`, replacing what it had.
    #[uniffi::method(default(txn=None))]
    pub fn set_attribute(&self, name: &str, value: YValue, txn: Option<Arc<YTransaction>>) -> Result<()> {
        with_cell(
            &self.0,
            |c| {
                c.attributes.insert(name.to_string(), value.clone());
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    c.insert_attribute(txn, name, value.clone());
                    Ok(())
                })
            },
        )
    }

    /// The value of XML attribute `name`, `null` if it hasn't got it (a shared type in its JSON
    /// form).
    #[uniffi::method(default(txn=None))]
    pub fn get_attribute(&self, name: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YValue>> {
        with_cell(
            &self.0,
            |c| Ok(c.attributes.get(name).cloned()),
            |c| c.readonly(txn, |c, txn| Ok(c.get_attribute(txn, name).map(|out| out_into_yvalue(&out, txn)))),
        )
    }

    /// Removes XML attribute `name`.
    #[uniffi::method(default(txn=None))]
    pub fn remove_attribute(&self, name: String, txn: Option<Arc<YTransaction>>) -> Result<()> {
        with_cell(
            &self.0,
            |c| {
                c.attributes.remove(&name);
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    c.remove_attribute(txn, &name);
                    Ok(())
                })
            },
        )
    }

    /// Every XML attribute, in no particular order.
    #[uniffi::method(default(txn=None))]
    pub fn attributes(&self, txn: Option<Arc<YTransaction>>) -> Result<HashMap<String, YValue>> {
        with_cell(
            &self.0,
            |c| Ok(c.attributes.clone()),
            |c| {
                c.readonly(txn, |c, txn| {
                    Ok(c.attributes(txn).map(|(name, value)| (name.to_string(), out_into_yvalue(&value, txn))).collect())
                })
            },
        )
    }

    /// The text as a delta: inserts of strings and embeds with their formatting, XML nodes embedded
    /// in it as handles; between two snapshots, what changed. `NotIntegrated` for a preliminary
    /// XML text.
    #[uniffi::method(default(snapshot=None,prev_snapshot=None,txn=None))]
    pub fn to_delta(
        &self,
        snapshot: Option<Arc<YSnapshot>>,
        prev_snapshot: Option<Arc<YSnapshot>>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<Vec<YXmlDelta>> {
        integrated_of(&self.0)?.mutably(txn, |c, txn| {
            let hi: Option<Snapshot> = snapshot.map(|snap| snap.deref().deref().clone());
            let lo: Option<Snapshot> = prev_snapshot.map(|snap| snap.deref().deref().clone());
            let doc = txn.doc().clone();
            let delta = c.diff_range(txn, hi.as_ref(), lo.as_ref(), |change| change);
            Ok(delta
                .into_iter()
                .map(|d| {
                    let child = match d.insert {
                        Out::YXmlText(text) => YDeltaXmlChild::Text(Arc::new(YXmlText::from_ref(text, doc.clone()))),
                        Out::YXmlElement(element) => YDeltaXmlChild::Element(Arc::new(YXmlElement::from_ref(element, doc.clone()))),
                        Out::YXmlFragment(fragment) => YDeltaXmlChild::Fragment(Arc::new(YXmlFragment::from_ref(fragment, doc.clone()))),
                        Out::Any(any) => YDeltaXmlChild::Embed(into_yvalue(&any), d.attributes.map(|attrs| into_yattrs3(attrs.deref()))),
                        other => YDeltaXmlChild::Embed(out_into_yvalue(&other, &*txn), d.attributes.map(|attrs| into_yattrs3(attrs.deref()))),
                    };
                    YXmlDelta::YInsert(child)
                })
                .collect())
        })
    }

    /// Calls `observer` after every committed transaction that changed this XML text itself — its text, its XML attributes
    /// — with the transaction's origin (Yjs's `observe`). The rules of `YText.observe` hold.
    /// `NotIntegrated` for a preliminary one.
    pub fn observe(&self, observer: Arc<dyn crate::events::YXmlObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlTextRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_xml_text(shared, c.doc.clone(), observer))
    }

    /// Calls `observer` after every committed transaction that changed this XML text (the nodes in it included) or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlTextRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_deep(shared, c.doc.clone(), observer))
    }
}
