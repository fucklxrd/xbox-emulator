use std::path::{Path, PathBuf};

pub fn sys_targets(data: &[u8]) -> Vec<String> {
    let Ok(pe) = tombox_pe::PeFile::parse(data) else { return Vec::new() };
    let mut v: Vec<String> = Vec::new();
    for (_, target) in pe.exports.iter().flat_map(|e| e.forwarders.iter()) {
        let Some((dll, _)) = target.rsplit_once('.') else { continue };
        if dll.len() > 4 && dll.to_ascii_lowercase().ends_with("_sys") && !v.iter().any(|x| x.eq_ignore_ascii_case(dll)) {
            v.push(dll.to_string());
        }
    }
    v
}

pub fn system32() -> PathBuf {
    PathBuf::from(std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into())).join("System32")
}

pub fn stage_sys_companions(st: &Path, staged: &[String]) -> Result<Vec<String>, String> {
    let mut done: Vec<String> = Vec::new();
    for n in staged {
        let Ok(data) = std::fs::read(st.join(n)) else { continue };
        for t in sys_targets(&data) {
            if done.iter().any(|d| d.eq_ignore_ascii_case(&t)) {
                continue;
            }
            let base = &t[..t.len() - 4];
            let src = system32().join(format!("{base}.dll"));
            let dst = st.join(format!("{t}.dll"));
            std::fs::copy(&src, &dst).map_err(|e| format!("copy {} for {n}'s forwarders to {t}: {e}", src.display()))?;
            done.push(t);
        }
    }
    Ok(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sys_targets_ignore_pc_forwarders() {
        let k = std::fs::read(system32().join("kernel32.dll")).unwrap();
        assert!(sys_targets(&k).is_empty());
        assert!(sys_targets(b"not a pe").is_empty());
    }

    #[test]
    fn stage_copies_every_forwarded_sys_dll_once() {
        let in_build_dir = |build: &Path| {
            std::fs::read_dir(build.join("dab")).ok()?.flatten().map(|e| e.path().join("out").join("dab.dll")).filter(|p| p.is_file()).max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        };
        let built = std::env::current_exe().unwrap().ancestors().skip(1).find_map(|a| [a.join("dab.dll"), a.join("deps").join("dab.dll")].into_iter().find(|p| p.is_file()).or_else(|| in_build_dir(&a.join("build"))));
        let Some(built) = built else { panic!("dab.dll (an xhost dev-dependency) is not in the target dir above the test executable") };
        let data = std::fs::read(&built).unwrap();
        assert_eq!(sys_targets(&data), vec!["dab_sys".to_string()]);
        let st = std::env::temp_dir().join(format!("xhost_sys_stage_{}", std::process::id()));
        std::fs::create_dir_all(&st).unwrap();
        std::fs::copy(&built, st.join("dab.dll")).unwrap();
        std::fs::copy(&built, st.join("dab2.dll")).unwrap();
        let got = stage_sys_companions(&st, &["dab.dll".into(), "dab2.dll".into(), "absent.dll".into()]).unwrap();
        assert_eq!(got, vec!["dab_sys".to_string()]);
        assert_eq!(std::fs::metadata(st.join("dab_sys.dll")).unwrap().len(), std::fs::metadata(system32().join("dab.dll")).unwrap().len());
        let _ = std::fs::remove_dir_all(&st);
    }
}
