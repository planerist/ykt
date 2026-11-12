//! Awareness through ykt's API: two peers exchanging updates (typed and JSON states), observers with
//! added / updated / removed clients and the origin, removal (cleared states, removeStates, the
//! outdated check on an injected clock, the local state renewed), what callbacks may not do, garbage.

use crate::attrs::{YAttributes, YValue};
use crate::awareness::*;
use crate::doc::{YDoc, YDocOptions};
use crate::observers::YSubscription;
use crate::tools::Error;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

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

/// A clock the test moves by hand.
#[derive(Default)]
struct TestClock(AtomicU64);

impl YAwarenessClock for TestClock {
    fn now_millis(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

impl TestClock {
    fn advance(&self, millis: u64) {
        self.0.fetch_add(millis, Ordering::SeqCst);
    }
}

fn peer(client_id: u64, clock: &Arc<TestClock>) -> Arc<YAwareness> {
    Arc::new(YAwareness::new(&doc(client_id), Some(clock.clone())))
}

fn user(name: &str) -> YValue {
    let state: YAttributes = HashMap::from([
        ("name".to_string(), YValue::String(name.into())),
        ("cursor".to_string(), YValue::AttrMap(HashMap::from([("index".to_string(), YValue::Number(3.0))]))),
    ]);
    YValue::AttrMap(state)
}

/// Sends what `from` has to `to`.
fn send(from: &YAwareness, to: &YAwareness) {
    to.apply_update(from.encode_update(None).unwrap(), Some(b"peer".to_vec())).unwrap();
}

#[derive(Default)]
struct Events(Mutex<Vec<YAwarenessEvent>>);

impl YAwarenessObserver for Events {
    fn on_change(&self, event: YAwarenessEvent) {
        self.0.lock().unwrap().push(event);
    }
}

impl Events {
    fn take(&self) -> Vec<(Vec<u64>, Vec<u64>, Vec<u64>, Option<Vec<u8>>)> {
        std::mem::take(&mut *self.0.lock().unwrap()).into_iter().map(|e| (e.added, e.updated, e.removed, e.origin)).collect()
    }
}

#[test]
fn two_peers_exchange_states() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let b = peer(2, &clock);
    assert_eq!(a.client_id(), 1);
    assert_eq!(a.local_state().unwrap(), None);

    a.set_local_state(user("Ann")).unwrap();
    b.set_local_state_json(r#"{"name":"Bob","cursor":null}"#.into()).unwrap();
    send(&a, &b);
    send(&b, &a);

    assert_eq!(a.local_state().unwrap(), Some(user("Ann")));
    let bob = YValue::AttrMap(HashMap::from([("name".to_string(), YValue::String("Bob".into())), ("cursor".to_string(), YValue::Null)]));
    for peer in [&a, &b] {
        assert_eq!(peer.states().unwrap(), HashMap::from([(1, user("Ann")), (2, bob.clone())]));
    }
    assert_eq!(b.states_json().unwrap()[&2], r#"{"name":"Bob","cursor":null}"#);
    assert_eq!(b.meta(1).unwrap().map(|m| m.clock), Some(1));
    assert_eq!(b.meta(9).unwrap(), None);

    // An update of some clients only; unknown ones are left out.
    a.set_local_state(user("Ann K.")).unwrap();
    b.apply_update(a.encode_update(Some(vec![1, 77])).unwrap(), None).unwrap();
    assert_eq!(b.states().unwrap()[&1], user("Ann K."));
    assert_eq!(b.meta(1).unwrap().map(|m| m.clock), Some(2));
}

#[test]
fn an_old_update_doesnt_override_a_newer_state() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let b = peer(2, &clock);
    a.set_local_state(user("first")).unwrap();
    let old = a.encode_update(None).unwrap();
    a.set_local_state(user("second")).unwrap();
    send(&a, &b);
    b.apply_update(old, None).unwrap();
    assert_eq!(b.states().unwrap()[&1], user("second"));
}

#[test]
fn observers_hear_added_updated_and_removed_clients_with_the_origin() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let b = peer(2, &clock);
    let changes = Arc::new(Events::default());
    let updates = Arc::new(Events::default());
    let _changes_sub = b.clone().observe_change(changes.clone()).unwrap();
    let _updates_sub = b.clone().observe_update(updates.clone()).unwrap();

    a.set_local_state(user("Ann")).unwrap();
    send(&a, &b); // added
    a.set_local_state(user("Ann")).unwrap();
    send(&a, &b); // the same state again: an update, no change
    a.set_local_state(user("Ann K.")).unwrap();
    send(&a, &b); // changed
    a.clear_local_state().unwrap();
    send(&a, &b); // removed
    b.set_local_state(user("Bob")).unwrap(); // local: no origin

    let peer = Some(b"peer".to_vec());
    assert_eq!(
        changes.take(),
        vec![
            (vec![1], vec![], vec![], peer.clone()),
            (vec![], vec![1], vec![], peer.clone()),
            (vec![], vec![], vec![1], peer.clone()),
            (vec![2], vec![], vec![], None),
        ]
    );
    assert_eq!(
        updates.take(),
        vec![
            (vec![1], vec![], vec![], peer.clone()),
            (vec![], vec![1], vec![], peer.clone()),
            (vec![], vec![1], vec![], peer.clone()),
            (vec![], vec![], vec![1], peer.clone()),
            (vec![2], vec![], vec![], None),
        ]
    );
    assert!(!b.states().unwrap().contains_key(&1));
}

#[test]
fn remove_states_drops_a_peer_and_tells_observers() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let b = peer(2, &clock);
    a.set_local_state(user("Ann")).unwrap();
    send(&a, &b);
    let changes = Arc::new(Events::default());
    let _sub = b.clone().observe_change(changes.clone()).unwrap();

    b.remove_states(vec![1, 42]).unwrap(); // 42: never heard of, nothing to tell

    assert_eq!(changes.take(), vec![(vec![], vec![], vec![1], None)]);
    assert!(b.states().unwrap().is_empty());
    // The peer's next state brings it back.
    a.set_local_state(user("Ann again")).unwrap();
    send(&a, &b);
    assert_eq!(b.states().unwrap()[&1], user("Ann again"));
}

#[test]
fn outdated_peers_are_removed_and_the_local_state_renewed() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let b = peer(2, &clock);
    let c = peer(3, &clock);
    a.set_local_state(user("Ann")).unwrap();
    b.set_local_state(user("Bob")).unwrap();
    c.set_local_state(user("Cid")).unwrap();
    send(&a, &b);
    send(&c, &b);
    let changes = Arc::new(Events::default());
    let _sub = b.clone().observe_change(changes.clone()).unwrap();

    clock.advance(20_000);
    c.set_local_state(user("Cid")).unwrap(); // c keeps sending (its state renewed), a doesn't
    send(&c, &b);
    assert_eq!(b.remove_outdated(30_000).unwrap(), Vec::<u64>::new());
    let bob_clock = b.meta(2).unwrap().unwrap().clock;
    assert!(bob_clock > 1, "older than half the timeout: the local state is renewed");
    clock.advance(15_000);
    assert_eq!(b.remove_outdated(30_000).unwrap(), vec![1]);

    assert_eq!(b.states().unwrap().keys().copied().collect::<std::collections::BTreeSet<_>>(), [2, 3].into());
    assert_eq!(changes.take().last().unwrap().2, vec![1]);
    assert_eq!(b.local_state().unwrap(), Some(user("Bob")), "never removes the local state");
}

/// Calls the awareness inside its own callback, and unsubscribes there.
struct Misbehaving {
    awareness: Mutex<Option<Arc<YAwareness>>>,
    sub: Mutex<Option<Arc<YSubscription>>>,
    results: Mutex<Vec<bool>>,
}

impl YAwarenessObserver for Misbehaving {
    fn on_change(&self, _event: YAwarenessEvent) {
        let awareness = self.awareness.lock().unwrap().clone().unwrap();
        let locked = matches!(awareness.states(), Err(Error::Locked)) && matches!(awareness.set_local_state(YValue::Null), Err(Error::Locked));
        self.results.lock().unwrap().push(locked);
        if let Some(sub) = self.sub.lock().unwrap().take() {
            assert!(sub.unsubscribe());
        }
    }
}

#[test]
fn inside_a_callback_the_awareness_is_locked_and_unsubscribing_works() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    let observer = Arc::new(Misbehaving { awareness: Mutex::new(Some(a.clone())), sub: Mutex::new(None), results: Mutex::new(vec![]) });
    let sub = a.clone().observe_change(observer.clone()).unwrap();
    *observer.sub.lock().unwrap() = Some(sub);

    a.set_local_state(user("Ann")).unwrap();
    a.set_local_state(user("Ann K.")).unwrap();

    assert_eq!(*observer.results.lock().unwrap(), vec![true]);
    assert_eq!(a.local_state().unwrap(), Some(user("Ann K.")));
    *observer.awareness.lock().unwrap() = None; // the observer held the awareness: let it go
}

#[test]
fn garbage_is_invalid_data_and_changes_nothing() {
    let clock = Arc::new(TestClock::default());
    let a = peer(1, &clock);
    a.set_local_state(user("Ann")).unwrap();
    let good = a.encode_update(None).unwrap();
    let b = peer(2, &clock);
    for bytes in [vec![], vec![0xff; 12], good[..good.len() - 1].to_vec(), vec![5, 1]] {
        assert!(matches!(b.apply_update(bytes.clone(), None), Err(Error::InvalidData(_))), "{bytes:?}");
    }
    assert!(matches!(a.set_local_state_json("{not json".into()), Err(Error::InvalidData(_))));
    assert!(b.states().unwrap().is_empty());
    assert_eq!(a.local_state().unwrap(), Some(user("Ann")));
    // A state that isn't JSON (a broken peer's) is left out of states(), kept in statesJson().
    let bad = {
        use yrs::updates::encoder::Encode;
        let mut clients = HashMap::new();
        clients.insert(yrs::ClientID::new(9), yrs::sync::awareness::AwarenessUpdateEntry { clock: 1, json: "{oops".into() });
        yrs::sync::awareness::AwarenessUpdate { clients }.encode_v1()
    };
    b.apply_update(bad, None).unwrap();
    assert!(b.states().unwrap().is_empty());
    assert_eq!(b.states_json().unwrap()[&9], "{oops");
}

#[test]
fn the_system_clock_by_default() {
    let a = YAwareness::new(&doc(1), None);
    a.set_local_state(YValue::Bool(true)).unwrap();
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis() as u64;
    let updated = a.meta(1).unwrap().unwrap().last_updated_millis;
    assert!(now.abs_diff(updated) < 60_000);
    assert_eq!(a.remove_outdated(30_000).unwrap(), Vec::<u64>::new());
}
