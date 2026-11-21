use std::path::PathBuf;

const EXPORTS: &[(&str, &str, u32)] = &[
    ("XAudio2Create", "XAudio2Create", 1),
    ("CreateAudioReverb", "CreateAudioReverb", 2),
    ("CreateAudioVolumeMeter", "CreateAudioVolumeMeter", 3),
    ("CreateFX", "CreateFX", 4),
    ("X3DAudioCalculate", "X3DAudioCalculate", 5),
    ("X3DAudioInitialize", "X3DAudioInitialize", 6),
    ("CreateAudioReverbV2_8", "CreateAudioReverbV2_8", 7),
    ("XAudio2CreateV2_9", "XAudio2Create", 8),
    ("XAudio2CreateWithVersionInfo", "XAudio2CreateWithVersionInfo", 9),
    ("XAudio2CreateWithSharedContexts", "XAudio2CreateWithSharedContexts", 10),
];

#[cfg(windows)]
fn find_lib_exe() -> Option<PathBuf> {
    let host = |dir: PathBuf| -> Option<PathBuf> {
        let p = dir.join("bin").join("HostX64").join("x64").join("lib.exe");
        p.exists().then_some(p)
    };
    if let Ok(d) = std::env::var("VCToolsInstallDir") {
        if let Some(p) = host(PathBuf::from(d)) {
            return Some(p);
        }
    }
    let pf86 = std::env::var("ProgramFiles(x86)").unwrap_or_else(|_| "C:\\Program Files (x86)".into());
    let vswhere = PathBuf::from(&pf86).join("Microsoft Visual Studio").join("Installer").join("vswhere.exe");
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Ok(o) = std::process::Command::new(&vswhere)
        .args(["-all", "-products", "*", "-requires", "Microsoft.VisualStudio.Component.VC.Tools.x86.x64", "-property", "installationPath"])
        .output()
    {
        roots.extend(String::from_utf8_lossy(&o.stdout).lines().map(str::trim).filter(|l| !l.is_empty()).map(PathBuf::from));
    }
    for root in roots {
        let msvc = root.join("VC").join("Tools").join("MSVC");
        let Ok(rd) = std::fs::read_dir(&msvc) else { continue };
        let mut vers: Vec<PathBuf> = rd.filter_map(|e| e.ok().map(|e| e.path())).collect();
        vers.sort();
        if let Some(p) = vers.into_iter().rev().find_map(host) {
            return Some(p);
        }
    }
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join("lib.exe")).find(|p| p.exists()))
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let mut rs = String::from("pub static CONSOLE_ORDINALS: &[(&str, u32)] = &[\n");
    for (name, _, ord) in EXPORTS {
        rs.push_str(&format!("    ({name:?}, {ord}),\n"));
    }
    rs.push_str("];\n");
    std::fs::write(out_dir.join("ordinals.rs"), rs).unwrap();
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if !msvc {
        return;
    }
    emit_exports(&out_dir);
}

#[cfg(not(windows))]
fn emit_exports(_: &std::path::Path) {
    for (name, target, ordinal) in EXPORTS {
        let symbol = if name == target {
            (*name).to_owned()
        } else {
            format!("{name}={target}")
        };
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{symbol},@{ordinal}");
    }
}

#[cfg(windows)]
fn emit_exports(out_dir: &std::path::Path) {
    let mut def = String::from("LIBRARY XAudio2_9\nEXPORTS\n");
    for (name, target, ord) in EXPORTS {
        if name == target {
            def.push_str(&format!("    {name} @{ord}\n"));
        } else {
            def.push_str(&format!("    {name}={target} @{ord}\n"));
        }
    }
    let def_path = out_dir.join("xaudio2_9.def");
    let lib_path = out_dir.join("xaudio2_9_ord.lib");
    let exp_path = out_dir.join("xaudio2_9_ord.exp");
    std::fs::write(&def_path, def).unwrap();
    let lib = find_lib_exe().expect("lib.exe (MSVC librarian) not found: needed to build the XAudio2_9 ordinal export table");
    let st = std::process::Command::new(&lib)
        .arg("/NOLOGO")
        .arg("/MACHINE:X64")
        .arg(format!("/DEF:{}", def_path.display()))
        .arg(format!("/OUT:{}", lib_path.display()))
        .status()
        .unwrap_or_else(|e| panic!("{}: {e}", lib.display()));
    assert!(st.success() && exp_path.exists(), "lib.exe failed to produce {}", exp_path.display());
    println!("cargo:rustc-cdylib-link-arg={}", exp_path.display());
}
