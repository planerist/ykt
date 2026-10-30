#!/bin/zsh
set -e

cd "$(dirname "$0")"

OUT=target/swift

cargo build --release -p yrs_uniffi
rm -rf $OUT
cargo run --release --features=uniffi/cli --bin uniffi-bindgen generate --language swift --out-dir $OUT --library target/release/libyrs_uniffi.dylib
# uniffi 0.32 `rename` (see uniffi.toml) misses the error type of throwing constructors
sed -i '' 's/FfiConverterTypeError_lift/FfiConverterTypeYError_lift/g' $OUT/Ykt.swift

mkdir -p $OUT/include
cp $OUT/YktFFI.h $OUT/include/
cp $OUT/YktFFI.modulemap $OUT/include/module.modulemap

rm -rf yrs_swift/YktFFI.xcframework
xcodebuild -create-xcframework -library target/release/libyrs_uniffi.a -headers $OUT/include -output yrs_swift/YktFFI.xcframework

mkdir -p yrs_swift/Sources/Ykt
cp $OUT/Ykt.swift yrs_swift/Sources/Ykt/
