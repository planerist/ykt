//! Update utilities without a document, through ykt's API (the same calls the Kotlin and Swift
//! bindings make): merging (= applying one by one, in any order), diffing against a replica's state
//! vector (exactly what it lacks, deletions included), state vectors read from updates (= the
//! document's), and bytes that aren't updates (an error, never a crash). Every case in v1 and v2.

use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YUpdateEvent, YUpdateObserver};
use crate::snapshots::{apply_update, apply_update_v2, encode_state_as_update, encode_state_as_update_v2, encode_state_vector};
use crate::tools::Error;
use crate::updates::*;
use std::sync::{Arc, Mutex};
use yrs::updates::decoder::Decode;
use yrs::StateVector;

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

/// v1 or v2: the encoding a case runs in.
#[derive(Clone, Copy, Debug)]
enum V {
    One,
    Two,
}

const BOTH: [V; 2] = [V::One, V::Two];

fn apply(v: V, doc: &YDoc, update: &[u8]) {
    match v {
        V::One => apply_update(doc, update, None),
        V::Two => apply_update_v2(doc, update, None),
    }
    .unwrap()
}

fn full_state(v: V, doc: &YDoc) -> Vec<u8> {
    match v {
        V::One => encode_state_as_update(doc, None),
        V::Two => encode_state_as_update_v2(doc, None),
    }
    .unwrap()
}

fn merge(v: V, updates: &[Vec<u8>]) -> crate::tools::Result<Vec<u8>> {
    match v {
        V::One => merge_updates_v1(updates.to_vec()),
        V::Two => merge_updates_v2(updates.to_vec()),
    }
}

fn diff(v: V, update: &[u8], sv: &[u8]) -> crate::tools::Result<Vec<u8>> {
    match v {
        V::One => diff_update_v1(update.to_vec(), sv.to_vec()),
        V::Two => diff_update_v2(update.to_vec(), sv.to_vec()),
    }
}

fn sv_from_update(v: V, update: &[u8]) -> crate::tools::Result<Vec<u8>> {
    match v {
        V::One => encode_state_vector_from_update_v1(update.to_vec()),
        V::Two => encode_state_vector_from_update_v2(update.to_vec()),
    }
}

/// A state vector's content (its bytes follow a hash map's order).
fn sv(bytes: &[u8]) -> StateVector {
    StateVector::decode_v1(bytes).unwrap()
}

#[derive(Default)]
struct Updates(Mutex<Vec<Vec<u8>>>);

impl YUpdateObserver for Updates {
    fn on_update(&self, event: YUpdateEvent) {
        self.0.lock().unwrap().push(event.update);
    }
}

/// A document edited in five transactions (inserts, a format, deletes), with every transaction's
/// update as it was sent.
fn edited(v: V, client: u64) -> (YDoc, Vec<Vec<u8>>) {
    let doc = doc(client);
    let updates = Arc::new(Updates::default());
    let sub = match v {
        V::One => doc.observe_update_v1(updates.clone()),
        V::Two => doc.observe_update_v2(updates.clone()),
    }
    .unwrap();
    let text = doc.get_text("t").unwrap();
    text.insert(0, "hello", None, None).unwrap();
    text.insert(5, " world", None, None).unwrap();
    text.format(0, 5, r#"{"bold":true}"#.into(), None).unwrap();
    text.delete(0, 1, None).unwrap();
    text.insert(10, "!", None, None).unwrap();
    drop(sub);
    let updates = updates.0.lock().unwrap().clone();
    (doc, updates)
}

fn text_of(doc: &YDoc) -> String {
    doc.get_text("t").unwrap().to_string(None).unwrap()
}

// Merging --------------------------------------------------------------------------------------------

#[test]
fn merging_updates_is_applying_them_one_by_one() {
    for v in BOTH {
        let (source, updates) = edited(v, 1);
        assert_eq!(updates.len(), 5);
        let one_by_one = doc(2);
        for u in &updates {
            apply(v, &one_by_one, u);
        }
        let merged = doc(3);
        apply(v, &merged, &merge(v, &updates).unwrap());

        assert_eq!(text_of(&merged), "ello world!", "{v:?}");
        assert_eq!(text_of(&merged), text_of(&one_by_one));
        assert_eq!(
            merged.get_text("t").unwrap().to_delta(None, None, None).unwrap(),
            source.get_text("t").unwrap().to_delta(None, None, None).unwrap(),
            "the formatting too"
        );
        assert_eq!(sv(&encode_state_vector(&merged).unwrap()), sv(&encode_state_vector(&source).unwrap()));
    }
}

#[test]
fn merging_in_any_order_gives_the_same_document() {
    for v in BOTH {
        let (_, updates) = edited(v, 1);
        let mut reversed = updates.clone();
        reversed.reverse();
        let merged = doc(2);
        apply(v, &merged, &merge(v, &reversed).unwrap());

        assert_eq!(text_of(&merged), "ello world!", "{v:?}");
    }
}

#[test]
fn merging_concurrent_edits_of_two_replicas() {
    for v in BOTH {
        let (a, a_updates) = edited(v, 1);
        let b = doc(2);
        b.get_text("t").unwrap().insert(0, "B", None, None).unwrap();
        let mut all = a_updates.clone();
        all.push(full_state(v, &b));

        let merged = doc(3);
        apply(v, &merged, &merge(v, &all).unwrap());
        apply(v, &a, &full_state(v, &b));

        assert_eq!(text_of(&merged), text_of(&a), "{v:?}");
        assert_eq!(sv(&encode_state_vector(&merged).unwrap()), sv(&encode_state_vector(&a).unwrap()));
    }
}

#[test]
fn a_merge_of_merges_and_a_merge_of_one() {
    for v in BOTH {
        let (_, updates) = edited(v, 1);
        let halves = vec![merge(v, &updates[..2]).unwrap(), merge(v, &updates[2..]).unwrap()];
        let target = doc(2);
        apply(v, &target, &merge(v, &halves).unwrap());
        assert_eq!(text_of(&target), "ello world!", "{v:?}");

        let single = doc(3);
        apply(v, &single, &merge(v, &updates[..1]).unwrap());
        assert_eq!(text_of(&single), "hello");
    }
}

#[test]
fn merging_nothing_is_an_empty_update() {
    for v in BOTH {
        let empty = merge(v, &[]).unwrap();
        let target = doc(1);
        let updates = Arc::new(Updates::default());
        let _sub = target.observe_update_v1(updates.clone()).unwrap();

        apply(v, &target, &empty);

        assert_eq!(text_of(&target), "");
        assert!(updates.0.lock().unwrap().is_empty(), "{v:?}: applying it changes nothing");
        assert_eq!(sv(&sv_from_update(v, &empty).unwrap()), StateVector::default());
    }
}

// Diffing ----------------------------------------------------------------------------------------------

#[test]
fn a_diff_against_a_replicas_state_vector_is_what_it_lacks() {
    for v in BOTH {
        let (source, updates) = edited(v, 1);
        let full = merge(v, &updates).unwrap();
        // The replica has the first two transactions only.
        let replica = doc(2);
        apply(v, &replica, &updates[0]);
        apply(v, &replica, &updates[1]);
        let replica_sv = encode_state_vector(&replica).unwrap();

        let missing = diff(v, &full, &replica_sv).unwrap();
        assert!(missing.len() < full.len(), "{v:?}: not the whole update");
        // Alone, on an empty document, it can't be integrated: it builds on what the replica has.
        let fresh = doc(3);
        apply(v, &fresh, &missing);
        assert_eq!(text_of(&fresh), "");

        apply(v, &replica, &missing);
        assert_eq!(text_of(&replica), "ello world!");
        assert_eq!(
            replica.get_text("t").unwrap().to_delta(None, None, None).unwrap(),
            source.get_text("t").unwrap().to_delta(None, None, None).unwrap()
        );
        // Now nothing is missing: the next diff has no insertions.
        let nothing = diff(v, &full, &encode_state_vector(&replica).unwrap()).unwrap();
        assert_eq!(sv(&sv_from_update(v, &nothing).unwrap()), StateVector::default());
    }
}

#[test]
fn a_diff_keeps_the_deletions() {
    for v in BOTH {
        let a = doc(1);
        let text = a.get_text("t").unwrap();
        text.insert(0, "abc", None, None).unwrap();
        let replica = doc(2);
        apply(v, &replica, &full_state(v, &a));
        text.delete(0, 1, None).unwrap(); // a delete adds nothing to the state vector

        let missing = diff(v, &full_state(v, &a), &encode_state_vector(&replica).unwrap()).unwrap();
        apply(v, &replica, &missing);

        assert_eq!(text_of(&replica), "bc", "{v:?}");
    }
}

#[test]
fn a_diff_against_an_empty_state_vector_is_everything() {
    for v in BOTH {
        let (_, updates) = edited(v, 1);
        let full = merge(v, &updates).unwrap();
        let empty_sv = encode_state_vector(&doc(9)).unwrap();

        let everything = diff(v, &full, &empty_sv).unwrap();
        let fresh = doc(2);
        apply(v, &fresh, &everything);

        assert_eq!(text_of(&fresh), "ello world!", "{v:?}");
    }
}

#[test]
fn a_diff_takes_the_v1_state_vector_of_a_doc_or_a_transaction() {
    let (source, updates) = edited(V::Two, 1);
    let replica = doc(2);
    apply(V::Two, &replica, &updates[0]);
    let txn = replica.transaction(None).unwrap();
    let from_txn = txn.state_vector_v1().unwrap();
    txn.commit().unwrap();

    let missing = diff_update_v2(full_state(V::Two, &source), from_txn).unwrap();
    apply(V::Two, &replica, &missing);

    assert_eq!(text_of(&replica), "ello world!");
}

// State vectors ----------------------------------------------------------------------------------------

#[test]
fn the_state_vector_of_an_update_is_the_documents() {
    for v in BOTH {
        let (a, _) = edited(v, 1);
        let b = doc(2);
        b.get_text("t").unwrap().insert(0, "B", None, None).unwrap();
        apply(v, &a, &full_state(v, &b));

        let from_update = sv_from_update(v, &full_state(v, &a)).unwrap();

        assert_eq!(sv(&from_update), sv(&encode_state_vector(&a).unwrap()), "{v:?}");
        assert_eq!(sv(&from_update).get(&yrs::block::ClientID::new(2)), 1, "both clients");
    }
}

#[test]
fn the_state_vector_of_one_transactions_update_is_empty_unless_it_starts_the_history() {
    // As Yjs: a state vector counts a client's clocks from 0 up; an update with a later part only
    // (one transaction's) doesn't say how far a document that has it got.
    for v in BOTH {
        let (_, updates) = edited(v, 1);
        assert_eq!(sv(&sv_from_update(v, &updates[0]).unwrap()).get(&yrs::block::ClientID::new(1)), 5);
        assert_eq!(sv(&sv_from_update(v, &updates[1]).unwrap()), StateVector::default());
    }
}

// Bytes that aren't updates ------------------------------------------------------------------------------

fn is_invalid_data<T: std::fmt::Debug>(result: crate::tools::Result<T>) -> bool {
    matches!(result, Err(Error::InvalidData(_)))
}

#[test]
fn garbage_is_an_error() {
    let garbage: [&[u8]; 4] = [&[], &[0xff; 12], &[1, 2, 3], &[7, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]];
    for v in BOTH {
        for bytes in garbage {
            assert!(is_invalid_data(merge(v, &[bytes.to_vec()])), "{v:?} merge {bytes:?}");
            assert!(is_invalid_data(sv_from_update(v, bytes)), "{v:?} state vector of {bytes:?}");
            assert!(is_invalid_data(diff(v, bytes, &[0])), "{v:?} diff of {bytes:?}");
        }
        // One bad update spoils the merge.
        let (_, updates) = edited(v, 1);
        assert!(is_invalid_data(merge(v, &[updates[0].clone(), vec![0xff; 5]])), "{v:?}");
        // A good update, a bad state vector.
        assert!(is_invalid_data(diff(v, &updates[0], &[0xff, 0xff])), "{v:?}");
    }
}

#[test]
fn a_v1_update_is_no_v2_update() {
    // (The other way round isn't always caught: v2 bytes may happen to read as some v1 update.)
    let (_, v1) = edited(V::One, 1);
    assert!(is_invalid_data(merge_updates_v2(vec![v1[0].clone()])));
    assert!(is_invalid_data(encode_state_vector_from_update_v2(v1[0].clone())));
    assert!(is_invalid_data(diff_update_v2(v1[0].clone(), vec![0])));
}

#[test]
fn truncated_and_corrupted_updates_never_crash() {
    for v in BOTH {
        let (_, updates) = edited(v, 1);
        let full = merge(v, &updates).unwrap();
        let mut errors = 0;
        for len in 0..full.len() {
            let cut = full[..len].to_vec();
            // Ok or an error, whatever the cut: never a panic out of the call.
            errors += merge(v, &[cut.clone()]).is_err() as usize;
            let _ = sv_from_update(v, &cut);
            let _ = diff(v, &cut, &[0]);
        }
        assert!(errors > 0, "{v:?}: some cuts fail");
        for i in 0..full.len() {
            let mut flipped = full.clone();
            flipped[i] ^= 0xa5;
            let _ = merge(v, &[flipped.clone()]);
            let _ = sv_from_update(v, &flipped);
            let _ = diff(v, &flipped, &[0]);
        }
    }
}
