use std::collections::BTreeSet;
use std::path::PathBuf;

const NATIVE: &[&str] = &[
    "MFStartup",
    "MFShutdown",
    "MFResetDXGIDeviceManagerX",
    "MFCreateDXGISurfaceBufferX",
    "LFGetGlobalPool",
    "MFBlockThread",
    "MFUnblockThread",
    "MFJoinIoPort",
    "MFGetPrivateWorkqueues",
    "MFGetPlatform",
];

const MISSING: &[&str] = &[];

const SURFACE_ALIAS: (&str, &str) = ("MFCreateDXGISurfaceBufferX", "MFCreateDXGISurfaceBuffer");

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

fn pe_export_names(bytes: &[u8]) -> Option<Vec<String>> {
    let pe = u32_at(bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let nsec = u16_at(bytes, pe + 6)? as usize;
    let optsz = u16_at(bytes, pe + 20)? as usize;
    let opt = pe + 24;
    let dd = opt + if u16_at(bytes, opt)? == 0x20b { 112 } else { 96 };
    let erva = u32_at(bytes, dd)?;
    if erva == 0 {
        return Some(Vec::new());
    }
    let secs = opt + optsz;
    let map = |rva: u32| -> Option<usize> {
        (0..nsec).find_map(|i| {
            let s = secs + i * 40;
            let va = u32_at(bytes, s + 12)?;
            let vs = u32_at(bytes, s + 8)?.max(u32_at(bytes, s + 16)?);
            let raw = u32_at(bytes, s + 20)?;
            (rva >= va && rva < va + vs).then(|| (rva - va + raw) as usize)
        })
    };
    let ed = map(erva)?;
    let nnames = u32_at(bytes, ed + 24)? as usize;
    let names = map(u32_at(bytes, ed + 32)?)?;
    let mut out = Vec::with_capacity(nnames);
    for i in 0..nnames {
        let p = map(u32_at(bytes, names + i * 4)?)?;
        let end = bytes.get(p..)?.iter().position(|&c| c == 0)?;
        out.push(String::from_utf8(bytes[p..p + end].to_vec()).ok()?);
    }
    Some(out)
}

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

fn read_list(path: &str) -> Vec<String> {
    println!("cargo:rerun-if-changed={path}");
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{path}: {e}"))
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

fn main() {
    let console = read_list("console_exports.txt");
    let embedded = read_list("pc_exports.txt");
    let sysroot = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let sys = PathBuf::from(sysroot).join("System32").join("mfplat.dll");
    println!("cargo:rerun-if-changed={}", sys.display());
    let host = std::fs::read(&sys).ok().and_then(|b| pe_export_names(&b));
    let source = if host.is_some() { "host System32 mfplat.dll" } else { "embedded pc_exports.txt" };
    let pc: BTreeSet<String> = host.unwrap_or(embedded).into_iter().collect();

    let mut forwarded: BTreeSet<String> = pc.iter().filter(|n| !NATIVE.contains(&n.as_str())).cloned().collect();
    for c in &console {
        if !NATIVE.contains(&c.as_str()) && !MISSING.contains(&c.as_str()) {
            forwarded.insert(c.clone());
        }
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").map(|e| e == "msvc").unwrap_or(false);
    if msvc {
        emit_exports(&out_dir, &forwarded);
    }

    let mut rs = String::new();
    rs.push_str(&format!("pub const FORWARD_SOURCE: &str = {source:?};\n"));
    rs.push_str("pub static FORWARDED: &[&str] = &[\n");
    for n in &forwarded {
        rs.push_str(&format!("    {n:?},\n"));
    }
    rs.push_str("];\n");
    rs.push_str("pub static MISSING: &[&str] = &[\n");
    for n in MISSING {
        rs.push_str(&format!("    {n:?},\n"));
    }
    rs.push_str("];\n");
    rs.push_str("pub static CONSOLE_EXPORTS: &[&str] = &[\n");
    for n in &console {
        rs.push_str(&format!("    {n:?},\n"));
    }
    rs.push_str("];\n");
    rs.push_str(&format!("pub static FORWARDED_ALIASES: &[(&str, &str)] = &[({:?}, {:?})];\n", SURFACE_ALIAS.0, SURFACE_ALIAS.1));
    std::fs::write(out_dir.join("forwarded.rs"), rs).unwrap();
}

#[cfg(not(windows))]
fn emit_exports(_: &std::path::Path, forwarded: &BTreeSet<String>) {
    for name in NATIVE.iter().filter(|name| **name != SURFACE_ALIAS.0) {
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{name}");
    }
    for name in forwarded {
        println!("cargo:rustc-cdylib-link-arg=/EXPORT:{name}=mfplat_sys.{name}");
    }
    let (alias, target) = SURFACE_ALIAS;
    println!("cargo:rustc-cdylib-link-arg=/EXPORT:{alias}=mfplat_sys.{target}");
}

#[cfg(windows)]
fn emit_exports(out_dir: &std::path::Path, forwarded: &BTreeSet<String>) {
    let mut def = String::from("LIBRARY mfplat\nEXPORTS\n");
    for n in NATIVE.iter().filter(|n| **n != SURFACE_ALIAS.0) {
        def.push_str(&format!("    {n}\n"));
    }
    for n in forwarded {
        def.push_str(&format!("    {n}=mfplat_sys.{n}\n"));
    }
    def.push_str(&format!("    {}=mfplat_sys.{}\n", SURFACE_ALIAS.0, SURFACE_ALIAS.1));
    let def_path = out_dir.join("mfplat_forward.def");
    let lib_path = out_dir.join("mfplat_forward.lib");
    let exp_path = out_dir.join("mfplat_forward.exp");
    std::fs::write(&def_path, def).unwrap();
    let lib = find_lib_exe().expect("lib.exe (MSVC librarian) not found: needed to build the mfplat forwarder export table");
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
