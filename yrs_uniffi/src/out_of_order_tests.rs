//! Updates out of order: a document that got a client's update before that client's earlier ones (a
//! gap yrs keeps in its blocks) or before the updates it builds on (pending updates), encoded whole
//! or as a diff, in lib0 v1 and v2: the bytes read on a fresh replica, which converges once the
//! missing updates arrive. yrs 0.28's own v2 writer failed here (`updates::DocEncoderV2`).
//!
//! And what yrs 0.28 got wrong underneath, fixed in `vendor/yrs`: it integrated a client's update
//! over the gap its missing earlier ones leave, so the document depended on the order updates came
//! in, and some were lost for good. A client's updates now wait for its earlier ones, as in Yjs.

use crate::doc::{YDoc, YDocOptions};
use crate::observers::{YUpdateEvent, YUpdateObserver};
use crate::snapshots::{
    apply_update, apply_update_v2, decode_state_vector, encode_state_as_update, encode_state_as_update_v2,
    encode_state_from_snapshot_v1, encode_state_from_snapshot_v2, encode_state_vector, snapshot,
};
use crate::text::YText;
use std::sync::{Arc, Mutex};

fn doc(client_id: u64) -> YDoc {
    YDoc::new(Some(YDocOptions {
        client_id: Some(client_id),
        guid: None,
        collection_id: None,
        gc: Some(false),
        auto_load: None,
        should_load: None,
        bytes_offset: None,
    }))
}

fn text_of(doc: &YDoc) -> String {
    doc.get_text("t").unwrap().to_string(None).unwrap()
}

/// `doc`'s changes in a transaction of their own, as an update (lib0 v2).
fn edit(doc: &YDoc, change: impl FnOnce(&YText, Option<Arc<crate::transaction::YTransaction>>)) -> Vec<u8> {
    let text = doc.get_text("t").unwrap();
    let txn = Arc::new(doc.transaction(None).unwrap());
    change(&text, Some(txn.clone()));
    let update = txn.encode_update_v2().unwrap();
    txn.commit().unwrap();
    update
}

/// A replica that reads `update`: the encoding must be readable, else this panics with `what`.
fn replica_of(update: &[u8], v2: bool, what: &str) -> YDoc {
    let replica = doc(99);
    let applied = if v2 { apply_update_v2(&replica, update, None) } else { apply_update(&replica, update, None) };
    applied.unwrap_or_else(|e| panic!("{what}: {e}"));
    replica
}

#[derive(Default)]
struct Updates(Mutex<Vec<Vec<u8>>>);

impl YUpdateObserver for Updates {
    fn on_update(&self, event: YUpdateEvent) {
        self.0.lock().unwrap().push(event.update);
    }
}

#[test]
fn every_encoding_of_updates_out_of_order_reads_and_converges() {
    // Client 1: "abc", then "x" at the start, then "y" at the end (built on the first only).
    let source = doc(1);
    let first = edit(&source, |t, txn| t.insert(0, "abc", None, txn).unwrap());
    let second = edit(&source, |t, txn| t.insert(0, "x", None, txn).unwrap());
    let third = edit(&source, |t, txn| t.insert(4, "y", None, txn).unwrap());
    // Client 2 builds on the first: "B" after "a".
    let other = doc(2);
    apply_update_v2(&other, &first, None).unwrap();
    let from_other = edit(&other, |t, txn| t.insert(1, "B", None, txn).unwrap());
    // Client 3 builds on both: "C" after "B".
    let third_client = doc(3);
    apply_update_v2(&third_client, &first, None).unwrap();
    apply_update_v2(&third_client, &from_other, None).unwrap();
    let from_third = edit(&third_client, |t, txn| t.insert(2, "C", None, txn).unwrap());

    // The server gets them out of order — clients 2's and 3's before what they build on, client
    // 1's third before its second — and never the second. The third goes in the last transaction.
    let server = doc(4);
    let heard = Arc::new(Updates::default());
    let sub = server.observe_update_v2(heard.clone()).unwrap();
    apply_update_v2(&server, &from_third, None).unwrap();
    apply_update_v2(&server, &from_other, None).unwrap();
    apply_update_v2(&server, &first, None).unwrap();
    let txn = server.transaction(None).unwrap();
    txn.apply_v2(third.clone()).unwrap();
    let in_transaction = txn.encode_update_v2().unwrap();
    let diff = txn.diff_v2(encode_state_vector(&doc(98)).unwrap()).unwrap();
    txn.commit().unwrap();
    drop(sub);

    let nothing = || Some(Arc::new(decode_state_vector(Some(encode_state_vector(&doc(98)).unwrap())).unwrap()));
    let snap = snapshot(&server).unwrap();
    let whole = [
        ("encodeStateAsUpdateV2", encode_state_as_update_v2(&server, None).unwrap(), true),
        ("encodeStateAsUpdateV2 with a state vector", encode_state_as_update_v2(&server, nothing()).unwrap(), true),
        ("encodeStateAsUpdate", encode_state_as_update(&server, None).unwrap(), false),
        ("encodeStateAsUpdate with a state vector", encode_state_as_update(&server, nothing()).unwrap(), false),
    ];
    let mut more = vec![
        ("YTransaction.diffV2".to_string(), diff, true),
        ("YTransaction.encodeUpdateV2".to_string(), in_transaction, true),
        ("encodeStateFromSnapshotV2".to_string(), encode_state_from_snapshot_v2(&server, &snap).unwrap(), true),
        ("encodeStateFromSnapshotV1".to_string(), encode_state_from_snapshot_v1(&server, &snap).unwrap(), false),
    ];
    more.extend(heard.0.lock().unwrap().iter().enumerate().map(|(i, u)| (format!("observeUpdateV2 #{i}"), u.clone(), true)));
    for (what, bytes, v2) in &more {
        replica_of(bytes, *v2, what);
    }

    // The whole state, then the missing update: the replica has everything.
    for (what, bytes, v2) in whole {
        let replica = replica_of(&bytes, v2, what);
        apply_update_v2(&replica, &second, None).unwrap();
        assert_eq!(text_of(&replica), "xaBCbcy", "{what}");
    }
    apply_update_v2(&server, &second, None).unwrap();
    assert_eq!(text_of(&server), "xaBCbcy");
}

/// A small deterministic random source (xorshift).
struct Random(u64);

impl Random {
    fn next(&mut self, bound: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % bound as u64) as u32
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            items.swap(i, self.next(i as u32 + 1) as usize);
        }
    }
}

/// Updates (lib0 v2) of three clients editing one text, each getting some of the others' updates
/// before its next edit: later updates build on earlier ones of other clients.
pub(crate) fn interleaved_updates(seed: u64, edits: usize) -> Vec<Vec<u8>> {
    let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let clients: Vec<YDoc> = (0..3).map(|i| doc(20 + i)).collect();
    let mut made: Vec<(usize, Vec<u8>)> = vec![];
    for n in 0..edits {
        let i = random.next(3) as usize;
        let received: Vec<Vec<u8>> = made.iter().filter(|(from, _)| *from != i && random.next(3) != 0).map(|(_, u)| u.clone()).collect();
        if !received.is_empty() {
            let txn = clients[i].transaction(None).unwrap();
            for u in received {
                txn.apply_v2(u).unwrap();
            }
            txn.commit().unwrap();
        }
        let choice = random.next(5);
        let (a, b) = (random.next(1000), random.next(1000));
        let update = edit(&clients[i], |text, txn| {
            let len = text.length(txn.clone()).unwrap();
            match choice {
                0 if len > 2 => {
                    let at = a % (len - 1);
                    text.delete(at, 1 + b % (len - at).min(4), txn).unwrap()
                }
                1 | 2 if len > 0 => text.insert(a % (len + 1), &format!(" c{i}.{n}"), None, txn).unwrap(),
                _ => text.push(&format!("\nline c{i}.{n}"), None, txn).unwrap(),
            }
        });
        made.push((i, update));
    }
    made.into_iter().map(|(_, u)| u).collect()
}

#[test]
fn out_of_order_updates_of_several_clients_encode_readably() {
    for seed in 1..=30 {
        let mut updates = interleaved_updates(seed, 25);
        Random(seed + 7).shuffle(&mut updates);
        let server = doc(1);
        // A replica that took the first half of the updates, as they came, and asks for the rest.
        let behind = doc(2);
        for u in &updates[..updates.len() / 2] {
            apply_update_v2(&behind, u, None).unwrap();
        }
        let behind_vector = || Some(Arc::new(decode_state_vector(Some(encode_state_vector(&behind).unwrap())).unwrap()));
        let mut middle = None;

        for (step, update) in updates.iter().enumerate() {
            apply_update_v2(&server, update, None).unwrap();
            let at = format!("seed {seed} step {step}");
            let v2 = replica_of(&encode_state_as_update_v2(&server, None).unwrap(), true, &at);
            let v1 = replica_of(&encode_state_as_update(&server, None).unwrap(), false, &at);
            assert_eq!(text_of(&v2), text_of(&server), "{at}: the replica reads what the server has");
            assert_eq!(text_of(&v1), text_of(&server), "{at}: in v1 too");
            // As a diff for a replica behind: it reads, in both encodings.
            replica_of(&encode_state_as_update_v2(&server, behind_vector()).unwrap(), true, &at);
            replica_of(&encode_state_as_update(&server, behind_vector()).unwrap(), false, &at);
            if step == updates.len() / 2 {
                middle = Some(encode_state_as_update_v2(&server, None).unwrap());
            }
        }

        // A replica made from the state halfway, then every update: it has what the server has.
        let late = replica_of(&middle.unwrap(), true, "halfway");
        for u in &updates {
            apply_update_v2(&late, u, None).unwrap();
        }
        assert_eq!(text_of(&late), text_of(&server), "seed {seed}: converged");
    }
}

#[test]
fn every_order_of_the_same_updates_gives_the_same_document() {
    for seed in 1..=30 {
        let updates = interleaved_updates(seed, 25);
        let in_order = doc(1);
        for u in &updates {
            apply_update_v2(&in_order, u, None).unwrap();
        }
        let mut shuffled = updates.clone();
        Random(seed + 7).shuffle(&mut shuffled);
        let out_of_order = doc(2);
        for u in &shuffled {
            apply_update_v2(&out_of_order, u, None).unwrap();
        }
        assert_eq!(text_of(&out_of_order), text_of(&in_order), "seed {seed}");
        assert_eq!(encode_state_vector(&out_of_order).unwrap(), encode_state_vector(&in_order).unwrap(), "seed {seed}: nothing left waiting");
    }
}

#[test]
fn a_clients_update_waits_for_its_earlier_ones() {
    // Client 1 writes "ab". Client 2 writes "X" at the start, "Y" after it, then "Z" between "a"
    // and "b": the last builds on client 1's text only, not on client 2's earlier edits.
    let one = doc(1);
    let ab = edit(&one, |t, txn| t.insert(0, "ab", None, txn).unwrap());
    let two = doc(2);
    apply_update_v2(&two, &ab, None).unwrap();
    let x = edit(&two, |t, txn| t.insert(0, "X", None, txn).unwrap());
    let y = edit(&two, |t, txn| t.insert(1, "Y", None, txn).unwrap());
    let z = edit(&two, |t, txn| t.insert(3, "Z", None, txn).unwrap());
    assert_eq!(text_of(&two), "XYaZb");

    // Without "X", neither "Y" nor "Z" is in, whatever the order: they wait, as in Yjs. yrs 0.28
    // showed "Z" in the first order only.
    let mut texts = vec![];
    for order in [[&ab, &y, &z], [&z, &y, &ab]] {
        let server = doc(3);
        for u in order {
            apply_update_v2(&server, u, None).unwrap();
        }
        texts.push(text_of(&server));
        apply_update_v2(&server, &x, None).unwrap();
        assert_eq!(text_of(&server), "XYaZb");
    }
    assert_eq!(texts, ["ab", "ab"]);
}

#[test]
fn an_update_isnt_left_out_for_the_order_it_came_in() {
    // Seven updates of three clients (lib0 v1), from the randomized test, minimized: yrs 0.28.1
    // left out " c0.4" (#4) when they came in the second order, though everything it builds on had
    // arrived. #7 and #11 build on updates that never come: they wait in both orders.
    let hex = [
        (0, "01011400040101740a0a6c696e652063302e3000"),
        (1, "010115008414090a0a6c696e652063312e3100"),
        (2, "01011600040101740a0a6c696e652063322e3200"),
        (4, "0101140ac415041505052063302e3400"),
        (5, "0101160a8416090a0a6c696e652063322e3500"),
        (7, "010116148414180a0a6c696e652063322e3700"),
        (11, "0101150ac4161a161b062063312e313100"),
    ];
    let bytes = |i: usize| -> Vec<u8> {
        let h = hex.iter().find(|(n, _)| *n == i).unwrap().1;
        (0..h.len()).step_by(2).map(|k| u8::from_str_radix(&h[k..k + 2], 16).unwrap()).collect()
    };
    for order in [[0, 1, 2, 4, 5, 7, 11], [5, 4, 11, 2, 0, 7, 1]] {
        let server = doc(9);
        for i in order {
            apply_update(&server, &bytes(i), None).unwrap();
        }
        assert_eq!(text_of(&server), "\nline c0.0\nline c0.4 c1.1\nline c2.2\nline c2.5", "{order:?}");
    }
}
