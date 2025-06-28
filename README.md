# Ykt

Ykt is a Kotlin binding for [Y-CRDT](https://github.com/y-crdt/). It provides distributed data types that enable real-time collaboration between devices. Ykt can sync data with any other platform that has a Y-CRDT binding, allowing for seamless cross-domain communication. The library is a thin wrapper around Yrs, taking advantage of the safety and performance of Rust.

## Disclaimer
Although this project is still a work in progress, I successfully use it in my own projects. It fully supports all YText capabilities, including deltas, formatting, and transactions.

Built on top of the mature Yrs framework, it offers a stable foundation. The primary limitation at this stage is the incomplete API, not stability or performance.

## Getting Started

This project utilizes the uniffi binding, enabling yrs_kt to generate bindings for a wide variety of languages. For more information, please refer to the uniffi [supported platforms](https://mozilla.github.io/uniffi-rs/latest/).

Project organization:

- [yrs_uniffi](yrs_uniffi) is uniffi bindings for yrs
- [yrs_kt](yrs_kt) is Kotlin wrapper and basic tests for yrs_uniffi

## Build Artifacts
Check GitHub Actions for JAR artifacts. Currently supported platforms include:

- Darwin (macOS): aarch64 & x86_64
- Linux: x86_64

Additional target platforms can be added with ease.

## What is implemented / TODO list

This project source is based on the y-crdt ywasm project source.

- [x] Kotlin JVM + basic tests
- [ ] Kotlin Multiplatform + tests


- [ ] migrate doc examples from Wasm to Kotlin code


- [x] YDocument
- [x] YTransaction
  - [ ] get
- [x] YText
  - [x] insert (we need better Attrs interface)
  - [ ] id
  - [ ] insert_embed 
  - [ ] quote
  - [x] to_delta
  - [x] apply_delta
  - [ ] observe
  - [ ] observe_deep
  - [ ] unobserve
  - [ ] unobserve_deep
- [ ] Better Attrs interface
- [ ] Awareness
- [ ] YMap
- [ ] YUndoManager
- [ ] YWeakLink
- [x] YXmlElement, YXmlFragment, YXmlText
  - [ ] YXmlElement: tree_walker, observe, unobserve, observe_deep, observe_deep
  - [ ] YXmlFragment: tree_walker, observe, unobserve, observe_deep, observe_deep
  - [ ] YXmlEvent
  - [ ] YXmlText: quote, apply_delta, observe, unobserver, observe_deep
- [ ] Review & simplify Error 
- [ ] YOutput: support all types (currently only str is supported)


## Maintainers

- [Sasha Zverev](https://github.com/planerist)
