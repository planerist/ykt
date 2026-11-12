use crate::attrs::{from_yattrs, into_yattrs, out_into_yvalue, parse_attrs, YAttributes, YValue};
use crate::array::YArray;
use crate::collection::{integrated_of, is_alive, is_prelim, with_cell, Integrated, SharedCell, SharedCollection, YBranchId};
use crate::map::{array_prelim, map_prelim, YMap};
use crate::delta::{y_into_delta, YDelta};
use crate::observers::{origin_bytes, subscription_key, text_delta, ObserveResult, YSubscription, YTextEvent, YTextObserver};
use crate::snapshots::YSnapshot;
use crate::sticky::{YAssoc, YStickyIndex};
use crate::tools::{check_range, utf16_len, utf16_offset, Error, Result};
use crate::transaction::YTransaction;
use std::cell::RefCell;
use std::ops::Deref;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use yrs::types::TYPE_REFS_TEXT;
use yrs::types::Attrs;
use yrs::block::{EmbedPrelim, Prelim};
use yrs::{Any, GetString, Observable, Snapshot, Text, TextPrelim, TextRef};

/// A shared data type used for collaborative text editing. It enables multiple users to add and
/// remove chunks of text in efficient manner. This type is internally represented as a mutable
/// double-linked list of text chunks - an optimization occurs during `YTransaction.commit`, which
/// allows to squash multiple consecutively inserted characters together as a single chunk of text
/// even between transaction boundaries in order to preserve more efficient memory model.
///
/// Indexes and lengths count UTF-16 code units (unless the document was made with
/// `bytesOffset = true`), in a preliminary text too.
///
/// Like all Yrs shared data types, `YText` is resistant to the problem of interleaving (situation
/// when characters inserted one after another may interleave with other peers concurrent inserts
/// after merging all updates together). In case of Yrs conflict resolution is solved by using
/// unique document id to determine correct and consistent ordering.
#[derive(uniffi::Object)]
#[uniffi::export(Eq, Hash)]
#[repr(transparent)]
pub struct YText {
    pub(crate) inner: Arc<SharedCell<String, TextRef>>,
}

unsafe impl Sync for YText {}
unsafe impl Send for YText {}

impl YText {
    pub fn new(init: SharedCollection<String, TextRef>) -> Self {
        YText {
            inner: Arc::new(RefCell::new(init)),
        }
    }

    fn insert_attrs(&self, index: u32, chunk: &str, attributes: Option<Attrs>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        let formatted = attributes.as_ref().is_some_and(|a| !a.is_empty());
        with_cell(
            &self.inner,
            |s| {
                if formatted {
                    return Err(Error::NotIntegrated);
                }
                let at = utf16_offset(s, index)?;
                s.insert_str(at, chunk);
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    if index > c.len(txn) {
                        return Err(Error::OutOfBounds);
                    }
                    match attributes {
                        Some(attrs) => c.insert_with_attributes(txn, index, chunk, attrs),
                        None => c.insert(txn, index, chunk),
                    }
                    Ok(())
                })
            },
        )
    }

    fn format_attrs(&self, index: u32, length: u32, attrs: Attrs, txn: Option<Arc<YTransaction>>) -> Result<()> {
        integrated_of(&self.inner)?.mutably(txn, |c, txn| {
            check_range(index, length, c.len(txn))?;
            c.format(txn, index, length, attrs);
            Ok(())
        })
    }

    /// Embeds a new shared type at `index` and gives its handle.
    fn embed_shared<P, R>(
        &self,
        index: u32,
        prelim: P,
        attributes: Option<YAttributes>,
        txn: Option<Arc<YTransaction>>,
        handle: impl FnOnce(P::Return, yrs::Doc) -> R,
    ) -> Result<R>
    where
        P: Prelim + Into<EmbedPrelim<P>>,
    {
        let c = integrated_of(&self.inner)?;
        let doc = c.doc.clone();
        c.mutably(txn, |c, txn| {
            if index > c.len(txn) {
                return Err(Error::OutOfBounds);
            }
            let nested = match attributes {
                Some(attrs) if !attrs.is_empty() => c.insert_embed_with_attributes(txn, index, prelim, from_yattrs(&attrs)),
                _ => c.insert_embed(txn, index, prelim),
            };
            Ok(handle(nested, doc))
        })
    }

    /// The text's shared ref and its document, resolved in a read-only transaction.
    pub(crate) fn integrated_ref(&self) -> Result<(TextRef, yrs::Doc)> {
        let c = integrated_of(&self.inner)?;
        c.read(None, |text, _| Ok((text.clone(), c.doc.clone())))
    }
}

#[uniffi::export]
impl YText {
    /// Creates a new preliminary instance of a `YText` shared data type, with its state initialized
    /// to provided parameter.
    ///
    /// Preliminary instances can be nested into other shared data types such as `YArray` and `YMap`.
    /// Once a preliminary instance has been inserted this way, it becomes integrated into ywasm
    /// document store and cannot be nested again: attempt to do so will result in an exception.
    #[uniffi::constructor]
    pub fn new_with_text(init: Option<String>) -> Self {
        YText::new(SharedCollection::prelim(init.unwrap_or_default()))
    }

    #[inline]
    pub fn get_type(&self) -> u8 {
        TYPE_REFS_TEXT
    }

    /// Whether this is a preliminary text, not in a document yet.
    #[inline]
    pub fn prelim(&self) -> bool {
        is_prelim(&self.inner)
    }

    /// Whether the text is in its document, read in `txn`: `false` for a preliminary text or one
    /// deleted from its parent.
    pub fn alive(&self, txn: &YTransaction) -> Result<bool> {
        is_alive(&self.inner, txn)
    }

    /// The length of the text (UTF-16 units).
    #[uniffi::method(default(txn=None))]
    pub fn length(&self, txn: Option<Arc<YTransaction>>) -> Result<u32> {
        with_cell(&self.inner, |s| Ok(utf16_len(s)), |c| c.readonly(txn, |c, txn| Ok(c.len(txn))))
    }

    /// The text as a string, without its formatting.
    #[uniffi::method(name = "toText", default(txn=None))]
    pub fn to_string(&self, txn: Option<Arc<YTransaction>>) -> Result<String> {
        with_cell(&self.inner, |s| Ok(s.clone()), |c| c.readonly(txn, |c, txn| Ok(c.get_string(txn))))
    }

    /// Inserts `chunk` at `index` (0 ..= `length()`, else `OutOfBounds`), with formatting
    /// `attributes` given as a JSON object (`InvalidAttributes` otherwise; `insertWithAttributes`
    /// takes them typed). A preliminary text takes no attributes (`NotIntegrated`).
    #[uniffi::method(default(attributes=None, txn=None))]
    pub fn insert(
        &self,
        index: u32,
        chunk: &str,
        attributes: Option<String>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        self.insert_attrs(index, chunk, parse_attrs(attributes)?, txn)
    }

    /// Inserts `chunk` at `index` (0 ..= `length()`, else `OutOfBounds`) with exactly these
    /// formatting `attributes`, as Yjs's `insert(index, text, attributes)`: an attribute of the text
    /// around it that the map lacks (or has as `null`) isn't taken over — an empty map inserts
    /// plain text even inside bold — whereas `insert` without attributes takes the formatting
    /// around it. A preliminary text takes an empty map only (`NotIntegrated`).
    #[uniffi::method(default(txn=None))]
    pub fn insert_with_attributes(
        &self,
        index: u32,
        chunk: &str,
        attributes: YAttributes,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        self.insert_attrs(index, chunk, Some(from_yattrs(&attributes)), txn)
    }

    /// Inserts an embed — any plain value: a map (a mention, an image), a string, a number… — at
    /// `index`, with formatting `attributes` (Yjs's `insertEmbed`). It takes one unit of the
    /// text's length and comes back as one insert in deltas. `NotIntegrated` for a preliminary
    /// text.
    #[uniffi::method(default(attributes=None, txn=None))]
    pub fn insert_embed(
        &self,
        index: u32,
        embed: YValue,
        attributes: Option<YAttributes>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        integrated_of(&self.inner)?.mutably(txn, |c, txn| {
            if index > c.len(txn) {
                return Err(Error::OutOfBounds);
            }
            let embed: Any = embed.into();
            match attributes {
                Some(attrs) if !attrs.is_empty() => {
                    c.insert_embed_with_attributes(txn, index, embed, from_yattrs(&attrs));
                }
                _ => {
                    c.insert_embed(txn, index, embed);
                }
            }
            Ok(())
        })
    }

    /// Embeds a new text with `text` in it at `index`, with formatting `attributes` (Yjs's
    /// `insertEmbed(index, new Y.Text())`), and returns it. Deltas give it in its JSON form;
    /// `observeDeep` hears its changes.
    #[uniffi::method(default(text=None, attributes=None, txn=None))]
    pub fn insert_embed_text(
        &self,
        index: u32,
        text: Option<String>,
        attributes: Option<YAttributes>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<Arc<YText>> {
        self.embed_shared(index, TextPrelim::new(text.unwrap_or_default()), attributes, txn, |text, doc| {
            Arc::new(YText::new(SharedCollection::integrated(text, doc)))
        })
    }

    /// Embeds a new map with `entries` at `index` (as `insertEmbedText`), and returns it.
    #[uniffi::method(default(entries=None, attributes=None, txn=None))]
    pub fn insert_embed_map(
        &self,
        index: u32,
        entries: Option<YAttributes>,
        attributes: Option<YAttributes>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<Arc<YMap>> {
        self.embed_shared(index, map_prelim(entries), attributes, txn, |map, doc| Arc::new(YMap(Integrated::new(map, doc))))
    }

    /// Embeds a new array with `values` at `index` (as `insertEmbedText`), and returns it.
    #[uniffi::method(default(values=None, attributes=None, txn=None))]
    pub fn insert_embed_array(
        &self,
        index: u32,
        values: Option<Vec<YValue>>,
        attributes: Option<YAttributes>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<Arc<YArray>> {
        self.embed_shared(index, array_prelim(values), attributes, txn, |array, doc| Arc::new(YArray(Integrated::new(array, doc))))
    }

    /// Formats `length` units from `index` on with `attributes`, a JSON object (a `null` value
    /// removes that attribute; `formatWithAttributes` takes them typed). `OutOfBounds` past the
    /// end; `NotIntegrated` for a preliminary text.
    #[uniffi::method(default(txn=None))]
    pub fn format(
        &self,
        index: u32,
        length: u32,
        attributes: String,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        self.format_attrs(index, length, parse_attrs(Some(attributes))?.unwrap_or_default(), txn)
    }

    /// Formats `length` units from `index` on with `attributes`: each one set, or removed by a
    /// `null` value (Yjs's `format`). `OutOfBounds` past the end; `NotIntegrated` for a
    /// preliminary text.
    #[uniffi::method(default(txn=None))]
    pub fn format_with_attributes(
        &self,
        index: u32,
        length: u32,
        attributes: YAttributes,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        self.format_attrs(index, length, from_yattrs(&attributes), txn)
    }

    /// The text's identity in its document, the same on every replica: its root name, or for a
    /// nested text the id of the item that holds it. `NotIntegrated` for a preliminary text.
    pub fn id(&self) -> Result<YBranchId> {
        Ok(integrated_of(&self.inner)?.id().into())
    }

    /// Appends `chunk` at the end, with formatting `attributes` as in `insert`.
    #[uniffi::method(default(attributes=None, txn=None))]
    pub fn push(
        &self,
        chunk: &str,
        attributes: Option<String>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<()> {
        let attributes = parse_attrs(attributes)?;
        let formatted = attributes.as_ref().is_some_and(|a| !a.is_empty());
        with_cell(
            &self.inner,
            |s| {
                if formatted {
                    return Err(Error::NotIntegrated);
                }
                s.push_str(chunk);
                Ok(())
            },
            |c| {
                c.mutably(txn, |c, txn| {
                    let len = c.len(txn);
                    match attributes {
                        Some(attrs) => c.insert_with_attributes(txn, len, chunk, attrs),
                        None => c.push(txn, chunk),
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
            &self.inner,
            |s| {
                let end = index.checked_add(length).ok_or(Error::OutOfBounds)?;
                let (from, to) = (utf16_offset(s, index)?, utf16_offset(s, end)?);
                s.drain(from..to);
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

    /// The text as a delta of inserts with their formatting attributes; between two snapshots, what
    /// changed (as Yjs's `toDelta(snapshot, prevSnapshot)`). An embedded shared type comes in its
    /// JSON form. `NotIntegrated` for a preliminary text.
    #[uniffi::method(default(snapshot=None,prev_snapshot=None,txn=None))]
    pub fn to_delta(
        &self,
        snapshot: Option<Arc<YSnapshot>>,
        prev_snapshot: Option<Arc<YSnapshot>>,
        txn: Option<Arc<YTransaction>>,
    ) -> Result<Vec<YDelta>> {
        integrated_of(&self.inner)?.mutably(txn, |c, txn| {
            let hi: Option<Snapshot> = snapshot.map(|snap| snap.deref().deref().clone());
            let lo: Option<Snapshot> = prev_snapshot.map(|snap| snap.deref().deref().clone());
            let delta = c.diff_range(txn, hi.as_ref(), lo.as_ref(), |change| change);
            Ok(delta
                .into_iter()
                .map(|d| YDelta::YInsert(out_into_yvalue(&d.insert, &*txn), d.attributes.map(|attrs| into_yattrs(*attrs))))
                .collect())
        })
    }

    /// Calls `observer` after every committed transaction that changed this text, with the change
    /// as a delta and the transaction's origin (Yjs's `ytext.observe`). The callback runs on the
    /// committing thread while the document is still locked: it must not read or write the
    /// document (see the `observers` module). The subscription stops on `unsubscribe()` or when it
    /// is released.
    pub fn observe(&self, observer: Arc<dyn YTextObserver>) -> ObserveResult {
        let c = integrated_of(&self.inner)?;
        let text: TextRef = c.read(None, |text, _| Ok(text.clone()))?;
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        text.observe(key.clone(), move |txn, event| {
            if !running.load(Ordering::SeqCst) {
                return;
            }
            observer.on_change(YTextEvent {
                delta: text_delta(event.delta(txn), txn),
                origin: origin_bytes(txn),
            });
        });
        // The handle keeps the document alive: `text` points into its store, which must
        // outlive the unobserve (a subscription may be released after the document).
        let doc = c.doc.clone();
        Ok(YSubscription::new(active, move || {
            let removed = text.unobserve(key);
            drop(doc);
            removed
        }))
    }

    /// Calls `observer` after every committed transaction that changed this text (its embedded types included) or any shared
    /// type in it, with an event per changed type (target, path from here, change) and the
    /// transaction's origin (Yjs's `observeDeep`). The rules of `observe` hold.
    pub fn observe_deep(&self, observer: Arc<dyn crate::events::YDeepObserver>) -> ObserveResult {
        let c = integrated_of(&self.inner)?;
        let text: TextRef = c.read(None, |text, _| Ok(text.clone()))?;
        Ok(crate::events::observe_deep(text, c.doc.clone(), observer))
    }

    /// A relative position at `index` (UTF-16 units) of this text, as Yjs's
    /// `createRelativePositionFromTypeIndex`: it keeps its place through later edits, local and
    /// remote (see `YStickyIndex`). `assoc` says which neighbour it sticks to. Throws `OutOfBounds`
    /// past the end; at the end with `After` it stays at the end.
    #[uniffi::method(default(txn=None))]
    pub fn sticky_index(&self, index: u32, assoc: YAssoc, txn: Option<Arc<YTransaction>>) -> Result<YStickyIndex> {
        integrated_of(&self.inner)?.read(txn, |text, view| YStickyIndex::at(text, view, index, text.len(view), assoc))
    }

    /// Applies a delta (Quill's, Yjs's `applyDelta`): retains (formatting with their attributes),
    /// inserts with attributes, deletes. A retain past the end stops there; a delete past the end
    /// throws `OutOfBounds`, before anything is applied.
    #[uniffi::method(default(txn=None))]
    pub fn apply_delta(&self, delta: Vec<YDelta>, txn: Option<Arc<YTransaction>>) -> Result<()> {
        integrated_of(&self.inner)?.mutably(txn, |c, txn| {
            check_delta(&delta, c.len(txn), txn.doc().offset_kind())?;
            c.apply_delta(txn, delta.iter().map(y_into_delta));
            Ok(())
        })
    }
}

/// `OutOfBounds` if `delta` deletes past the end of a text of `len` (yrs would panic half-way). A
/// retain past the end stops at it, as yrs's does.
fn check_delta(delta: &[YDelta], len: u32, offsets: yrs::OffsetKind) -> Result<()> {
    let (mut pos, mut len) = (0u32, len);
    for step in delta {
        match step {
            YDelta::YRetain(n, _) => pos = pos.saturating_add(*n).min(len),
            YDelta::YInsert(value, _) => {
                let n = match value {
                    YValue::String(s) => match offsets {
                        yrs::OffsetKind::Bytes => s.len() as u32,
                        yrs::OffsetKind::Utf16 => utf16_len(s),
                    },
                    _ => 1,
                };
                pos += n;
                len += n;
            }
            YDelta::YDelete(n) => {
                check_range(pos, *n, len)?;
                len -= n;
            }
        }
    }
    Ok(())
}
