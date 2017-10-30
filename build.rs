//use std::env;

fn main(){
    println!("cargo:rustc-link-lib=static=LIBCMT");
    println!("cargo:rustc-link-lib=static=MSVCRT");
}