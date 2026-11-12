# Vendored crates

## yrs 0.28.1, patched

`yrs/` is [yrs 0.28.1](https://crates.io/crates/yrs/0.28.1) from crates.io (MIT, [y-crdt](https://github.com/y-crdt/y-crdt)) with two fixes. The workspace's `Cargo.toml` uses it through `[patch.crates-io]`.

- **`src/update.rs`, `Update::integrate`.** A client's update that arrives before that client's earlier ones now waits for them, as in Yjs. yrs integrated it over the gap, with a skip in the block store, so the document depended on the order updates arrived in, and some were left out for good (0.28.1's retry of pending updates, y-crdt#739, made that rarer but not gone). Tests: `yrs_uniffi/src/out_of_order_tests.rs`.
- **`src/block.rs`, `src/slice.rs`.** A skip's length is written with `write_var`, where decoders read it. In lib0 v2 it went to the length column, and the bytes didn't decode.

To drop the patch once a yrs release has both fixes: delete `vendor/yrs` and the `[patch.crates-io]` section, bump `yrs` in `yrs_uniffi/Cargo.toml`, run `cargo update -p yrs`.
