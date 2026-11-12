use crate::collection::{integrated_of, is_alive, is_prelim, with_cell, Integrated, SharedCell, SharedCollection};
use crate::tools::{check_range, Error, Result};
use crate::transaction::YTransaction;
use crate::xml::YXmlChild;
use std::cell::RefCell;
use std::mem;
use std::sync::Arc;
use yrs::types::TYPE_REFS_XML_FRAGMENT;
use yrs::{Doc, GetString, TransactionMut, XmlFragment, XmlFragmentRef};

/// Represents a list of `YXmlElement` and `YXmlText` types.
/// A `YXmlFragment` is similar to a `YXmlElement`, but it does not have a
/// nodeName and it does not have attributes. Though it can be bound to a DOM
/// element - in this case the attributes and the nodeName are not shared
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
#[repr(transparent)]
pub struct YXmlFragment(pub(crate) Arc<SharedCell<Vec<YXmlChild>, XmlFragmentRef>>);

unsafe impl Sync for YXmlFragment {}
unsafe impl Send for YXmlFragment {}

impl YXmlFragment {
    pub fn from_ref(fragment_ref: XmlFragmentRef, doc: Doc) -> Self {
        YXmlFragment(Arc::new(RefCell::new(SharedCollection::integrated(fragment_ref, doc))))
    }

    pub fn new_with_collection(init: SharedCollection<Vec<YXmlChild>, XmlFragmentRef>) -> Self {
        YXmlFragment(Arc::new(RefCell::new(init)))
    }

    pub fn integrate(&self, txn: &mut TransactionMut, xml_fragment: XmlFragmentRef) {
        let doc = txn.doc().clone();

        let old_value = {
            let mut guard = self.0.borrow_mut();
            mem::replace(&mut *guard, SharedCollection::Integrated(Integrated::new(xml_fragment.clone(), doc)))
        };

        if let SharedCollection::Prelim(raw) = old_value {
            for child in raw {
                xml_fragment.push_back(txn, child.clone());
            }
        };
    }
}

#[uniffi::export]
impl YXmlFragment {
    /// A preliminary fragment of `children` (preliminary nodes, else `AlreadyIntegrated`).
    #[uniffi::constructor]
    pub fn new(children: Vec<YXmlChild>) -> Result<Self> {
        for child in children.iter() {
            child.assert_xml_prelim()?;
        }
        Ok(YXmlFragment::new_with_collection(SharedCollection::prelim(children)))
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_XML_FRAGMENT
    }

    /// Whether this is a preliminary fragment, not in a document yet.
    #[inline]
    pub fn prelim(&self) -> bool {
        is_prelim(&self.0)
    }

    /// Whether the fragment is in its document, read in `txn`: `false` for a preliminary fragment
    /// or a deleted one.
    pub fn alive(&self, txn: &YTransaction) -> Result<bool> {
        is_alive(&self.0, txn)
    }

    /// The number of child nodes.
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        with_cell(&self.0, |c| Ok(c.len() as u32), |c| c.readonly(txn, |c, txn| Ok(c.len(txn))))
    }

    /// Inserts a preliminary node (`AlreadyIntegrated` otherwise) at `index` (0 ..= `length()`,
    /// else `OutOfBounds`).
    #[uniffi::method(default(txn=None))]
    pub fn insert(&self, index: u32, xml_node: YXmlChild, txn: Option<Arc<YTransaction>>) -> Result<()> {
        xml_node.assert_xml_prelim()?;
        with_cell(
            &self.0,
            |c| {
                if index as usize > c.len() {
                    return Err(Error::OutOfBounds);
                }
                c.insert(index as usize, xml_node.clone());
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
                c.push(xml_node.clone());
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
                check_range(index, length, c.len() as u32)?;
                c.drain((index as usize)..((index + length) as usize));
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
            |c| Ok(c.first().cloned()),
            |c| c.readonly(txn, |c, txn| Ok(c.first_child().map(|xml| YXmlChild::from_xml(xml, txn.doc().clone())))),
        )
    }

    /// Every node inside, depth-first in document order — children, their children… — as handles
    /// (Yjs's `createTreeWalker` without a filter, yrs's `successors`). Nodes embedded in an XML
    /// text aren't walked into, as in Yjs; deleted ones are left out.
    #[uniffi::method(default(txn=None))]
    pub fn tree_walker(&self, txn: Option<Arc<YTransaction>>) -> Result<Vec<YXmlChild>> {
        with_cell(
            &self.0,
            |children| YXmlChild::prelim_descendants(children),
            |c| c.read(txn, |fragment, view| Ok(fragment.successors(view).map(|node| YXmlChild::from_xml(node, c.doc.clone())).collect())),
        )
    }

    /// The fragment as XML text.
    #[uniffi::method(name = "toText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        with_cell(
            &self.0,
            |c| {
                let mut str = String::new();
                for child in c.iter() {
                    str.push_str(&child.to_text(txn.clone())?);
                }
                Ok(str)
            },
            |c| c.readonly(txn.clone(), |c, txn| Ok(c.get_string(txn))),
        )
    }

    /// Calls `observer` after every committed transaction that changed this fragment itself — its children
    /// — with the transaction's origin (Yjs's `observe`). The rules of `YText.observe` hold.
    /// `NotIntegrated` for a preliminary one.
    pub fn observe(&self, observer: Arc<dyn crate::events::YXmlObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlFragmentRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_xml_node(shared, c.doc.clone(), observer))
    }

    /// Calls `observer` after every committed transaction that changed this fragment (the nodes in it included) or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> crate::observers::ObserveResult {
        let c = integrated_of(&self.0)?;
        let shared: XmlFragmentRef = c.read(None, |shared, _| Ok(shared.clone()))?;
        Ok(crate::events::observe_deep(shared, c.doc.clone(), observer))
    }
}
