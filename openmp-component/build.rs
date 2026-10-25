fn main() {
    cxx_build::bridge("src/lib.rs")
        .std("c++17")
        .compile("cef-openmp-cxxbridge");

    println!("cargo:rerun-if-changed=src/lib.rs");
}
