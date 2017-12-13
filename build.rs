//use std::env;

fn main() {
    if cfg!(target_os = "windows") {
        println!("cargo:rustc-link-lib=static=LIBCMT");
        println!("cargo:rustc-link-lib=static=MSVCRT");
    }
}
