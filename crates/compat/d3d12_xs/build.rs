use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=shaders/query_accum.hlsl");
    println!("cargo:rerun-if-env-changed=TOMBOX_DXC");

    let source = PathBuf::from("shaders/query_accum.hlsl");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("query_accum.dxil");

    #[cfg(windows)]
    if std::env::var_os("TOMBOX_DXC").is_none() {
        println!("cargo:rerun-if-env-changed=TOMBOX_DXC_DIR");
        let hlsl = std::fs::read_to_string(&source).expect("read query_accum.hlsl");
        let code = tombox_xshader::dxc::compile(&hlsl, "cs_6_0")
            .unwrap_or_else(|err| panic!("compile query_accum.hlsl: {err}"));
        std::fs::write(output, code).expect("write query_accum.dxil");
        return;
    }

    let dxc = std::env::var_os("TOMBOX_DXC").unwrap_or_else(|| "dxc".into());
    let status = std::process::Command::new(&dxc)
        .args(["-T", "cs_6_0", "-E", "main", "-Fo"])
        .arg(&output)
        .arg(&source)
        .status()
        .unwrap_or_else(|err| {
            panic!("could not run {:?}: {err}. Set TOMBOX_DXC to a host DXC executable", dxc)
        });
    assert!(status.success(), "DXC failed to compile query_accum.hlsl ({status})");
}
