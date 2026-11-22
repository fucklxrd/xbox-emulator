use crate::centroid_abi::TB_CENTROID_PC_DEFINE;

pub const RTFMT_PC_CENTROID: u8 = 0x80;

pub fn canonical_decls(hlsl: &str) -> String {
    let is_decl = |l: &str| {
        let s = l.trim();
        s.starts_with("bool tb_c") && s.ends_with(" = false;") && s["bool tb_c".len()..s.len() - " = false;".len()].bytes().all(|b| b.is_ascii_digit())
    };
    let lines: Vec<&str> = hlsl.split('\n').collect();
    let mut out: Vec<&str> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if is_decl(lines[i]) {
            let mut j = i;
            while j < lines.len() && is_decl(lines[j]) {
                j += 1;
            }
            let mut run: Vec<&str> = lines[i..j].to_vec();
            run.sort_by_key(|l| l.trim()["bool tb_c".len()..].split(' ').next().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0));
            out.extend(run);
            i = j;
        } else {
            out.push(lines[i]);
            i += 1;
        }
    }
    out.join("\n")
}

pub fn disable_loop_grad(hlsl: &str) -> String {
    hlsl.split('\n')
        .map(|l| if l.starts_with(char::is_whitespace) && l.trim() == "#if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6" { "    #if 0" } else { l })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn pc_define(rtfmt: &[u8; 8]) -> String {
    if rtfmt[0] & RTFMT_PC_CENTROID != 0 { format!("#define {TB_CENTROID_PC_DEFINE}\n") } else { String::new() }
}

pub fn finalize(hlsl: &str, rtfmt: &[u8; 8], loop_grad: bool) -> String {
    let mut h = canonical_decls(hlsl);
    if !loop_grad {
        h = disable_loop_grad(&h);
    }
    if rtfmt.iter().any(|&k| k != 0) {
        let defs: String = rtfmt.iter().enumerate().filter(|(_, k)| **k & 0x7f != 0).map(|(i, k)| format!("#define TB_RTFMT{i} {}u\n", k & 0x7f)).collect();
        h = pc_define(rtfmt) + &defs + &h;
    }
    h
}

pub fn loop_grad_from_env() -> bool {
    std::env::var("TOMBOX_LOOP_GRAD").map(|v| v != "0").unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finalize_matches_the_runtime_steps() {
        let src = "a\n    bool tb_c395 = false;\n    bool tb_c72 = false;\n    #if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6\nb";
        assert_eq!(finalize(src, &[0; 8], true), "a\n    bool tb_c72 = false;\n    bool tb_c395 = false;\n    #if defined(__SHADER_TARGET_MAJOR) && __SHADER_TARGET_MAJOR >= 6\nb");
        assert_eq!(finalize(src, &[0; 8], false), "a\n    bool tb_c72 = false;\n    bool tb_c395 = false;\n    #if 0\nb");
        let f = finalize("x", &[0x81, 0, 2, 0, 0, 0, 0, 0], true);
        assert_eq!(f, format!("#define {TB_CENTROID_PC_DEFINE}\n#define TB_RTFMT0 1u\n#define TB_RTFMT2 2u\nx"));
    }
}
