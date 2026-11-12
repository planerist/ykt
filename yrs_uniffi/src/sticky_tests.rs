//! Relative positions through ykt's API (the same calls the Kotlin and Swift bindings make): what
//! `assoc` does with an insert exactly at the position, surviving local and remote inserts and
//! deletes, a deleted range around it, the start and the end, UTF-16 offsets, encode/decode (Yjs's
//! bytes) and decoding on another replica, transactions, and bytes that aren't a position.

use crate::doc::{YDoc, YDocOptions};
use crate::snapshots::{apply_update_v2, encode_state_as_update_v2};
use crate::sticky::{YAssoc, YStickyIndex, YStickyOffset};
use crate::text::YText;
use crate::tools::Error;
use std::sync::Arc;

// Helpers ------------------------------------------------------------------------------------------

fn doc(client_id: u64) -> YDoc {
    YDoc::new(Some(YDocOptions {
        client_id: Some(client_id),
        guid: None,
        collection_id: None,
        gc: None,
        auto_load: None,
        should_load: None,
        bytes_offset: None,
    }))
}

fn text_with(doc: &YDoc, content: &str) -> YText {
    let text = doc.get_text("t").unwrap();
    text.insert(0, content, None, None).unwrap();
    text
}

fn pos(text: &YText, index: u32, assoc: YAssoc) -> YStickyIndex {
    text.sticky_index(index, assoc, None).unwrap()
}

/// The position's index in `doc` now.
fn at(doc: &YDoc, pos: &YStickyIndex) -> Option<u32> {
    pos.offset(doc, None).unwrap().map(|o| o.index)
}

fn sync(from: &YDoc, to: &YDoc) {
    apply_update_v2(to, &encode_state_as_update_v2(from, None).unwrap(), None).unwrap();
}

// Assoc: an insert exactly at the position ---------------------------------------------------------

#[test]
fn after_moves_on_with_an_insert_at_its_spot_and_before_stays() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    let before = pos(&text, 1, YAssoc::Before);
    let after = pos(&text, 1, YAssoc::After);
    assert_eq!(at(&doc, &before), Some(1));
    assert_eq!(at(&doc, &after), Some(1));

    text.insert(1, "XY", None, None).unwrap(); // "aXYbc"

    assert_eq!(at(&doc, &before), Some(1), "sticks to 'a': the insert went after it");
    assert_eq!(at(&doc, &after), Some(3), "sticks to 'b': the insert went before it");
    assert_eq!(after.offset(&doc, None).unwrap(), Some(YStickyOffset { index: 3, assoc: YAssoc::After }));
    assert_eq!(before.assoc(), YAssoc::Before);
}

// Local and remote edits ---------------------------------------------------------------------------

#[test]
fn a_position_keeps_its_place_through_local_edits() {
    let doc = doc(1);
    let text = text_with(&doc, "hello world");
    let w = pos(&text, 6, YAssoc::After); // before 'w'

    text.insert(0, "big ", None, None).unwrap(); // "big hello world"
    assert_eq!(at(&doc, &w), Some(10));
    text.delete(0, 6, None).unwrap(); // "llo world"
    assert_eq!(at(&doc, &w), Some(4));
    text.insert(9, "!!", None, None).unwrap(); // after it: no move
    text.format(0, 9, r#"{"bold":true}"#.into(), None).unwrap(); // formatting: no move
    assert_eq!(at(&doc, &w), Some(4));
    assert_eq!(&text.to_string(None).unwrap()[4..], "world!!");
}

#[test]
fn a_position_keeps_its_place_through_remote_edits() {
    let a = doc(1);
    let a_text = text_with(&a, "hello world");
    let b = doc(2);
    sync(&a, &b);
    let b_text = b.get_text("t").unwrap();
    let w = pos(&b_text, 6, YAssoc::Before); // after the space, on replica B

    a_text.insert(0, ">> ", None, None).unwrap();
    a_text.delete(3, 5, None).unwrap(); // ">>  world"
    sync(&a, &b);

    assert_eq!(b_text.to_string(None).unwrap(), ">>  world");
    assert_eq!(at(&b, &w), Some(4));
    // B's own concurrent edit before it counts too.
    b_text.insert(0, "b", None, None).unwrap();
    assert_eq!(at(&b, &w), Some(5));
}

#[test]
fn a_deleted_range_around_it_collapses_it_to_the_cut() {
    for gc in [true, false] {
        let doc = YDoc::new(Some(YDocOptions {
            client_id: Some(1),
            guid: None,
            collection_id: None,
            gc: Some(gc),
            auto_load: None,
            should_load: None,
            bytes_offset: None,
        }));
        let text = text_with(&doc, "hello world");
        let inside_after = pos(&text, 8, YAssoc::After);
        let inside_before = pos(&text, 8, YAssoc::Before);
        let after_it = pos(&text, 11, YAssoc::Before); // the end, sticking to 'd'

        text.delete(6, 5, None).unwrap(); // "hello "

        assert_eq!(at(&doc, &inside_after), Some(6), "gc {gc}");
        assert_eq!(at(&doc, &inside_before), Some(6), "gc {gc}");
        assert_eq!(at(&doc, &after_it), Some(6), "gc {gc}");
        text.insert(6, "there", None, None).unwrap();
        assert_eq!(at(&doc, &inside_after), Some(6), "gc {gc}: stays at the cut");
        text.delete(0, 11, None).unwrap();
        assert_eq!(at(&doc, &inside_after), Some(0), "gc {gc}: everything deleted");
    }
}

// The start and the end --------------------------------------------------------------------------------

#[test]
fn positions_at_the_start() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    let start_before = pos(&text, 0, YAssoc::Before);
    let start_after = pos(&text, 0, YAssoc::After);

    text.insert(0, "X", None, None).unwrap();

    assert_eq!(at(&doc, &start_before), Some(0), "the very start stays the start");
    assert_eq!(at(&doc, &start_after), Some(1), "sticks to 'a'");
}

#[test]
fn positions_at_the_end() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    let end_after = pos(&text, 3, YAssoc::After);
    let end_before = pos(&text, 3, YAssoc::Before);
    assert_eq!(at(&doc, &end_after), Some(3));
    assert_eq!(at(&doc, &end_before), Some(3));

    text.push("de", None, None).unwrap();

    assert_eq!(at(&doc, &end_after), Some(5), "the very end stays the end");
    assert_eq!(at(&doc, &end_before), Some(3), "sticks to 'c'");
}

#[test]
fn positions_in_an_empty_text() {
    let doc = doc(1);
    let text = doc.get_text("t").unwrap();
    let before = pos(&text, 0, YAssoc::Before);
    let after = pos(&text, 0, YAssoc::After);
    assert_eq!(at(&doc, &before), Some(0));
    assert_eq!(at(&doc, &after), Some(0));

    text.insert(0, "abc", None, None).unwrap();

    assert_eq!(at(&doc, &before), Some(0), "the start");
    assert_eq!(at(&doc, &after), Some(3), "the end");
}

#[test]
fn past_the_end_or_preliminary_is_an_error() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    for assoc in [YAssoc::Before, YAssoc::After] {
        assert!(matches!(text.sticky_index(4, assoc, None), Err(Error::OutOfBounds)));
    }
    let prelim = YText::new_with_text(Some("abc".into()));
    assert!(matches!(prelim.sticky_index(0, YAssoc::After, None), Err(Error::NotIntegrated)));
}

#[test]
fn offsets_are_utf16_units() {
    let doc = doc(1);
    let text = text_with(&doc, "😀ab"); // 😀 is two UTF-16 units
    let b = pos(&text, 3, YAssoc::After);
    assert_eq!(at(&doc, &b), Some(3));

    text.insert(0, "😀", None, None).unwrap();

    assert_eq!(at(&doc, &b), Some(5));
}

// Encoding and other replicas ------------------------------------------------------------------------

#[test]
fn encode_decode_round_trip() {
    let doc = doc(1);
    let text = text_with(&doc, "hello");
    for index in 0..=5 {
        for assoc in [YAssoc::Before, YAssoc::After] {
            let p = pos(&text, index, assoc);
            let decoded = YStickyIndex::decode(p.encode()).unwrap();
            assert_eq!(decoded, p);
            assert_eq!(decoded.assoc(), assoc);
            assert_eq!(at(&doc, &decoded), Some(index), "{index} {assoc:?}");
        }
    }
}

#[test]
fn the_encoding_is_yjss() {
    // Y.encodeRelativePosition: tag 0 + item id, or tag 1 + the root type's name; then assoc as a
    // signed var int (0 after, -1 before).
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    assert_eq!(pos(&text, 1, YAssoc::After).encode(), vec![0, 1, 1, 0], "item 1#1 ('b')");
    assert_eq!(pos(&text, 1, YAssoc::Before).encode(), vec![0, 1, 0, 0x41], "item 1#0 ('a')");
    assert_eq!(pos(&text, 3, YAssoc::After).encode(), vec![1, 1, b't', 0], "the end of root type 't'");
    assert_eq!(pos(&text, 0, YAssoc::Before).encode(), vec![1, 1, b't', 0x41], "the start of 't'");
}

#[test]
fn a_position_sent_to_another_replica() {
    let a = doc(1);
    let a_text = text_with(&a, "hello world");
    let sent = pos(&a_text, 6, YAssoc::After).encode();
    let end = pos(&a_text, 11, YAssoc::After).encode();

    // A replica without A's changes can't place it yet.
    let b = doc(2);
    let received = YStickyIndex::decode(sent).unwrap();
    assert_eq!(received.offset(&b, None).unwrap(), None);
    assert_eq!(YStickyIndex::decode(end.clone()).unwrap().offset(&b, None).unwrap(), None, "no text 't' there yet");

    sync(&a, &b);
    assert_eq!(at(&b, &received), Some(6));
    b.get_text("t").unwrap().insert(0, "+", None, None).unwrap();
    assert_eq!(at(&b, &received), Some(7));
    assert_eq!(at(&b, &YStickyIndex::decode(end).unwrap()), Some(12));
}

#[test]
fn bytes_that_arent_a_position_are_an_error() {
    for bytes in [vec![], vec![0xff; 6], vec![9, 1, 1, 0], vec![0, 1], vec![1, 5, b'a']] {
        assert!(matches!(YStickyIndex::decode(bytes.clone()), Err(Error::InvalidData(_))), "{bytes:?}");
    }
}

#[test]
fn positions_are_values() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    assert_eq!(pos(&text, 1, YAssoc::After), pos(&text, 1, YAssoc::After));
    assert_ne!(pos(&text, 1, YAssoc::After), pos(&text, 1, YAssoc::Before));
    assert_ne!(pos(&text, 1, YAssoc::After), pos(&text, 2, YAssoc::After));
    assert_eq!(pos(&text, 1, YAssoc::After).to_string(), "YStickyIndex(after 1#1)");
    assert_eq!(pos(&text, 3, YAssoc::After).to_string(), "YStickyIndex(after, in \"t\")");
}

// Transactions ------------------------------------------------------------------------------------------

#[test]
fn inside_a_transaction() {
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    let c = pos(&text, 2, YAssoc::After);
    let txn = Arc::new(doc.transaction(None).unwrap());
    text.insert(0, "XY", None, Some(txn.clone())).unwrap();

    // Made and read in the transaction, its uncommitted changes count.
    let y = text.sticky_index(2, YAssoc::After, Some(txn.clone())).unwrap();
    assert_eq!(c.offset(&doc, Some(txn.clone())).unwrap().map(|o| o.index), Some(4));
    assert_eq!(y.offset(&doc, Some(txn.clone())).unwrap().map(|o| o.index), Some(2));
    // Without it, the document is locked.
    assert!(matches!(c.offset(&doc, None), Err(Error::Locked)));
    assert!(matches!(text.sticky_index(0, YAssoc::After, None), Err(Error::Locked)));
    // Another document's transaction doesn't do.
    let other = self::doc(2);
    let other_txn = Arc::new(other.transaction(None).unwrap());
    assert!(matches!(c.offset(&doc, Some(other_txn.clone())), Err(Error::WrongDocument)));
    other_txn.commit().unwrap();

    txn.commit().unwrap();
    assert!(matches!(c.offset(&doc, Some(txn.clone())), Err(Error::TransactionCommitted)));
    assert!(matches!(text.sticky_index(0, YAssoc::After, Some(txn)), Err(Error::TransactionCommitted)));
    assert_eq!(at(&doc, &c), Some(4));
}

#[test]
fn reading_a_position_commits_nothing() {
    use crate::observers::{YTransactionCleanupEvent, YTransactionCleanupObserver};
    #[derive(Default)]
    struct Count(std::sync::Mutex<usize>);
    impl YTransactionCleanupObserver for Count {
        fn on_cleanup(&self, _: YTransactionCleanupEvent) {
            *self.0.lock().unwrap() += 1;
        }
    }
    let doc = doc(1);
    let text = text_with(&doc, "abc");
    let count = Arc::new(Count::default());
    let _sub = doc.observe_transaction_cleanup(count.clone()).unwrap();

    let p = pos(&text, 1, YAssoc::After);
    at(&doc, &p);

    assert_eq!(*count.0.lock().unwrap(), 0, "no transactions reach the observers");
}
