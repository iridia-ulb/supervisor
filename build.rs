use std::path::Path;



fn main() {
    /* tell Cargo to rerun this build script if the given directories change */
    println!("cargo:rerun-if-changed=client/src");
    println!("cargo:rerun-if-changed=client/public");
    println!("cargo:rerun-if-changed=shared/src");
    /* set up some environmental variables */
    let out_dir = std::env::var_os("OUT_DIR").unwrap();
    let js_file = Path::new(&out_dir).join("client.js");
    let wasm_file = Path::new(&out_dir).join("client_bg.wasm"); 
    /* build the Web-Assembly module */
    let success = std::process::Command::new("wasm-pack")
        .args(&[
            "build",
            "--target",
            "web",
            "--out-name",
            "client",
            "--out-dir"
        ])
        .arg(out_dir)
        .arg("client")
        .spawn()
        .expect("Could not execute wasm-pack, is it installed? https://rustwasm.github.io/wasm-pack/installer/")
        .wait()
        .expect("Could not execute wasm-pack, is it installed? https://rustwasm.github.io/wasm-pack/installer/")
        .success();
    /* check compilation exit code and if generated files exist */
    assert!(success, "Failed to compile client");
    eprintln!("Checking if {} exists", wasm_file.to_string_lossy());
    assert!(wasm_file.exists(), "WebAssembly module not generated");
    eprintln!("Checking if {} exists", js_file.to_string_lossy());
    assert!(wasm_file.exists(), "Javascript bindings not generated");
    /* set up some environmental variables */
    println!("cargo:rustc-env=CLIENT_JS={}", js_file.display());
    println!("cargo:rustc-env=CLIENT_WASM={}", wasm_file.display());
}
