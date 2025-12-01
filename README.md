# Ykt

Kotlin bindings for the [Y‑CRDT](https://github.com/y-crdt/) ecosystem. Ykt brings collaborative, conflict‑free data types to Kotlin applications by wrapping the high‑performance Rust implementation (Yrs) via UniFFI. Use it to build realtime text editors, notes, whiteboards, and any app that needs local‑first collaboration and seamless sync across devices and platforms.

## Why Ykt?
- Built on Yrs (Rust): safety, performance, and a mature CRDT core
- Kotlin‑first ergonomics with a small, focused API surface
- Interoperates with other Y‑CRDT bindings (JS, Rust, Swift, etc.) for cross‑platform sync

## Status
This project is actively developed and used in real projects. The foundation is stable thanks to Yrs. The primary limitation at this stage is an incomplete Kotlin API surface (not stability or performance).

Currently solid: YText (deltas, formatting, transactions), basic XML types, and core document/transaction flow.

## Quick start

Ykt uses Mozilla UniFFI to bridge Kotlin and Rust.

Project layout:
- [yrs_uniffi](yrs_uniffi) — UniFFI bindings for Yrs (Rust)
- [yrs_kt](yrs_kt) — Kotlin wrapper and basic tests

### Build artifacts
Prebuilt JARs are available in GitHub Actions.

Supported targets today:
- macOS (Darwin): aarch64 & x86_64
- Linux: x86_64

Adding more targets is straightforward; open an issue or PR if you need one.

### Install
Option A — Download the JAR from GitHub Actions and include it in your classpath.

Option B — Build locally:
```
./gradlew build
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

## Roadmap / What’s implemented
Source is inspired by y-crdt’s ywasm project.

- Kotlin
  - [x] JVM + basic tests
  - [ ] Multiplatform + tests

- Documentation
  - [ ] Migrate doc examples from Wasm to Kotlin code

- Core types
  - [x] YDocument
  - [x] YTransaction (basic)
  - [x] YText
    - [x] insert (Attrs interface will improve)
    - [x] to_delta
    - [x] apply_delta
    - [ ] id
    - [ ] insert_embed
    - [ ] quote
    - [ ] observe / observe_deep / unobserve / unobserve_deep
  - [x] XML: YXmlElement, YXmlFragment, YXmlText
    - [ ] YXmlElement: tree_walker, observe/unobserve, observe_deep
    - [ ] YXmlFragment: tree_walker, observe/unobserve, observe_deep
    - [ ] YXmlEvent
    - [ ] YXmlText: quote, apply_delta, observe/unobserve, observe_deep

- Utilities & ergonomics
  - [ ] Better Attrs interface
  - [ ] YOutput: full type coverage (currently only string)
  - [ ] Awareness
  - [ ] YMap
  - [ ] YUndoManager
  - [ ] YWeakLink
  - [ ] Review & simplify Error types

## Contributing
Issues and PRs are welcome! If you’re missing a platform or a specific API, please open an issue to discuss design and approach.

## License
This project is licensed under the terms of the [MIT License](LICENSE).

## Maintainer
- [Sasha Zverev](https://github.com/planerist)
