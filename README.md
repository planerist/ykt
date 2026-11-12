# Ykt

[Yjs](https://docs.yjs.dev) CRDTs for Kotlin/JVM and Swift: [yrs](https://github.com/y-crdt/y-crdt) (Rust) through UniFFI. It speaks the Yjs wire format, so documents sync with Yjs clients in the browser.

Used in production by [Yaranga](https://yaranga.net): a note is one `Y.Text` of lines, edited by Lexical on the web and by a native editor on iPhone, iPad and Mac, and synced through a Kotlin server.

## Install

**Swift** (macOS, iOS, iOS Simulator, Mac Catalyst):
```swift
.package(url: "https://github.com/planerist/ykt", from: "0.5.0")   // then: import Ykt
```

**Kotlin/JVM** (macOS arm64 / x86_64, Linux x86_64): take `yrs_kt-<version>.jar` from [Releases](https://github.com/planerist/ykt/releases), or run `./gradlew build`.

**From source:** `./build_swift_bindings.sh [--all]`, then `cd yrs_swift && swift test`.

## Quick start

Kotlin:
```kotlin
val doc = YDoc(YDocOptions(1u))                         // client id 1
val text = doc.getText("content")

text.insert(0u, "hello")
text.insertWithAttributes(5u, " world", mapOf("bold" to YValue.Bool(true)))
text.toText()                                           // "hello world"
text.toDelta()                                          // [insert "hello", insert " world" {bold}]

// Send every change to the other replicas…
val sub = doc.observeUpdateV2 { event -> send(event.update) }
doc.transact("editor") { txn -> text.delete(0u, 1u, txn) }   // one transaction = one update

// …and apply theirs.
applyUpdateV2(doc, bytes.toDirectByteBuffer(), "remote".toByteArray())
sub.close()
```

The same in Swift: the same names with argument labels, and most calls `throw`.
```swift
let doc = YDoc(options: YDocOptions(clientId: 1))
let text = try doc.getText(name: "content")
try text.insert(index: 0, chunk: "hello")
let sub = try doc.observeUpdateV2 { event in send(event.update) }
try doc.transact(origin: "editor") { txn in try text.delete(index: 0, length: 1, txn: txn) }
try applyUpdateV2(doc: doc, update: bytes, origin: Data("remote".utf8))
```

## Which type for which editor

| Shared type | Used by | Yjs docs |
|---|---|---|
| `YText`, rich text as a delta | [Quill](https://github.com/yjs/y-quill) | [Y.Text](https://docs.yjs.dev/api/shared-types/y.text) |
| `YText`, plain text | [CodeMirror](https://github.com/yjs/y-codemirror.next), [Monaco](https://github.com/yjs/y-monaco) | [Y.Text](https://docs.yjs.dev/api/shared-types/y.text) |
| `YXmlFragment` / `YXmlElement` | [ProseMirror](https://github.com/yjs/y-prosemirror), so also Tiptap and BlockNote | [Y.XmlFragment](https://docs.yjs.dev/api/shared-types/y.xmlfragment) |
| `YXmlText`, a tree of texts | [Lexical](https://github.com/facebook/lexical/tree/main/packages/lexical-yjs), [Slate](https://github.com/BitPhinix/slate-yjs), so also Plate | [Y.XmlText](https://docs.yjs.dev/api/shared-types/y.xmltext) |
| `YMap` / `YArray` | app state, boards, outlines | [Y.Map](https://docs.yjs.dev/api/shared-types/y.map), [Y.Array](https://docs.yjs.dev/api/shared-types/y.array) |
| `YAwareness` | cursors and presence in all of the above | [y-protocols](https://github.com/yjs/y-protocols) |

## API

Examples are in Kotlin; Swift has the same names, with argument labels. Every call that writes takes an optional `txn` as its last argument; without one, the call is its own transaction.

### Document
```kotlin
YDoc(YDocOptions(clientId, gc = true))
doc.getText(name); doc.getMap(name); doc.getArray(name); doc.getXmlFragment(name)   // root types
doc.transact(origin) { txn -> … }       // origin: who wrote, as observers see it
encodeStateVector(doc)                                         // what I have (bytes)
encodeStateAsUpdateV2(doc, decodeStateVector(theirVector))     // what they lack (null: everything)
applyUpdateV2(doc, update, origin)       // V1 variants too
snapshot(doc); encodeStateFromSnapshotV2(doc, snapshot)   // past states need gc = false
```

### Text
```kotlin
text.insert(index, "chunk")                               // takes the formatting around it
text.insertWithAttributes(index, "chunk", attrs)          // exactly these attributes
text.formatWithAttributes(index, length, mapOf("bold" to YValue.Null))  // null removes
text.delete(index, length)
text.insertEmbed(index, YValue.AttrMap(mapOf("mention" to YValue.String("note://42"))))
text.toText(); text.toDelta(); text.applyDelta(delta); text.length()
text.id()                                                 // the same on every replica
// Indexes are UTF-16 units. Attributes can also be JSON strings: insert(i, s, "{\"bold\":true}").
```

### Observers
```kotlin
text.observe { e -> if (e.originString != "editor") render(e.delta) }  // once per transaction
note.observeDeep { e -> e.events.forEach { change -> … } }  // the type and everything inside it
doc.observeUpdateV2 { e -> send(e.update) }
doc.observeTransactionCleanup { e -> e.beforeState; e.afterState; e.deleteSet }
doc.observeAfterTransaction { … }; doc.observeSubdocs { … }; doc.observeDestroy { … }
xmlFragment.observe { e -> e.children; e.attributes }      // the XML types too

sub.unsubscribe()   // or close() / use { }; in Swift, release it
```
- **Threads.** A callback runs on the committing thread, while the document is locked. Reading or writing the document there throws `Locked`: take what you need from the event.
- **One call at a time per document.** Share a document behind a lock, or give each replica its own thread and sync them by updates ([tests](yrs_kt/src/test/kotlin/ObserverTest.kt)).

### Map and array
```kotlin
val settings = doc.getMap("settings")
settings.set("theme", YValue.String("dark")); settings.get("theme"); settings.delete("theme")
settings.keys(); settings.values(); settings.len(); settings.toJson()

val tags = doc.getArray("tags")
tags.push(listOf(YValue.String("work"))); tags.insert(0u, values); tags.delete(0u, 1u); tags.get(0u)

settings.observe { e -> e.keys }       // Inserted / Updated(old, new) / Removed(old)
tags.observe { e -> e.delta }          // Insert / Delete / Retain

// Nested shared types
val title = note.setText("title", "Untitled")    // also setMap, setArray, setDoc; YArray.insertText, …
note.getOutput("meta")?.asMap                    // a Value, or a handle: Text, Map, Array, Xml…, Doc, WeakLink
```

### XML
```kotlin
val root = doc.getXmlFragment("prosemirror")
val p = YXmlElement("p", children = listOf(createXmlText("hello")))   // a new node…
root.push(YXmlChild.Element(p))                                       // …in the document from here on
p.setAttribute("level", YValue.Number(1.0)); p.nextSibling(); p.parent(); root.firstChild()
root.treeWalker()                              // every node inside, depth-first, as handles
xmlText.applyDelta(delta); xmlText.toDelta()   // as YText's; nodes embedded in it come as handles
```
`treeWalker` is Yjs's [`createTreeWalker`](https://docs.yjs.dev/api/shared-types/y.xmlfragment) without a filter: filter the list.

### Weak links
```kotlin
val quote = tags.quote(1u, 2u)                   // values 1 and 2, and what is inserted between them
val pinned = note.setWeakLink("pinned", quote)   // also YArray.insertWeakLink, YText.insertEmbedWeakLink
pinned.unquote()                                 // the quoted values now, as YOutput
val current = settings.setWeakLink("current", settings.link("theme")!!)   // a link to a map entry…
current.deref()                                  // …its value now; null once the entry is deleted
note.getOutput("pinned")?.asWeakLink             // on a replica too; observe / observeDeep hear changes
```
As yrs's [weak links](https://docs.rs/yrs/0.28.0/yrs/#weak-links-and-quotations); Yjs has none yet. `get` and `toJson` give a link as `null`.

### Positions
```kotlin
val caret = text.stickyIndex(5u, YAssoc.AFTER)   // AFTER: an insert at the caret goes before it
caret.offset(doc)?.index                         // where it is after any edits; null: not here yet
YStickyIndex.decode(caret.encode())              // Yjs's RelativePosition bytes
```

### Undo
```kotlin
val undo = YUndoManager(doc, text, YUndoOptions(captureTimeoutMillis = 500, trackedOrigins = listOf("editor")))
undo.undo(); undo.redo(); undo.canUndo(); undo.stopCapturing(); undo.expandScope(otherText)
undo.observeItemAdded { mapOf("caret" to YValue.Number(5.0)) }  // meta kept with the item…
undo.observeItemPopped { e -> e.meta["caret"] }                 // …and given back on undo
// Maps and arrays: YUndoManager.forMap / forArray, expandScopeMap / expandScopeArray.
```

### Updates without a document
```kotlin
mergeUpdatesV2(stored)                    // compact what a server keeps
diffUpdateV2(update, clientStateVector)   // answer a sync request without loading the doc
encodeStateVectorFromUpdateV2(update)
```

### Awareness
```kotlin
val awareness = YAwareness(doc)
awareness.setLocalState(YValue.AttrMap(mapOf("name" to YValue.String("Ann"))))
awareness.observeUpdate { e -> send(awareness.encodeUpdate(e.added + e.updated + e.removed)) }
awareness.applyUpdate(bytes, "remote".toByteArray())
awareness.states()                        // every peer's state
timer(3_000) { awareness.removeOutdated(30_000uL) }   // drop silent peers, as Yjs does
awareness.clearLocalState()               // going offline: then send one last update
```

## Errors

One type: `YError` in Swift, `YException` in Kotlin. Match on the case, not on the message.

| Case | When |
|---|---|
| `Locked` | The document is busy on this thread: inside a transaction or a callback |
| `TransactionCommitted` | A transaction used after its commit |
| `WrongDocument` | Another document's transaction, text, position or link |
| `TypeDeleted` | A type its parent deleted |
| `NotIntegrated` | A preliminary type (not in a document yet) where one is needed |
| `AlreadyIntegrated` | A node inserted twice |
| `OutOfBounds` | An index or a range past the end, or an empty quote |
| `InvalidAttributes` | Attributes that aren't a JSON object |
| `InvalidData` | Bytes that aren't an update, state vector, snapshot, position or awareness update |
| `GcEnabled` | A snapshot state of a document with `gc = true` |

## Planned

- [ ] Kotlin Multiplatform
- [ ] `YText.quote`

## Releasing

Set the version in `yrs_uniffi/Cargo.toml`, then run `gh workflow run main.yml --ref main -f version=x.y.z`. The workflow builds and tests everything, tags `vx.y.z` with a generated `Package.swift`, and publishes the JARs and the xcframework.

## License

[MIT](LICENSE) · [Sasha Zverev](https://github.com/planerist)
