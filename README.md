# Ykt

Kotlin bindings for the [Y‑CRDT](https://github.com/y-crdt/) ecosystem. Ykt brings collaborative, conflict‑free data types to Kotlin applications by wrapping the high‑performance Rust implementation (Yrs) via UniFFI. Use it to build realtime text editors, notes, whiteboards, and any app that needs local‑first collaboration and seamless sync across devices and platforms.

## Why Ykt?
- Built on Yrs (Rust): safety, performance, and a mature CRDT core
- Kotlin‑first ergonomics with a small, focused API surface
- Interoperates with other Y‑CRDT bindings (JS, Rust, Swift, etc.) for cross‑platform sync

## Status
This project is actively developed and used in real projects. The foundation is stable thanks to Yrs. The primary limitation at this stage is an incomplete Kotlin API surface (not stability or performance).

Currently solid: YText (deltas, formatting with JSON or typed attributes, embeds, transactions, observers), YMap and YArray with nested shared types (+ observers), deep and XML observers, document update, transaction and subdocument events, update utilities without a document, relative positions, undo / redo, awareness, basic XML types, core document/transaction flow, and one error type.

## Quick start

Ykt uses Mozilla UniFFI to bridge Rust to Kotlin and Swift.

Project layout:
- [yrs_uniffi](yrs_uniffi) — UniFFI bindings for Yrs (Rust)
- [yrs_kt](yrs_kt) — Kotlin wrapper and basic tests
- [yrs_swift](yrs_swift) — Swift package (SwiftPM) and basic tests

### Build artifacts
Prebuilt JARs and the Swift xcframework are attached to [GitHub Releases](https://github.com/planerist/ykt/releases);
the Swift package is the repository itself at a release tag.

Supported targets today:
- Kotlin/JVM: macOS (Darwin) aarch64 & x86_64, Linux x86_64
- Swift: macOS (aarch64 & x86_64), iOS, iOS Simulator, Mac Catalyst (aarch64 & x86_64)

Adding more targets is straightforward; open an issue or PR if you need one.

### Install
Option A — Download the JAR from GitHub Releases and include it in your classpath.

Option B — Build locally:
```
./gradlew build
```

### Swift
Add the package by its URL and a version (from 0.5.0), then `import Ykt`:
```swift
dependencies: [
    .package(url: "https://github.com/planerist/ykt", from: "0.5.0"),
],
targets: [
    .target(name: "App", dependencies: [.product(name: "Ykt", package: "ykt")]),
]
```
(In Xcode: File → Add Package Dependencies…, the URL, "Up to Next Major".) SwiftPM downloads the
release's `YktFFI.xcframework.zip` (macOS, iOS, iOS Simulator, Mac Catalyst) and checks it against the
checksum in the tag's `Package.swift`. `ykt-swift-<version>.zip` (the `Ykt` folder, to add as a local
package) stays in each release too.

To build locally (`--all` adds iOS, Mac Catalyst and both macOS architectures, requires the rustup
targets) and use `yrs_swift/` as a path-based package:
```
./build_swift_bindings.sh
cd yrs_swift && swift test
```

### Releasing
From main, with the version in `yrs_uniffi/Cargo.toml`: Actions → Main → Run workflow with that
version, or `gh workflow run main.yml --ref main -f version=x.y.z`. The workflow builds and tests
everything, then on top of the built commit commits the root `Package.swift`
(`swift_release_manifest.sh`: its binary target is the release's `YktFFI.xcframework.zip`, with its
SHA-256 checksum) and the generated `yrs_swift/Sources/Ykt/Ykt.swift`, tags that commit `vx.y.z`, pushes
only the tag (main doesn't get the commit), and creates the GitHub release with the JARs, the
xcframework zip and `ykt-swift-x.y.z.zip`. Pushing a `v*` tag by hand no longer releases.

## Usage example (Kotlin/JVM)
```kotlin
fun main() {
    // Create a document and a YText shared type
    val doc = YDoc()
    val text = doc.getText("content")

    // It's fine to mutate without an explicit transaction for YText
    text.insert(0u, "hello")
    text.insert(5u, " world", "{\"bold\":true}") // simple attrs as JSON

    // Read the current textual value
    println(text.toText()) // -> "hello world" (attributes don't affect plain text)

    // You can also inspect structural changes as a Y delta
    val delta = text.toDelta()
    println("Delta: $delta")

    // Apply a delta (e.g., from a remote peer)
    text.applyDelta(
        listOf(
            YDelta.YRetain(11u, null),
            YDelta.YInsert(YValue.String("!"), null)
        )
    )
    println(text.toText()) // -> "hello world!"

    // Using an explicit transaction is also OK (and is required for XML APIs); see YXmlTest
    doc.transact { txn ->
        // e.g., xml operations that require txn
    }
}
```

Root types: `YDoc.getText(name)`, `getXmlFragment(name)`, `getMap(name)`, `getArray(name)` return the
document's root type of that name, creating it the first time (Yjs's `doc.getText` and so on); the same
name gives the same type, through any handle. They throw `Locked` while the document is locked — in
an open transaction or an observer's callback — instead of waiting for a lock that can't come there.
Since 0.5.0 `getText` and `getXmlFragment` throw too (they used to wait, for good, in those places):
Swift callers need `try`.

Notes:
- API names may evolve while the Kotlin surface stabilizes.
- Transactions are recommended for consistent reads/snapshots and are required for XML APIs; YText supports direct ops without an explicit transaction.

## Formatting and embeds
Attributes can be given typed, as `YAttributes` (a map of `YValue`), instead of a JSON string:

- `YText.insertWithAttributes(index, chunk, attributes, txn?)` — the inserted text gets exactly these
  attributes (Yjs's `insert(index, text, attributes)`): one the text around it has and the map lacks,
  or has as `null`, isn't taken over, so an empty map inserts plain text even inside bold. `insert`
  without attributes takes the formatting around it;
- `YText.formatWithAttributes(index, length, attributes, txn?)` — sets each attribute on the range; a
  `null` value removes it (Yjs's `format`);
- `YText.insertEmbed(index, embed, attributes?, txn?)` — any plain value as an embed (a mention, an
  image, a date: usually a map): one unit of the text's length, one insert in deltas and events;
- `YText.id()` — the text's identity in its document, the same on every replica: `YBranchId.Root(name)`,
  or `Nested(client, clock)` for a text inside another type.

The JSON calls (`insert(index, chunk, json)`, `format(index, length, json)`) stay as they were.

```kotlin
text.insertWithAttributes(0u, "Yaranga", mapOf("bold" to YValue.Bool(true), "link" to YValue.String(url)))
text.formatWithAttributes(0u, 7u, mapOf("bold" to YValue.Null))           // not bold any more
text.insertEmbed(7u, YValue.AttrMap(mapOf("mention" to YValue.String("note://42"))))
```

```swift
try text.insertWithAttributes(index: 0, chunk: "Yaranga", attributes: ["bold": .bool(true)])
try text.formatWithAttributes(index: 0, length: 7, attributes: ["bold": .null])
try text.insertEmbed(index: 7, embed: .attrMap(["date": .string("2026-11-12")]))
```

## Observers
Callbacks from Rust into Kotlin or Swift when a document or a text changes, as Yjs's `observe` and
`on('update')`:

- `YText.observe` — the text's change as a delta (retain / insert / delete, UTF‑16 units, with
  formatting attributes) and the transaction's origin, once per committed transaction;
- `YDoc.observeUpdateV1` / `observeUpdateV2` — every committed transaction as one update, ready to
  store or send (`applyUpdate` on another replica);
- `YDoc.observeTransactionCleanup` — the state vectors before and after every transaction, and what
  it deleted; `observeAfterTransaction` — the same, after the types' observers and before the cleanup
  (Yjs's `afterTransaction`);
- `observeDeep` on `YText`, `YMap`, `YArray`, `YXmlFragment`, `YXmlElement`, `YXmlText` (Yjs's
  `observeDeep`) — a `YDeepEvent(events, origin)` per transaction that changed the type or any type
  in it: one `YEvent` per changed type, shallower ones first — `TextChange(target, path, delta)`,
  `MapChange(target, path, keys)`, `ArrayChange(target, path, delta)`,
  `XmlChange(target, path, children, attributes)`, `XmlTextChange(target, path, delta, attributes)`.
  `target` is the changed type's handle, `path` the way to it from the observed type
  (`YPathSegment.Key(key)` / `Index(index)`, empty for the type itself);
- `observe` on the XML types — a `YXmlEvent(target, children, delta, attributes, origin)`: an element's
  or a fragment's children (inserted nodes as handles) and attributes (old and new values), an XML
  text's delta and attributes;
- `YDoc.observeSubdocs` — `YSubdocsEvent(added, removed, loaded, origin)`, subdocuments (`setDoc`,
  `load()`, `destroyDoc()`, deleting one); `observeDestroy` — `YDestroyEvent(guid)` when the document
  is destroyed (`YDoc.destroyDoc()`, Yjs's `destroy`: in Kotlin `destroy()` frees a handle).
  Registering a document observer is a transaction of its own, which after-transaction and cleanup
  observers hear (an empty one).

Each returns a `YSubscription`: `unsubscribe()` stops it, and so does releasing it (Kotlin `close()`
/ `use { }`, Swift ARC). It keeps the document's store alive, so it may outlive the document.

```kotlin
val sub = text.observe { event ->
    if (event.originString != "editor") applyToTheView(event.delta)   // skip our own writes
}
val updates = doc.observeUpdateV2 { event -> send(event.update) }
doc.transact("editor") { txn -> text.insert(0u, "hi", null, txn) }  // one event, origin "editor"
sub.close(); updates.close()
```

```swift
let sub = try text.observe { event in
    if event.originString != "editor" { applyToTheView(event.delta) }
}
let updates = try doc.observeUpdateV2 { event in send(event.update) }
let deep = try note.observeDeep { event in
    for change in event.events { if case .textChange(let text, let path, let delta) = change { render(path, text, delta) } }
}
```

Rules:
- Callbacks run synchronously on the thread that commits the transaction, while the document is
  still locked: they must not read or write the document (that throws `Locked`); everything they
  need is in the event. Defer anything else.
- Unsubscribing is safe anywhere, inside a callback too: the callback is switched off at once.
- Subscribing to document events while a transaction is open throws.
- Threads: ykt documents expect one call at a time. Share a document between threads behind a lock
  (callbacks then run on whichever thread commits), or give each replica its own thread and sync
  replicas through their update events (see the threading tests in `yrs_uniffi/src/observer_tests.rs`,
  `yrs_swift/Tests/YktTests/ObserverTests.swift`, `yrs_kt/src/test/kotlin/ObserverTest.kt`).

## Update utilities
Functions on encoded updates alone, no document needed (Yjs's `mergeUpdates`, `diffUpdate`,
`encodeStateVectorFromUpdate`): a server or an outbox can compact stored updates, or answer a
replica's sync request, without loading the document.

- `mergeUpdatesV1` / `mergeUpdatesV2(updates)` — one update that is all of them: applying it is the
  same as applying each in turn, in any order (an empty list gives an empty update);
- `diffUpdateV1` / `diffUpdateV2(update, stateVector)` — what of `update` a replica with that state
  vector lacks (deletions always included);
- `encodeStateVectorFromUpdateV1` / `V2(update)` — the state vector of a document that has exactly
  that update.

State vectors are always lib0 v1 here, whatever the update's encoding (as in Yjs, where both
encodings of a state vector are the same bytes): what `encodeStateVector(doc)` and
`YTransaction.stateVectorV1()` return. Bytes that aren't a valid update or state vector throw
`InvalidData`; they never crash the process.

```kotlin
val stored: List<ByteArray> = loadUpdates(noteId)          // v2 updates, as observeUpdateV2 sent them
val compacted = mergeUpdatesV2(stored)                       // store this one instead
val reply = diffUpdateV2(compacted, clientStateVector)       // what the client lacks
```

```swift
let compacted = try mergeUpdatesV2(updates: stored)
let reply = try diffUpdateV2(update: compacted, stateVector: clientStateVector)
let state = try encodeStateVectorFromUpdateV2(update: compacted)
```

## Relative positions
A place in a text that keeps its spot through local and remote edits — a caret, the ends of a
selection, a comment's range (yrs's `StickyIndex`, Yjs's `RelativePosition`):

- `YText.stickyIndex(index, assoc, txn?)` — the position at `index` (UTF-16 units), as Yjs's
  `createRelativePositionFromTypeIndex`; `OutOfBounds` past the end;
- `YStickyIndex.offset(doc, txn?)` — where it is now, as `YStickyOffset(index, assoc)` (Yjs's
  `createAbsolutePositionFromRelativePosition`); `null` while the document lacks what it refers to
  (made on a replica whose changes haven't arrived). A position whose neighbour was deleted
  collapses to where the deleted part was;
- `encode()` / `YStickyIndex.decode(bytes)` — Yjs's encoding (`encodeRelativePosition`), so
  positions travel between ykt and Yjs clients; garbage throws `InvalidData`. Positions compare
  equal by value, and print as `YStickyIndex(after 1#5)`.

`YAssoc` says which neighbour it sticks to, so where an insert exactly at the position goes:
`After` (Yjs's default, `assoc >= 0`) sticks to the character after it, so the insert goes before
the position and the position moves on; `Before` sticks to the character before it, so the position
stays. At the very start `Before` and at the very end `After` stay at the start / the end.

```kotlin
val anchor = text.stickyIndex(selection.start, YAssoc.AFTER)
val head = text.stickyIndex(selection.end, YAssoc.BEFORE)
send(anchor.encode())                                  // or keep it across remote updates
val start = YStickyIndex.decode(bytes).offset(doc)?.index
```

```swift
let caret = try text.stickyIndex(index: 5, assoc: .after)
try applyUpdateV2(doc: doc, update: remote)
let index = try caret.offset(doc: doc)?.index        // moved with the text around it
```

## Undo / redo
`YUndoManager(doc, scope, options)` — yrs's `UndoManager`, Yjs's `Y.UndoManager` — records the
changes of the transactions it tracks, by origin, to the texts in its scope (`expandScope(text)` adds
more), as stack items:

- `undo()` / `redo()` revert the last item / bring it back, and return whether anything changed;
  `canUndo()`, `canRedo()`, `clear()`;
- `YUndoOptions(captureTimeoutMillis = 500, trackedOrigins = [])`: changes closer than the timeout
  merge into one item (`stopCapturing()` ends the item at once); with tracked origins, only the
  transactions of those origins are recorded — remote updates applied with another origin, or none,
  are never undone, and an undo keeps them. Without, transactions without an origin are recorded.
  `addTrackedOrigin` / `removeTrackedOrigin` change the set;
- `observeItemAdded` / `observeItemUpdated` (an item extended within the timeout) /
  `observeItemPopped` — `YUndoEvent(kind, origin, meta)`, `kind` `UNDO` / `REDO` for the stack. An
  added or updated observer returns the meta map to keep with the item (or null), a popped one gets
  it back: e.g. the selection to restore, as Yjs's `stackItem.meta`;
- `origin()` — the origin of the manager's own undo / redo transactions, as text observers see it.

```kotlin
val undo = YUndoManager(doc, text, YUndoOptions(trackedOrigins = listOf("editor")))
undo.observeItemAdded { mapOf("selection" to YValue.Number(caret.toDouble())) }
undo.observeItemPopped { event -> restoreCaret = event.meta["selection"] }   // apply after undo() returns
doc.transact("editor") { txn -> text.insert(0u, "typed", null, txn) }
undo.undo()
```

```swift
let undo = try YUndoManager(doc: doc, scope: text, options: YUndoOptions(trackedOrigins: ["editor"]))
let added = try undo.observeItemAdded { _ in ["selection": .number(Double(caret))] }
let popped = try undo.observeItemPopped { event in pending = event.meta["selection"] }
_ = try undo.undo()
```

Rules: the manager's callbacks run with the document locked, and inside them neither the document
nor the manager may be used (that throws `Locked`). `undo()`, `redo()` and `clear()` throw inside
a transaction or any callback instead of waiting for the lock. Release the manager on the thread
that uses the document; one released while the document is locked can't detach from it and stays
in memory with it (harmlessly).

## YMap and YArray
`YDoc.getMap(name)` / `YDoc.getArray(name)` — Yjs's `Y.Map` and `Y.Array`, of plain values (`YValue`:
null, bool, number, bigint, string, buffer, and arrays and maps of them) and of shared types nested in
them. Every call takes an optional transaction, as `YText`'s.

- `YMap`: `set(key, value)`, `get(key)` (null if absent), `delete(key)` (returns the old value),
  `containsKey`, `keys()` and `values()` (in no particular order, the same for both), `len()`,
  `clear()`, `toJson()` (the whole map), `id()`;
- `YArray`: `insert(index, values)`, `push(values)`, `delete(index, length)`, `get(index)`, `len()`,
  `toJson()` (the whole list), `id()`; indexes past the end throw `OutOfBounds`;
- `observe` — `YMapEvent(keys, origin)` with `YEntryChange` per key (`Inserted(value)`,
  `Updated(oldValue, newValue)`, `Removed(oldValue)`), `YArrayEvent(delta, origin)` with
  `YArrayDelta` steps (`Insert(values)`, `Delete(n)`, `Retain(n)`), once per committed transaction;
  the same rules as the other observers. Values in events are `YOutput` (below); `observeDeep`
  hears the types nested in them too (see Observers).

`getMap` / `getArray` throw while the document is locked instead of waiting for it. Numbers travel in
lib0's encoding, as Yjs's: an integer outside JS's safe range that a double holds exactly (2^60)
reaches other replicas as a number, not a bigint.

```kotlin
val settings = doc.getMap("settings")
settings.set("theme", YValue.String("dark"))
settings.observe { event -> event.keys["theme"]?.let { applyTheme(it) } }.use { /* ... */ }
val tags = doc.getArray("tags")
tags.push(listOf(YValue.String("work"), YValue.String("home")))
```

```swift
let settings = try doc.getMap(name: "settings")
try settings.set(key: "theme", value: .string("dark"))
let sub = try doc.getArray(name: "tags").observe { event in render(event.delta) }
```

### Nested shared types
A text, a map or an array inside a map or an array (Yjs's `map.set('k', new Y.Map())`), each with its
own changes, history and observers:

- making one gives its handle: `YMap.setText(key, text?)`, `setMap(key, entries?)`,
  `setArray(key, values?)`, `setDoc(key, doc)` (a subdocument); `YArray.insertText(index, text?)`,
  `insertMap`, `insertArray`, `insertDoc`; `YText.insertEmbedText / Map / Array(index, …, attributes?)`
  (a shared type embedded in a text);
- reading one: `YMap.getOutput(key)` / `YArray.getOutput(index)` give a `YOutput` — `Value(value)`
  for a plain value, or a handle: `Text`, `Map`, `Array`, `XmlElement`, `XmlFragment`, `XmlText`,
  `Doc` (helpers `asValue`, `asText`, `asMap`, `asArray`, `asDoc` in Swift and Kotlin). The plain
  calls (`get`, `values`, `toJson`, text deltas) give a nested type in its JSON form, as before;
- handles of the same type of the same document compare equal (`==`, hashable); `id()` is a type's
  identity on every replica (`YBranchId.Nested(client, clock)`); a nested type its parent deleted or
  replaced throws `TypeDeleted`;
- undo: `YUndoManager.forMap(doc, map, options)` / `forArray(doc, array, options)`, and
  `expandScopeMap` / `expandScopeArray`: the scope covers the types nested in them too.

```kotlin
val note = doc.getMap("note")
val title = note.setText("title", "Untitled")              // a YText inside the map
title.insert(8u, " 2")
val meta = note.getOutput("meta")?.asMap                  // a nested map another client made
```

```swift
let meta = try note.setMap(key: "meta", entries: ["pinned": .bool(true)])
if case .text(let title)? = try note.getOutput(key: "title") { try title.insert(index: 0, chunk: "> ") }
let undo = try YUndoManager.forMap(doc: doc, scope: note, options: YUndoOptions(trackedOrigins: ["editor"]))
```

## Awareness
`YAwareness(doc, clock?)` — y-protocols' awareness (yrs's `sync::Awareness`): short-lived state each
client shares with its peers (a cursor, a selection, a name and colour) outside the document's history.
States travel as awareness updates in y-protocols' encoding, so they mix with Yjs clients'.

- `setLocalState(state: YValue)` / `setLocalStateJson(json)`, `localState()` / `localStateJson()`,
  `clearLocalState()` (Yjs's `setLocalState(null)`: the client is gone); the local client is the
  document's client id (`clientId()`);
- `states()` — every client with a state (`statesJson()` as JSON; a state that isn't JSON, from a broken
  peer, is left out of `states()`), `meta(clientId)` — its clock and when it last came;
- `encodeUpdate(clients?)` — what to send to peers: by default every state, and the local client once
  cleared (so clear, then send, before going offline); with ids, those (e.g. an event's);
  `applyUpdate(bytes, origin?)` — a peer's update (garbage throws `InvalidData`);
- `removeStates(clients)` — e.g. of a peer whose connection closed (Yjs's `removeAwarenessStates`);
  `removeOutdated(timeoutMillis)` — Yjs's outdated check (30 s there; call it every few seconds):
  removes peers not heard of for that long, renews the local state once older than half of it, so
  peers doing the same keep it; returns the removed ids;
- `observeChange` (clients added, removed, or whose state changed: Yjs's `change`) and `observeUpdate`
  (every state that comes or is set, unchanged ones too: Yjs's `update`) — `YAwarenessEvent(added,
  updated, removed, origin)`.

Time is the system clock, or a `YAwarenessClock` (`nowMillis()`) given to the constructor. A typed state
is sent as JSON with its keys sorted, so the same state set again is no change for anyone. The
observers' rules hold: callbacks run while the awareness is in use, and calling it there throws
`Locked`. yrs's removal bumps a peer's clock (its next state would be ignored): ykt removes peers as
Yjs does, keeping their clock.

```kotlin
val awareness = YAwareness(doc)
awareness.setLocalState(YValue.AttrMap(mapOf("name" to YValue.String("Ann"), "cursor" to YValue.Number(3.0))))
awareness.observeUpdate { event -> send(awareness.encodeUpdate(event.added + event.updated + event.removed)) }
onMessage { bytes -> awareness.applyUpdate(bytes, "remote".toByteArray()) }
timer(3_000) { awareness.removeOutdated(30_000uL) }
```

```swift
let awareness = YAwareness(doc: doc)
try awareness.setLocalState(state: .attrMap(["name": .string("Ann")]))
let sub = try awareness.observeChange { event in redrawCursors(event.added + event.updated, gone: event.removed) }
try awareness.applyUpdate(update: bytes, origin: Data("remote".utf8))
```

## Errors
One error type: Swift `YError`, Kotlin `YException` (a sealed class). Every case carries a message
(Swift `case Locked(message: String)`, Kotlin `e.message`) that says what happened; match on the case,
not the text (`catch YError.OutOfBounds`, `catch (e: YException.OutOfBounds)`).

| Case | When |
|---|---|
| `Locked` | The document is locked on this thread: a transaction is open, or a callback runs while one commits (also an undo manager or an awareness in the middle of a call or one of its callbacks; a transaction used inside its own commit's callbacks). ykt throws instead of waiting for a lock that can't come. |
| `TransactionCommitted` | A transaction used after `commit()` (or after `transact { }` ended): any of its methods, or passed to a type's. |
| `WrongDocument` | A transaction, a text or a document that isn't the one the call belongs to: another document's transaction passed to a type's method (root types are found by name, so it would have written into the other document), another document's text as an undo scope, a position read in another document's transaction. |
| `TypeDeleted` | The shared type was deleted from its document (a nested node its parent removed). |
| `NotIntegrated` | A preliminary type (made with its constructor, not in a document yet) where a document is needed: formatting, attributes on insert, deltas, observing, positions, an undo scope. |
| `AlreadyIntegrated` | A node already in a document inserted again (or given as a new node's child, at any depth). |
| `OutOfBounds` | An index or a range past the end: text insert / delete / format, a delta deleting past the end (checked before anything is applied), array and XML children, positions. Indexes count UTF-16 units in preliminary texts too. |
| `InvalidAttributes` | Formatting attributes that aren't a JSON object. |
| `InvalidData` | Bytes that aren't a valid update, state vector, snapshot, relative position or awareness update, or are in the other encoding (v1 / v2); an update yrs can't apply; an awareness state given as JSON that isn't JSON. Never a crash: malformed v2 input, on which yrs panics, is caught. |
| `GcEnabled` | `encodeStateFromSnapshot` on a document that collects garbage: past states need `YDocOptions(gc = false)`. |

Changed in 0.5.0 (from 0.4.0's `Error` / Kotlin `Exception`): `AnotherTx` and `AnotherRwTx` → `Locked`;
`TxnCommitted` → `TransactionCommitted`; `RefDisposed` → `TypeDeleted`; `InvalidPrelimOp` →
`NotIntegrated`; `NotPrelim` → `AlreadyIntegrated`; `InvalidFmt` → `InvalidAttributes`; `InvalidParent`
→ `InvalidData`; `Custom` → `WrongDocument` (the only thing it said); `InvalidTransactionCtx` and
`InvalidDelta` removed (nothing threw them); `GcEnabled` new. Calls that used to panic (an index past
the end of a text or of XML children, a delta deleting past the end, a handle or a transaction used
inside its own commit's callbacks, malformed v2 bytes in `applyUpdateV2`) or to read freed memory (a
transaction's methods after its commit) throw the case above instead.

## Roadmap / What’s implemented
Source is inspired by y-crdt’s ywasm project.

- Platforms
  - [x] Kotlin/JVM + tests
  - [x] Swift (macOS, iOS, iOS Simulator, Mac Catalyst) + tests; SwiftPM by URL and version (0.5.0 on)
  - [ ] Kotlin Multiplatform + tests

- Documentation
  - [ ] Migrate doc examples from Wasm to Kotlin code

- Core types
  - [x] YDocument
    - [x] observe_update_v1 / observe_update_v2 (update + origin)
    - [x] observe_transaction_cleanup (state vectors before/after, delete set, origin)
    - [x] observe_after_transaction, observe_subdocs, observe_destroy (+ `destroyDoc()`, `load()`)
  - [x] YTransaction (basic: origin, state vectors, diff, apply, commit)
  - [x] YText
    - [x] insert, format: attributes as JSON or typed (`insertWithAttributes`, `formatWithAttributes`)
    - [x] to_delta
    - [x] apply_delta
    - [x] observe / unobserve (`YSubscription`: delta + origin)
    - [x] observe_deep (the text and the shared types embedded in it)
    - [x] id (`YBranchId`)
    - [x] insert_embed (plain values)
    - [ ] quote
  - [x] XML: YXmlElement, YXmlFragment, YXmlText
    - [x] YXmlElement, YXmlFragment, YXmlText: observe (`YXmlEvent`), observe_deep
    - [ ] YXmlElement / YXmlFragment: tree_walker
    - [ ] YXmlText: quote, apply_delta
  - [x] YMap (+ observe: key changes with old / new values), plain values and nested shared types
  - [x] YArray (+ observe: delta), plain values and nested shared types
  - [ ] YWeakLink

- Collaboration
  - [x] Relative positions (yrs `StickyIndex`, Yjs `RelativePosition`): a caret or a range that keeps its
    place through remote edits; encode/decode (in YText; YArray positions later)
  - [x] YUndoManager (scope: texts, maps, arrays; tracked origins, capture timeout, stack item events
    with meta)
  - [x] Update utilities without a document: merge_updates, diff_update,
    encode_state_vector_from_update (v1 and v2)
  - [x] Awareness (`YAwareness`: local state, peers' states, updates, outdated check, observers)

- Utilities & ergonomics
  - [x] Typed attributes (`YAttributes`) besides JSON strings for insert / format
  - [x] YOutput: plain values and handles of every shared type (maps, arrays, events); a text's
    deltas still give embedded shared types in their JSON form
  - [ ] Thread safety without external locking (yrs `sync`, a lock instead of `RefCell`)
  - [x] `getText` / `getXmlFragment` throw while the document is locked (as `getMap` / `getArray`)
    instead of waiting for the lock — inside a transaction or a callback they blocked for good

## Contributing
Issues and PRs are welcome! If you’re missing a platform or a specific API, please open an issue to discuss design and approach.

## License
This project is licensed under the terms of the [MIT License](LICENSE).

## Maintainer
- [Sasha Zverev](https://github.com/planerist)
