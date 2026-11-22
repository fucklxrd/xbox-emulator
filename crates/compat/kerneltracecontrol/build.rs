fn main() {
    println!("cargo:rerun-if-changed=exports.txt");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    let text = std::fs::read_to_string("exports.txt").expect("exports.txt");
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.get(2) == Some(&"missing") {
            continue;
        }
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{},@{}", f[1], f[0]);
    }
}
