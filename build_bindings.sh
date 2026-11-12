#!/bin/zsh

cd "$(dirname "$0")"

cargo build --release -p yrs_uniffi
cargo run --release --features=uniffi/cli --bin uniffi-bindgen generate --language kotlin --out-dir yrs_kt/src/main/kotlin --library target/release/libyrs_uniffi.$1

mkdir -p yrs_kt/src/main/resources/$2
cp -fr target/release/libyrs_uniffi.$1 yrs_kt/src/main/resources/$2