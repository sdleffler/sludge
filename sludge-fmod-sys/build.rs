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

const WINDOWS_PATH: &str = r#"C:\Program Files (x86)\FMOD SoundSystem\FMOD Studio API Windows\api\"#;
const LINUX_PATH: &str = "/opt/fmodstudioapi20105linux/api/";

impl FmodPaths {
    pub fn new() -> Self {
        let (api_root, path_delimiter, binary_extension, suffix) = if cfg!(target_os = "windows") {
            let suffix = if cfg!(features = "debug") {
                "L_vc"
            } else {
                "_vc"
            };
            (WINDOWS_PATH.to_owned(), "\\", "64", suffix)
        } else if cfg!(target_os = "linux") {
            (LINUX_PATH.to_owned(), "/", "86_64", "")
        } else {
            panic!("unknown/unsupported target OS")
        };

        let core_inc = "-I".to_owned() + &api_root + &format!(r#"core{}inc"#, path_delimiter);
        let studio_inc = "-I".to_owned() + &api_root + &format!(r#"studio{}inc"#, path_delimiter);

        let core_lib_path = api_root.to_owned() + &format!("core{}lib{}x{}", path_delimiter, path_delimiter, binary_extension);
        let core_lib = "fmod".to_owned() + suffix;
        let studio_lib_path = api_root.to_owned() + &format!("studio{}lib{}x{}", path_delimiter, path_delimiter, binary_extension);
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
