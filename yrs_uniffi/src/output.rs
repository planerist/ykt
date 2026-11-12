//! What a document holds where a value can be a shared type: `YOutput`, a plain value (`YValue`) or
//! a handle to the shared type — a text, a map or an array nested in a map or an array (Yjs's
//! `map.set('k', new Y.Map())`), an XML node, a subdocument, a weak link. Maps' and arrays' `getOutput`, their
//! observers and deep events give these; `get` / `toJson` keep giving plain values, a nested type
//! in its JSON form.

use crate::array::YArray;
use crate::attrs::{into_yvalue, YValue};
use crate::collection::{same_doc, Integrated, SharedCell, SharedCollection};
use crate::doc::YDoc;
use crate::map::YMap;
use crate::text::YText;
use crate::weak::YWeakLink;
use crate::xml_elem::YXmlElement;
use crate::xml_frag::YXmlFragment;
use crate::xml_text::YXmlText;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use yrs::types::ToJson;
use yrs::{Doc, Out, ReadTxn, SharedRef};

/// A value read from a document: a plain value, or a handle to a shared type (see the module docs).
/// Handles compare equal when they are the same type of the same document.
#[derive(uniffi::Enum, Clone, Debug, PartialEq, Eq)]
#[uniffi::export(Eq)]
pub enum YOutput {
    Value { value: YValue },
    Text { text: Arc<YText> },
    Map { map: Arc<YMap> },
    Array { array: Arc<YArray> },
    XmlElement { element: Arc<YXmlElement> },
    XmlFragment { fragment: Arc<YXmlFragment> },
    XmlText { text: Arc<YXmlText> },
    /// A subdocument.
    Doc { doc: Arc<YDoc> },
    /// A weak link: a quote, or a link to a map entry.
    WeakLink { link: Arc<YWeakLink> },
}

impl YOutput {
    /// `value` as ykt gives it: plain values as they are, shared types as handles of `doc`. Anything
    /// ykt has no handle for comes in its JSON form.
    pub(crate) fn from_out<T: ReadTxn>(value: Out, doc: &Doc, txn: &T) -> Self {
        match value {
            Out::Any(any) => YOutput::Value { value: into_yvalue(&any) },
            Out::YText(text) => YOutput::Text { text: Arc::new(YText::new(SharedCollection::integrated(text, doc.clone()))) },
            Out::YMap(map) => YOutput::Map { map: Arc::new(YMap(Integrated::new(map, doc.clone()))) },
            Out::YArray(array) => YOutput::Array { array: Arc::new(YArray(Integrated::new(array, doc.clone()))) },
            Out::YXmlElement(element) => YOutput::XmlElement { element: Arc::new(YXmlElement::from_ref(element, doc.clone())) },
            Out::YXmlFragment(fragment) => YOutput::XmlFragment { fragment: Arc::new(YXmlFragment::from_ref(fragment, doc.clone())) },
            Out::YXmlText(text) => YOutput::XmlText { text: Arc::new(YXmlText::from_ref(text, doc.clone())) },
            Out::YDoc(subdoc) => YOutput::Doc { doc: Arc::new(YDoc(subdoc)) },
            Out::YWeakLink(link) => YOutput::WeakLink { link: Arc::new(YWeakLink::from_ref(link, doc.clone())) },
            other => YOutput::Value { value: into_yvalue(&other.to_json(txn)) },
        }
    }
}

/// Handles' identity: the same type (branch) of the same document — two handles of one text are
/// equal; replicas' texts are not. A preliminary type is equal only to itself.
pub(crate) fn same_cell<P, S: SharedRef + 'static>(a: &Arc<SharedCell<P, S>>, b: &Arc<SharedCell<P, S>>) -> bool {
    if Arc::ptr_eq(a, b) {
        return true;
    }
    match (a.try_borrow(), b.try_borrow()) {
        (Ok(a), Ok(b)) => match (&*a, &*b) {
            (SharedCollection::Integrated(a), SharedCollection::Integrated(b)) => same_integrated(a, b),
            _ => false,
        },
        _ => false,
    }
}

pub(crate) fn same_integrated<S: SharedRef + 'static>(a: &Integrated<S>, b: &Integrated<S>) -> bool {
    same_doc(&a.doc, &b.doc) && a.id() == b.id()
}

/// A hash consistent with `same_cell`: the branch's id (a preliminary type's: its address — it
/// changes when the type is integrated, so don't keep one as a key across its insertion).
pub(crate) fn hash_cell<P, S: SharedRef + 'static, H: Hasher>(cell: &Arc<SharedCell<P, S>>, state: &mut H) {
    match cell.try_borrow().as_deref() {
        Ok(SharedCollection::Integrated(c)) => hash_integrated(c, state),
        _ => (Arc::as_ptr(cell) as usize).hash(state),
    }
}

pub(crate) fn hash_integrated<S: SharedRef + 'static, H: Hasher>(c: &Integrated<S>, state: &mut H) {
    (c.doc.clone().as_raw() as usize).hash(state);
    c.id().hash(state);
}

macro_rules! cell_identity {
    ($type:ty, $field:tt) => {
        impl PartialEq for $type {
            fn eq(&self, other: &Self) -> bool {
                same_cell(&self.$field, &other.$field)
            }
        }
        impl Eq for $type {}
        impl Hash for $type {
            fn hash<H: Hasher>(&self, state: &mut H) {
                hash_cell(&self.$field, state)
            }
        }
        impl std::fmt::Debug for $type {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                match self.$field.try_borrow().as_deref() {
                    Ok(SharedCollection::Integrated(c)) => write!(f, "{}({:?})", stringify!($type), c.id()),
                    _ => write!(f, "{}(preliminary)", stringify!($type)),
                }
            }
        }
    };
}

macro_rules! integrated_identity {
    ($type:ty) => {
        impl PartialEq for $type {
            fn eq(&self, other: &Self) -> bool {
                same_integrated(&self.0, &other.0)
            }
        }
        impl Eq for $type {}
        impl Hash for $type {
            fn hash<H: Hasher>(&self, state: &mut H) {
                hash_integrated(&self.0, state)
            }
        }
        impl std::fmt::Debug for $type {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}({:?})", stringify!($type), self.0.id())
            }
        }
    };
}

cell_identity!(YText, inner);
cell_identity!(YXmlElement, 0);
cell_identity!(YXmlFragment, 0);
cell_identity!(YXmlText, 0);
cell_identity!(YWeakLink, 0);
integrated_identity!(YMap);
integrated_identity!(YArray);

impl PartialEq for YDoc {
    fn eq(&self, other: &Self) -> bool {
        same_doc(&self.0, &other.0)
    }
}

impl Eq for YDoc {}

impl Hash for YDoc {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self.0.clone().as_raw() as usize).hash(state)
    }
}

impl std::fmt::Debug for YDoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "YDoc({})", self.0.guid())
    }
}

// YValue holds doubles: equality as `PartialEq` has it (NaN != NaN), which is what Swift's and
// Kotlin's would be.
impl Eq for YValue {}
