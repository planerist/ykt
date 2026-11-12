# Ykt

Kotlin bindings for the [Y‑CRDT](https://github.com/y-crdt/) ecosystem. Ykt brings collaborative, conflict‑free data types to Kotlin applications by wrapping the high‑performance Rust implementation (Yrs) via UniFFI. Use it to build realtime text editors, notes, whiteboards, and any app that needs local‑first collaboration and seamless sync across devices and platforms.

## Why Ykt?
- Built on Yrs (Rust): safety, performance, and a mature CRDT core
- Kotlin‑first ergonomics with a small, focused API surface
- Interoperates with other Y‑CRDT bindings (JS, Rust, Swift, etc.) for cross‑platform sync

## Status
This project is actively developed and used in real projects. The foundation is stable thanks to Yrs. The primary limitation at this stage is an incomplete Kotlin API surface (not stability or performance).

Currently solid: YText (deltas, formatting, transactions, observers), YMap and YArray of plain values (+ observers), document update and transaction observers, update utilities without a document, relative positions, undo / redo, basic XML types, and core document/transaction flow.

## Quick start

Ykt uses Mozilla UniFFI to bridge Rust to Kotlin and Swift.

Project layout:
- [yrs_uniffi](yrs_uniffi) — UniFFI bindings for Yrs (Rust)
- [yrs_kt](yrs_kt) — Kotlin wrapper and basic tests
- [yrs_swift](yrs_swift) — Swift package (SwiftPM) and basic tests

### Build artifacts
Prebuilt JARs and the Swift package are attached to [GitHub Releases](https://github.com/planerist/ykt/releases).

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
Download `ykt-swift-<version>.zip` from GitHub Releases, unzip it and add the `Ykt` folder as a local package dependency, then `import Ykt`.

To build locally (`--all` adds iOS, Mac Catalyst and both macOS architectures, requires the rustup targets):
```
./build_swift_bindings.sh
cd yrs_swift && swift test
```

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

Notes:
- API names may evolve while the Kotlin surface stabilizes.
- Transactions are recommended for consistent reads/snapshots and are required for XML APIs; YText supports direct ops without an explicit transaction.

## Observers
Callbacks from Rust into Kotlin or Swift when a document or a text changes, as Yjs's `observe` and
`on('update')`:

- `YText.observe` — the text's change as a delta (retain / insert / delete, UTF‑16 units, with
  formatting attributes) and the transaction's origin, once per committed transaction;
- `YDoc.observeUpdateV1` / `observeUpdateV2` — every committed transaction as one update, ready to
  store or send (`applyUpdate` on another replica);
- `YDoc.observeTransactionCleanup` — the state vectors before and after every transaction, and what
  it deleted.

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
```

Rules:
- Callbacks run synchronously on the thread that commits the transaction, while the document is
  still locked: they must not read or write the document (that throws `AnotherTx`); everything they
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
nor the manager may be used (that throws `AnotherTx`). `undo()`, `redo()` and `clear()` throw inside
a transaction or any callback instead of waiting for the lock. Release the manager on the thread
that uses the document; one released while the document is locked can't detach from it and stays
in memory with it (harmlessly).

## YMap and YArray
`YDoc.getMap(name)` / `YDoc.getArray(name)` — Yjs's `Y.Map` and `Y.Array` of plain values (`YValue`:
null, bool, number, bigint, string, buffer, and arrays and maps of them). Every call takes an
optional transaction, as `YText`'s.

- `YMap`: `set(key, value)`, `get(key)` (null if absent), `delete(key)` (returns the old value),
  `containsKey`, `keys()` and `values()` (in no particular order, the same for both), `len()`,
  `clear()`, `toJson()` (the whole map);
- `YArray`: `insert(index, values)`, `push(values)`, `delete(index, length)`, `get(index)`, `len()`,
  `toJson()` (the whole list); indexes past the end throw `OutOfBounds`;
- `observe` — `YMapEvent(keys, origin)` with `YEntryChange` per key (`Inserted(value)`,
  `Updated(oldValue, newValue)`, `Removed(oldValue)`), `YArrayEvent(delta, origin)` with
  `YArrayDelta` steps (`Insert(values)`, `Delete(n)`, `Retain(n)`), once per committed transaction;
  the same rules as the other observers.

Nested shared types (a `Y.Map` inside a `Y.Map`, a `Y.Text` in an array) aren't in ykt's API yet:
values are plain, and a nested type another client (Yjs) wrote reads in its JSON form — a map's
entries, an array's values, a text's string. `getMap` / `getArray` throw while the document is
locked instead of waiting for it. Numbers travel in lib0's encoding, as Yjs's: an integer outside
JS's safe range that a double holds exactly (2^60) reaches other replicas as a number, not a bigint.

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

## Roadmap / What’s implemented
Source is inspired by y-crdt’s ywasm project.

- Platforms
  - [x] Kotlin/JVM + tests
  - [x] Swift (macOS, iOS, iOS Simulator, Mac Catalyst) + tests
  - [ ] Kotlin Multiplatform + tests

- Documentation
  - [ ] Migrate doc examples from Wasm to Kotlin code

- Core types
  - [x] YDocument
    - [x] observe_update_v1 / observe_update_v2 (update + origin)
    - [x] observe_transaction_cleanup (state vectors before/after, delete set, origin)
    - [ ] observe_after_transaction, observe_subdocs, observe_destroy
  - [x] YTransaction (basic: origin, state vectors, diff, apply, commit)
  - [x] YText
    - [x] insert (Attrs interface will improve)
    - [x] to_delta
    - [x] apply_delta
    - [x] observe / unobserve (`YSubscription`: delta + origin)
    - [ ] observe_deep / unobserve_deep (changes of shared types embedded in the text)
    - [ ] id
    - [ ] insert_embed
    - [ ] quote
  - [x] XML: YXmlElement, YXmlFragment, YXmlText
    - [ ] YXmlElement: tree_walker, observe/unobserve, observe_deep
    - [ ] YXmlFragment: tree_walker, observe/unobserve, observe_deep
    - [ ] YXmlEvent
    - [ ] YXmlText: quote, apply_delta, observe/unobserve, observe_deep
  - [x] YMap (+ observe: key changes with old / new values) — plain values; nested shared types later
  - [x] YArray (+ observe: delta) — plain values; nested shared types later
  - [ ] YWeakLink

- Collaboration
  - [x] Relative positions (yrs `StickyIndex`, Yjs `RelativePosition`): a caret or a range that keeps its
    place through remote edits; encode/decode (in YText; YArray positions later)
  - [x] YUndoManager (scope, tracked origins, capture timeout, stack item events with meta; texts in
    the scope, YMap / YArray later)
  - [x] Update utilities without a document: merge_updates, diff_update,
    encode_state_vector_from_update (v1 and v2)
  - [ ] Awareness

- Utilities & ergonomics
  - [ ] Better Attrs interface (typed attributes instead of JSON strings for insert / format)
  - [ ] YOutput: full type coverage (to_delta: shared types embedded in a text)
  - [ ] Thread safety without external locking (yrs `sync`, a lock instead of `RefCell`)
  - [ ] Review & simplify Error types
  - [ ] `getText` / `getXmlFragment` throwing while the document is locked (as `getMap` / `getArray`
    do) instead of waiting for the lock — inside a transaction or a callback they block for good

## Contributing
Issues and PRs are welcome! If you’re missing a platform or a specific API, please open an issue to discuss design and approach.

## License
This project is licensed under the terms of the [MIT License](LICENSE).

## Maintainer
- [Sasha Zverev](https://github.com/planerist)
