use super::{hi64, r};
use crate::rdna2::Opnd;

pub(super) fn pair(o: &Opnd, signed: bool) -> Result<(String, String), String> {
    Ok(match o {
        Opnd::ConstInt(k) => (format!("{:#x}u", *k as i32 as u32), format!("{:#x}u", if *k < 0 { u32::MAX } else { 0 })),
        Opnd::Literal(k) => (format!("{k:#x}u"), format!("{:#x}u", if signed && (*k as i32) < 0 { u32::MAX } else { 0 })),
        Opnd::Vgpr(_) | Opnd::Sgpr(_) | Opnd::VccLo | Opnd::ExecLo => (r(o)?, r(&hi64(o))?),
        other => return Err(format!("64-bit integer compare operand {other:?} unsupported")),
    })
}

pub(super) fn cond64(cc: &str, signed: bool, a: (String, String), b: (String, String)) -> Result<String, String> {
    let ((al, ah), (bl, bh)) = (a, b);
    let (sa, sb) = if signed { (format!("asint({ah})"), format!("asint({bh})")) } else { (ah.clone(), bh.clone()) };
    let ord = |hop: &str, lop: &str| format!("(({sa} {hop} {sb}) || (({ah} == {bh}) && ({al} {lop} {bl})))");
    Ok(match cc {
        "f" => "false".into(),
        "lt" => ord("<", "<"),
        "eq" => format!("(({ah} == {bh}) && ({al} == {bl}))"),
        "le" => ord("<", "<="),
        "gt" => ord(">", ">"),
        "ne" => format!("(({ah} != {bh}) || ({al} != {bl}))"),
        "ge" => ord(">", ">="),
        "t" => "true".into(),
        other => return Err(format!("64-bit compare cond {other} unsupported")),
    })
}
