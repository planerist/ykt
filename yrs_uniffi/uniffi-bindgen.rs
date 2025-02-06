use uniffi;

fn main() {
    uniffi::uniffi_bindgen_main()
}

// cargo build --release  --package=yrs-uniffii
// cargo run --features=uniffi/cli --bin uniffi-bindgen generate --language kotlin --out-dir yrs_kt/src/main/kotlin --library target/release/libyrs_uniffi.dylib

// debug:
//  cargo run --features=uniffi/cli --bin uniffi-bindgen generate --language kotlin --out-dir yrs_kt/src/main/kotlin --library target/debug/libyrs_uniffi.dylib
