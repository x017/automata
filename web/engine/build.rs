//! Exports only the module's entry point from the browser build.
//!
//! rustc exports every `#[no_mangle]` function of every linked crate from a
//! cdylib, and wgpu's GL backend links wasm-bindgen on any wasm32 target
//! (unused under Emscripten). Its exports would keep imports from
//! wasm-bindgen's placeholder module, which Pyodide can't provide. Emscripten
//! takes the last `-sEXPORTED_FUNCTIONS`, and this one comes after rustc's.

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("emscripten") {
        println!("cargo:rustc-link-arg-cdylib=-sEXPORTED_FUNCTIONS=_PyInit__core");
    }
}
