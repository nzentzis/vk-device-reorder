use std::env;
use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=src/wrapper.h");

    let bindings = bindgen::Builder::default()
                  .header("src/wrapper.h")
                  .parse_callbacks(Box::new(bindgen::CargoCallbacks))
                  .generate()
                  .expect("Unable to generate bindings");

    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());
    bindings.write_to_file(out_path.join("bindings.rs"))
            .expect("Unable to write bindings to output directory");
}
