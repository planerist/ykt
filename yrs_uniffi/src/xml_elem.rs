use crate::attrs::{out_into_yvalue, YValue};
use crate::collection::{integrated_of, is_alive, is_prelim, with_cell, Integrated, SharedCell, SharedCollection};
use crate::tools::{check_range, Error, Result};
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::cell::RefCell;
use std::collections::HashMap;
use std::mem;
use std::sync::Arc;
use yrs::{Doc, GetString, TransactionMut, Xml, XmlElementRef, XmlFragment};

#[derive(Clone)]
pub(crate) struct PrelimXmElement {
    pub name: String,
    pub attributes: HashMap<String, YValue>,
    pub children: Vec<YXmlChild>,
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
#[uniffi::export(Eq, Hash)]
pub struct YXmlElement(pub(crate) Arc<SharedCell<PrelimXmElement, XmlElementRef>>);

unsafe impl Sync for YXmlElement {}
unsafe impl Send for YXmlElement {}

impl YXmlElement {
    pub fn from_ref(elem_ref: XmlElementRef, doc: Doc) -> Self {
        YXmlElement(Arc::new(RefCell::new(SharedCollection::integrated(elem_ref, doc))))
    }

    pub fn integrate(&self, txn: &mut TransactionMut, xml_element: XmlElementRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.borrow_mut();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(xml_element.clone(), doc)))
        };

        if let SharedCollection::Prelim(raw) = old_value {
            for child in raw.children.clone() {
                xml_element.push_back(txn, child);
            }
            for (name, value) in &raw.attributes {
                xml_element.insert_attribute(txn, name.clone(), value);
            }
        };
    }
}

#[uniffi::export]
impl YXmlElement {
    /// A preliminary element `name` with `attributes` and `children` (preliminary nodes, else
    /// `AlreadyIntegrated`).
    #[uniffi::constructor(default(attributes=None, children=None))]
    pub fn new(name: String, attributes: Option<HashMap<String, YValue>>, children: Option<Vec<YXmlChild>>) -> Result<YXmlElement> {
        let children = children.unwrap_or_default();
        for child in children.iter() {
            child.assert_xml_prelim()?;
        }
        Ok(YXmlElement(Arc::new(RefCell::new(SharedCollection::prelim(PrelimXmElement {
            name,
            attributes: attributes.unwrap_or_default(),
            children,
        })))))
    }

    /// Whether this is a preliminary element, not in a document yet.
    #[inline]
    pub fn prelim(&self) -> bool {
        is_prelim(&self.0)
    }

    /// Whether the element is in its document, read in `txn`: `false` for a preliminary element or
    /// a deleted one.
    pub fn alive(&self, txn: &YTransaction) -> Result<bool> {
        is_alive(&self.0, txn)
    }

    /// The element's tag name.
    #[uniffi::method(default(txn=None))]
    pub fn name(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        with_cell(&self.0, |c| Ok(c.name.clone()), |c| c.readonly(txn, |c, _| Ok(c.tag().to_string())))
    }

    /// The number of child nodes.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        with_cell(&self.0, |c| Ok(c.children.len() as u32), |c| c.readonly(txn, |c, txn| Ok(c.len(txn))))
    }

    /// Inserts a preliminary node (`AlreadyIntegrated` otherwise) at `index` (0 ..= `length()`,
    /// else `OutOfBounds`).
    #[uniffi::method(default(txn=None))]
    pub fn insert(&self, index: u32, xml_node: YXmlChild, txn: Option<Arc<YTransaction>>) -> Result<()> {
        xml_node.assert_xml_prelim()?;
        with_cell(
            &self.0,
            |c| {
                if index as usize > c.children.len() {
                    return Err(Error::OutOfBounds);
                }
                c.children.insert(index as usize, xml_node.clone());
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    if index > c.len(txn) {
                        return Err(Error::OutOfBounds);
                    }
                    c.insert(txn, index, xml_node.clone());
                    Ok(())
                })
            },
        )
    }

    /// Appends a preliminary node (`AlreadyIntegrated` otherwise).
    #[uniffi::method(default(txn=None))]
    pub fn push(&self, xml_node: YXmlChild, txn: Option<Arc<YTransaction>>) -> Result<()> {
        xml_node.assert_xml_prelim()?;
        with_cell(
            &self.0,
            |c| {
                c.children.push(xml_node.clone());
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    c.push_back(txn, xml_node.clone());
                    Ok(())
                })
            },
        )
    }

    /// Deletes `length` (by default 1) child nodes from `index` on (`OutOfBounds` if there aren't
    /// that many).
    #[uniffi::method(default(txn=None))]
    pub fn delete(&self, index: u32, length: Option<u32>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        let length = length.unwrap_or(1);
        with_cell(
            &self.0,
            |c| {
                check_range(index, length, c.children.len() as u32)?;
                c.children.drain((index as usize)..((index + length) as usize));
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

    /// The first child node, or `null` without children.
    #[uniffi::method(default(txn=None))]
    pub fn first_child(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        with_cell(
            &self.0,
            |c| Ok(c.children.first().cloned()),
            |c| c.readonly(txn, |c, txn| Ok(c.first_child().map(|xml| YXmlChild::from_xml(xml, txn.doc().clone())))),
        )
    }

    /// The next sibling node, or `null` for the last child. `NotIntegrated` for a preliminary
    /// element.
    #[uniffi::method(default(txn=None))]
    pub fn next_sibling(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| {
            Ok(c.siblings(txn).next().map(|node| YXmlChild::from_xml(node, txn.doc().clone())))
        })
    }

    /// The previous sibling node, or `null` for the first child. `NotIntegrated` for a preliminary
    /// element.
    #[uniffi::method(default(txn=None))]
    pub fn prev_sibling(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| {
            Ok(c.siblings(txn).next_back().map(|node| YXmlChild::from_xml(node, txn.doc().clone())))
        })
    }

    /// The parent node, or `null` for none. `NotIntegrated` for a preliminary element.
    #[uniffi::method(default(txn=None))]
    pub fn parent(&self, txn: Option<Arc<YTransaction>>) -> Result<Option<YXmlChild>> {
        integrated_of(&self.0)?.readonly(txn, |c, txn| Ok(c.parent().map(|node| YXmlChild::from_xml(node, txn.doc().clone()))))
    }

    /// Every node inside, depth-first in document order — children, their children… — as handles
    /// (Yjs's `createTreeWalker` without a filter, yrs's `successors`). Nodes embedded in an XML
    /// text aren't walked into, as in Yjs; deleted ones are left out.
    #[uniffi::method(default(txn=None))]
    pub fn tree_walker(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<YXmlChild>> {
        with_cell(
            &self.0,
            |element| YXmlChild::prelim_descendants(&element.children),
            |c| c.read(txn, |element, view| Ok(element.successors(view).map(|node| YXmlChild::from_xml(node, c.doc.clone())).collect())),
        )
    }

    /// The element as XML text.
    #[uniffi::method(name = "toText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        with_cell(
            &self.0,
            |c| {
                let mut str = String::new();
                for child in c.children.iter() {
                    str.push_str(&child.to_text(txn.clone())?);
                }
                Ok(str)
            },
            |c| c.readonly(txn.clone(), |c, txn| Ok(c.get_string(txn))),
        )
    }

    /// Sets attribute `name` to `value`, replacing what it had.
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

    /// The value of attribute `name`, `null` if the element hasn't got it. An attribute holding a
    /// shared type (written by Yjs) comes in its JSON form.
    #[uniffi::method(default(txn=None))]
    pub fn get_attribute(&self, name: &str, txn: Option<Arc<YTransaction>>) -> Result<Option<YValue>> {
        with_cell(
            &self.0,
            |c| Ok(c.attributes.get(name).cloned()),
            |c| c.readonly(txn, |c, txn| Ok(c.get_attribute(txn, name).map(|out| out_into_yvalue(&out, txn)))),
        )
    }

    /// Removes attribute `name`.
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

    /// Every attribute, in no particular order.
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

    /// Calls `observer` after every committed transaction that changed this element itself — its children, its attributes
    /// — with the transaction's origin (Yjs's `observe`). The rules of `YText.observe` hold.
    /// `NotIntegrated` for a preliminary one.
    pub fn observe(&self, observer: Arc<dyn crate::events::YXmlObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlElementRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_xml_node(shared, c.doc.clone(), observer))
    }

    /// Calls `observer` after every committed transaction that changed this element (the nodes in it included) or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlElementRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_deep(shared, c.doc.clone(), observer))
    }
}
