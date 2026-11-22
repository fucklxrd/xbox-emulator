use std::path::PathBuf;

const SHADERS: &[&str] = &["fortnite_vs_sample.dxbc", "fortnite_ps_sample.dxbc", "fortnite_cs_clear.dxbc", "xdefiant_vs_swappc_s4.dxbc", "xdefiant_ps_frontface_quad.dxbc"];
const CONSOLE_XSC: &[&str] = &["console_xsc/as2.dxbc", "console_xsc/as3.dxbc", "console_xsc/ms3.dxbc"];

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../analysis/shaders");
    for cfg in ["tombox_fixtures_shaders", "tombox_fixtures_console_xsc", "tombox_fixtures_mesh"] {
        println!("cargo:rustc-check-cfg=cfg({cfg})");
    }
    let present = |names: &[&str]| {
        names.iter().all(|n| {
            let p = root.join(n);
            println!("cargo:rerun-if-changed={}", p.display());
            p.is_file()
        })
    };
    if present(SHADERS) {
        println!("cargo:rustc-cfg=tombox_fixtures_shaders");
    }
    if present(CONSOLE_XSC) {
        println!("cargo:rustc-cfg=tombox_fixtures_console_xsc");
    }
    let mesh = root.join("mesh");
    println!("cargo:rerun-if-changed={}", mesh.display());
    if mesh.is_dir() {
        println!("cargo:rustc-cfg=tombox_fixtures_mesh");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
