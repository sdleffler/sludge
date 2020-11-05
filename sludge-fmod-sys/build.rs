extern crate bindgen;

use std::env;
use std::path::PathBuf;

struct FmodPaths {
    core_inc: String,
    studio_inc: String,
    core_lib_path: String,
    studio_lib_path: String,
    core_lib: String,
    studio_lib: String,
}

impl FmodPaths {
    pub fn new() -> Self {
        let api_root = if cfg!(target_os = "windows") {
            r#"C:\Program Files (x86)\FMOD SoundSystem\FMOD Studio API Windows\api\"#.to_owned()
        } else {
            panic!("unknown/unsupported target OS")
        };

        let core_inc = "-I".to_owned() + &api_root + r#"core\inc"#;
        let studio_inc = "-I".to_owned() + &api_root + r#"studio\inc"#;

        let suffix = if cfg!(target_os = "windows") {
            if cfg!(debug) {
                "L_vc"
            } else {
                "_vc"
            }
        } else {
            panic!("unknown/unsupported target OS")
        };

        let core_lib_path = api_root.clone() + r#"core\lib\x64"#;
        let core_lib = "fmod".to_owned() + suffix;
        let studio_lib_path = api_root.clone() + r#"studio\lib\x64"#;
        let studio_lib = "fmodstudio".to_owned() + suffix;

        Self {
            core_inc,
            studio_inc,
            core_lib_path,
            studio_lib_path,
            core_lib,
            studio_lib,
        }
    }
}

fn main() {
    let paths = FmodPaths::new();

    // Tell cargo to tell rustc to link the FMOD shared libs.
    println!("cargo:rustc-link-search={}", paths.core_lib_path);
    println!("cargo:rustc-link-search={}", paths.studio_lib_path);
    println!("cargo:rustc-link-lib={}", paths.core_lib);
    println!("cargo:rustc-link-lib={}", paths.studio_lib);

    // Tell cargo to invalidate the built crate whenever the wrapper changes
    println!("cargo:rerun-if-changed=wrapper.h");

    // The bindgen::Builder is the main entry point
    // to bindgen, and lets you build up options for
    // the resulting bindings.
    let bindings = bindgen::Builder::default()
        // The input header we would like to generate
        // bindings for.
        .header("wrapper.h")
        .clang_arg(&paths.core_inc)
        .clang_arg(&paths.studio_inc)
        // Tell cargo to invalidate the built crate whenever any of the
        // included header files changed.
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        // Finish the builder and generate the bindings.
        .generate()
        // Unwrap the Result and panic on failure.
        .expect("Unable to generate bindings");

    // Write the bindings to the $OUT_DIR/bindings.rs file.
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings
        .write_to_file(out_path.join("bindings.rs"))
        .expect("Couldn't write bindings!");
}
