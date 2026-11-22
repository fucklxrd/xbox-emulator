use xtfmsg::sys::HRESULT;

fn z(s: &str) -> Vec<u8> {
    let mut v = s.as_bytes().to_vec();
    v.push(0);
    v
}

pub fn chk(file: &str, func: &str, line: u32, name: &str, hr: HRESULT, msg: &str) {
    let short = z(&format!("XtfCHK: {func}: {line}"));
    let long = z(&format!("XtfCHK: {file}: {func}: {line}"));
    xtfmsg::chk::chk(&short, &long, &z(name), hr, &z(msg));
}
