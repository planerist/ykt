#!/bin/zsh
set -e

cd "$(dirname "$0")"

OUT=target/swift
rm -rf $OUT

lib() { echo target/$1/release/libyrs_uniffi.a }

# --all: macOS (arm64 + x86_64), iOS, iOS simulator and Mac Catalyst slices; requires the rustup targets
if [[ "$1" == "--all" ]]; then
    for t in aarch64-apple-darwin x86_64-apple-darwin aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios aarch64-apple-ios-macabi x86_64-apple-ios-macabi; do
        cargo build --release -p yrs_uniffi --target $t
    done
    mkdir -p $OUT/macos $OUT/ios-sim $OUT/maccatalyst
    lipo -create $(lib aarch64-apple-darwin) $(lib x86_64-apple-darwin) -output $OUT/macos/libyrs_uniffi.a
    lipo -create $(lib aarch64-apple-ios-sim) $(lib x86_64-apple-ios) -output $OUT/ios-sim/libyrs_uniffi.a
    # Mac Catalyst (iOS apps running on the Mac)
    lipo -create $(lib aarch64-apple-ios-macabi) $(lib x86_64-apple-ios-macabi) -output $OUT/maccatalyst/libyrs_uniffi.a
    LIBS=($OUT/macos/libyrs_uniffi.a $(lib aarch64-apple-ios) $OUT/ios-sim/libyrs_uniffi.a $OUT/maccatalyst/libyrs_uniffi.a)
    BINDGEN_LIB=target/aarch64-apple-darwin/release/libyrs_uniffi.dylib
else
    cargo build --release -p yrs_uniffi
    LIBS=(target/release/libyrs_uniffi.a)
    BINDGEN_LIB=target/release/libyrs_uniffi.dylib
fi

cargo run --release --features=uniffi/cli --bin uniffi-bindgen generate --language swift --out-dir $OUT --library $BINDGEN_LIB
# uniffi 0.32 `rename` (see uniffi.toml) misses the error type of throwing constructors
sed -i '' 's/FfiConverterTypeError_lift/FfiConverterTypeYError_lift/g' $OUT/Ykt.swift

mkdir -p $OUT/include
cp $OUT/YktFFI.h $OUT/include/
cp $OUT/YktFFI.modulemap $OUT/include/module.modulemap

XCF_ARGS=()
for l in $LIBS; do
    XCF_ARGS+=(-library $l -headers $OUT/include)
done
rm -rf yrs_swift/YktFFI.xcframework
xcodebuild -create-xcframework $XCF_ARGS -output yrs_swift/YktFFI.xcframework

mkdir -p yrs_swift/Sources/Ykt
cp $OUT/Ykt.swift yrs_swift/Sources/Ykt/
