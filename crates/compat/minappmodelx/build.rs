fn main() {
    println!("cargo:rerun-if-changed=exports.txt");
    let table = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("exports.txt");
    println!("cargo:rustc-env=MAM_EXPORTS={}", table.display());
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    let text = std::fs::read_to_string(&table).expect("exports.txt");
    for e in xone_exports::parse(&text) {
        assert_eq!(e.status, "implemented", "{}: only implemented rows are exported", e.name);
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{}=mam_{},@{}", e.name, e.name, e.ordinal);
    }
}
