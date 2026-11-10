//! Generates the Swift and Kotlin bindings from the built library (see scripts/mobile-bindings.sh).
fn main() {
    uniffi::uniffi_bindgen_main()
}
