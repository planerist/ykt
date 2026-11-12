//! Awareness (y-protocols' awareness, yrs's `sync::Awareness`): short-lived state each client shares
//! with its peers — a cursor, a selection, a name and colour, "online" — outside the document's
//! history. Each client sets only its own state; states travel as awareness updates (y-protocols'
//! encoding, so they mix with Yjs clients'), each with a clock that orders a client's states.
//!
//! `YAwareness(doc)` keeps the states of every client it has heard of; the local client is the
//! document's client id. A peer that stops sending is dropped by `removeOutdated(timeout)` (Yjs's
//! 30 s check), which also renews the local state so peers keep it. Time comes from the system
//! clock, or from a `YAwarenessClock` given to the constructor (tests, or an app's own clock).
//!
//! The rules of the document's observers hold: callbacks run on the calling thread while the
//! awareness is in use, and inside them the awareness can't be called (`Locked`); one call at a
//! time, from any thread.

use crate::attrs::{into_yvalue, YValue};
use crate::doc::YDoc;
use crate::observers::{subscription_key, ObserveResult, YSubscription};
use crate::tools::{Error, Result};
use crate::updates::decoding;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use yrs::sync::awareness::{AwarenessUpdate, AwarenessUpdateEntry, Event};
use yrs::sync::{Awareness, Clock, Timestamp};
use yrs::updates::decoder::Decode;
use yrs::updates::encoder::Encode;
use yrs::{Any, ClientID, Origin};

/// The time, in milliseconds, for an awareness: when a state was last updated.
#[uniffi::export(with_foreign)]
pub trait YAwarenessClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

/// Clients whose state an update or a local change added, updated or removed, and its origin
/// (`applyUpdate(bytes, origin)`; none for local changes).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct YAwarenessEvent {
    pub added: Vec<u64>,
    pub updated: Vec<u64>,
    pub removed: Vec<u64>,
    pub origin: Option<Vec<u8>>,
}

#[uniffi::export(with_foreign)]
pub trait YAwarenessObserver: Send + Sync {
    fn on_change(&self, event: YAwarenessEvent);
}

/// What an awareness knows of a client besides its state: the clock of its last state, and when
/// that came (the awareness's clock, milliseconds).
#[derive(uniffi::Record, Clone, Debug, PartialEq)]
pub struct YAwarenessMeta {
    pub clock: u32,
    pub last_updated_millis: u64,
}

/// The awareness states of a document's clients (see the module docs).
#[derive(uniffi::Object)]
pub struct YAwareness {
    /// A lock, not a cell: a second call at once (another thread, or a callback calling it again)
    /// fails instead of racing.
    awareness: Mutex<Awareness>,
    /// Above 0 while one of its callbacks runs.
    in_callback: Arc<AtomicUsize>,
    clock: Arc<dyn Clock>,
}

// SAFETY: as YUndoManager: the awareness is behind a lock; yrs's own callbacks (not `Send` without
// the `sync` feature) only call foreign observers, which are `Send + Sync`.
unsafe impl Send for YAwareness {}
unsafe impl Sync for YAwareness {}

struct ForeignClock(Arc<dyn YAwarenessClock>);

impl Clock for ForeignClock {
    fn now(&self) -> Timestamp {
        self.0.now_millis()
    }
}

#[uniffi::export]
impl YAwareness {
    /// The awareness of `doc`'s clients; the local client is `doc`'s client id. `clock` gives the
    /// time (by default the system's).
    #[uniffi::constructor(default(clock = None))]
    pub fn new(doc: &YDoc, clock: Option<Arc<dyn YAwarenessClock>>) -> Self {
        let clock: Arc<dyn Clock> = match clock {
            Some(clock) => Arc::new(ForeignClock(clock)),
            None => Arc::new(yrs::sync::time::SystemClock),
        };
        let shared = clock.clone();
        YAwareness {
            awareness: Mutex::new(Awareness::with_clock(doc.0.clone(), move || shared.now())),
            in_callback: Arc::new(AtomicUsize::new(0)),
            clock,
        }
    }

    /// The local client's id (the document's).
    pub fn client_id(&self) -> u64 {
        self.with(|a| a.client_id().get()).unwrap_or_default()
    }

    /// Sets the local client's state (usually a map: cursor, name, colour) and bumps its clock:
    /// the change observers hear it, and `encodeUpdate()` carries it to peers.
    pub fn set_local_state(&self, state: YValue) -> Result<()> {
        let json = to_json(state);
        self.with(|a| a.set_local_state_raw(json))
    }

    /// As `setLocalState`, from a JSON text (`InvalidData` if it isn't JSON).
    pub fn set_local_state_json(&self, json: String) -> Result<()> {
        Any::from_json(&json).map_err(|e| Error::InvalidData(e.to_string()))?;
        self.with(|a| a.set_local_state_raw(json))
    }

    /// The local client's state; `null` before it is set or after `clearLocalState()`.
    pub fn local_state(&self) -> Result<Option<YValue>> {
        self.with(|a| a.local_state_raw().and_then(|json| from_json(&json)))
    }

    /// The local client's state as JSON.
    pub fn local_state_json(&self) -> Result<Option<String>> {
        self.with(|a| a.local_state_raw().map(|json| json.to_string()))
    }

    /// Clears the local client's state, telling peers it is gone (Yjs's `setLocalState(null)`):
    /// send `encodeUpdate()` after it, before going offline.
    pub fn clear_local_state(&self) -> Result<()> {
        self.with(|a| {
            if a.local_state_raw().is_some() {
                a.clean_local_state();
            }
        })
    }

    /// The states of every client that has one, the local client's included. A state that isn't
    /// valid JSON (a broken peer's) is left out; `statesJson()` gives it as it came.
    pub fn states(&self) -> Result<HashMap<u64, YValue>> {
        self.with(|a| {
            a.iter()
                .filter_map(|(client, state)| Some((client.get(), from_json(state.data.as_ref()?)?)))
                .collect()
        })
    }

    /// The states of every client that has one, as JSON.
    pub fn states_json(&self) -> Result<HashMap<u64, String>> {
        self.with(|a| a.iter().filter_map(|(client, state)| Some((client.get(), state.data?.to_string()))).collect())
    }

    /// The clock and the time of a client's last state; `null` for a client never heard of.
    pub fn meta(&self, client_id: u64) -> Result<Option<YAwarenessMeta>> {
        self.with(|a| {
            a.meta(ClientID::new(client_id)).map(|(clock, last_updated)| YAwarenessMeta { clock, last_updated_millis: last_updated })
        })
    }

    /// An awareness update with the states of `clients`, in y-protocols' encoding: what to send to
    /// peers (`applyUpdate` there). By default every client with a state, and the local client
    /// also once cleared (so `clearLocalState()` then `encodeUpdate()` tells peers it is gone); with
    /// `clients`, those (a cleared one as gone; one never heard of is left out) — e.g. the ids of an
    /// update observer's event, to forward what changed.
    #[uniffi::method(default(clients = None))]
    pub fn encode_update(&self, clients: Option<Vec<u64>>) -> Result<Vec<u8>> {
        self.with(|a| {
            let clients: Vec<ClientID> = match clients {
                None => {
                    let local = a.client_id();
                    a.iter().filter(|(client, state)| state.data.is_some() || *client == local).map(|(client, _)| client).collect()
                }
                Some(clients) => clients.into_iter().map(ClientID::new).filter(|c| a.meta(*c).is_some()).collect(),
            };
            a.update_with_clients(clients).map(|u| u.encode_v1()).map_err(|e| Error::InvalidData(e.to_string()))
        })?
    }

    /// Applies a peer's awareness update: newer states replace older ones, a state sent as gone
    /// removes the client. The change observers hear it with `origin`. Bytes that aren't an
    /// awareness update throw `InvalidData`.
    #[uniffi::method(default(origin = None))]
    pub fn apply_update(&self, update: Vec<u8>, origin: Option<Vec<u8>>) -> Result<()> {
        let update = decoding(|| AwarenessUpdate::decode_v1(&update))?;
        self.with(|a| match origin {
            Some(origin) => a.apply_update_with(update, Origin::from(origin.as_slice())),
            None => a.apply_update(update),
        })?
        .map_err(|e| Error::InvalidData(e.to_string()))
    }

    /// Removes the states of `clients` (Yjs's `removeAwarenessStates`), e.g. of a peer whose
    /// connection closed; the local client's too, if given (as `clearLocalState`).
    pub fn remove_states(&self, clients: Vec<u64>) -> Result<()> {
        self.with(|a| {
            for client in clients {
                remove(a, ClientID::new(client));
            }
        })
    }

    /// Removes the peers whose state is older than `timeout_millis` (Yjs's outdated check, 30 s
    /// there; call it every few seconds), and renews the local state once it is older than half of
    /// it, so that peers doing the same keep it. Returns the removed clients.
    pub fn remove_outdated(&self, timeout_millis: u64) -> Result<Vec<u64>> {
        let now = self.clock.now();
        self.with(|a| {
            let local = a.client_id();
            let mut outdated = vec![];
            let mut renew = None;
            for (client, state) in a.iter() {
                let Some(data) = state.data else { continue };
                let age = now.saturating_sub(state.last_updated);
                if client == local {
                    if age >= timeout_millis / 2 {
                        renew = Some(data);
                    }
                } else if age >= timeout_millis {
                    outdated.push(client);
                }
            }
            if let Some(data) = renew {
                a.set_local_state_raw(data);
            }
            for client in &outdated {
                remove(a, *client);
            }
            outdated.into_iter().map(|c| c.get()).collect()
        })
    }

    /// Calls `observer` when clients are added, removed, or their state changes (Yjs's `change`).
    pub fn observe_change(self: Arc<Self>, observer: Arc<dyn YAwarenessObserver>) -> ObserveResult {
        let (key, active, callback) = self.callback(observer);
        self.with(|a| a.on_change(key.clone(), callback))?;
        Ok(self.subscription(active, move |a| a.unobserve_change(key)))
    }

    /// Calls `observer` on every state that arrives or is set, unchanged ones too (Yjs's
    /// `update`): e.g. to forward the update to other peers.
    pub fn observe_update(self: Arc<Self>, observer: Arc<dyn YAwarenessObserver>) -> ObserveResult {
        let (key, active, callback) = self.callback(observer);
        self.with(|a| a.on_update(key.clone(), callback))?;
        Ok(self.subscription(active, move |a| a.unobserve_update(key)))
    }
}

type Callback = Box<dyn FnMut(&Awareness, &Event, Option<&Origin>) + 'static>;

impl YAwareness {
    /// Runs `f` on the awareness; `Locked` while it is busy (inside its callbacks, or another call).
    fn with<R>(&self, f: impl FnOnce(&mut Awareness) -> R) -> Result<R> {
        if self.in_callback.load(Ordering::SeqCst) > 0 {
            return Err(Error::Locked);
        }
        let mut guard = match self.awareness.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::Poisoned(e)) => e.into_inner(),
            Err(TryLockError::WouldBlock) => return Err(Error::Locked),
        };
        Ok(f(&mut guard))
    }

    fn callback(&self, observer: Arc<dyn YAwarenessObserver>) -> (Origin, Arc<AtomicBool>, Callback) {
        let key = subscription_key();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        let in_callback = self.in_callback.clone();
        let callback: Callback = Box::new(move |_awareness, event, origin| {
            if running.load(Ordering::SeqCst) {
                in_callback.fetch_add(1, Ordering::SeqCst);
                let ids = |ids: &[ClientID]| ids.iter().map(|c| c.get()).collect::<Vec<_>>();
                observer.on_change(YAwarenessEvent {
                    added: ids(event.added()),
                    // A change event's `updated` are the clients whose state changed; an update
                    // event's, every client whose state came, changed or not.
                    updated: ids(event.updated()),
                    removed: ids(event.removed()),
                    origin: origin.map(|o| o.as_ref().to_vec()),
                });
                in_callback.fetch_sub(1, Ordering::SeqCst);
            }
        });
        (key, active, callback)
    }

    /// A subscription to one of the awareness's observers. It doesn't keep the awareness alive;
    /// unsubscribing while it is busy only switches the callback off.
    fn subscription(self: &Arc<Self>, active: Arc<AtomicBool>, remove: impl FnOnce(&mut Awareness) -> bool + Send + 'static) -> Arc<YSubscription> {
        let awareness = Arc::downgrade(self);
        YSubscription::new(active, move || match awareness.upgrade() {
            Some(awareness) => awareness.with(remove).unwrap_or(false),
            None => false,
        })
    }
}

/// Removes `client`'s state as Yjs's `removeAwarenessStates` does: a peer's without touching its
/// clock (yrs's `remove_state` bumps it, and the peer's next state would then be ignored as old), the
/// local client's with its clock bumped, so peers drop it too. Nothing for a client without a state
/// (yrs would tell observers of a client it never had).
fn remove(a: &mut Awareness, client: ClientID) {
    let Some((clock, _)) = a.meta(client) else { return };
    if !a.iter().any(|(c, state)| c == client && state.data.is_some()) {
        return;
    }
    if client == a.client_id() {
        a.clean_local_state();
    } else {
        let gone = AwarenessUpdateEntry { clock, json: "null".into() };
        let _ = a.apply_update(AwarenessUpdate { clients: HashMap::from([(client, gone)]) });
    }
}

/// `value` as JSON, a map's keys sorted: the same state always gives the same text, so setting it
/// again isn't a change for observers (here or at a peer).
fn to_json(value: YValue) -> String {
    fn write(value: &YValue, out: &mut String) {
        match value {
            YValue::AttrMap(entries) => {
                let mut keys: Vec<&String> = entries.keys().collect();
                keys.sort();
                out.push('{');
                for (i, key) in keys.into_iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(&YValue::String(key.clone()), out);
                    out.push(':');
                    write(&entries[key], out);
                }
                out.push('}');
            }
            YValue::Array(values) => {
                out.push('[');
                for (i, value) in values.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(value, out);
                }
                out.push(']');
            }
            plain => {
                // yrs writes from the start of the buffer it is given: a fresh one each time.
                let any: Any = plain.clone().into();
                let mut json = String::new();
                any.to_json(&mut json);
                out.push_str(&json);
            }
        }
    }
    let mut json = String::new();
    write(&value, &mut json);
    json
}

fn from_json(json: &str) -> Option<YValue> {
    Any::from_json(json).ok().map(|any| into_yvalue(&any))
}
