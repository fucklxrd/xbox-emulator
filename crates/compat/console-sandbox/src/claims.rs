use std::sync::OnceLock;

pub const TOKEN_SECURITY_ATTRIBUTES: u32 = 39;
const TOKEN_STATISTICS: u32 = 10;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
const STATUS_INVALID_PARAMETER: i32 = 0xC000_000Du32 as i32;
pub const TYPE_INT64: u16 = 1;
pub const TYPE_UINT64: u16 = 2;
pub const TYPE_STRING: u16 = 3;
pub const TYPE_FQBN: u16 = 4;
pub const TYPE_SID: u16 = 5;
pub const TYPE_BOOLEAN: u16 = 6;
pub const TYPE_OCTET_STRING: u16 = 0x10;
pub const LAUNCH_INSTANCE: &str = "XBOX://LI";
pub const LAUNCH_ENV: &str = "TOMBOX_LAUNCH_INSTANCE_ID";

#[link(name = "ntdll")]
extern "system" {
    fn NtQueryInformationToken(h: usize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32;
}

#[link(name = "bcrypt")]
extern "system" {
    fn BCryptGenRandom(alg: usize, buf: *mut u8, len: u32, flags: u32) -> i32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Int(u64),
    Str(Vec<u16>),
    Fqbn(u64, Vec<u16>),
    Bytes(Vec<u8>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attribute {
    pub name: Vec<u16>,
    pub kind: u16,
    pub flags: u32,
    pub values: Vec<Value>,
}

pub fn parse_guid(s: &str) -> Option<[u8; 16]> {
    let h: String = s.trim().trim_matches(|c| c == '{' || c == '}').chars().filter(|c| *c != '-').collect();
    if h.len() != 32 {
        return None;
    }
    let n = |a: usize, b: usize| u64::from_str_radix(&h[a..b], 16).ok();
    let mut g = [0u8; 16];
    g[0..4].copy_from_slice(&(n(0, 8)? as u32).to_le_bytes());
    g[4..6].copy_from_slice(&(n(8, 12)? as u16).to_le_bytes());
    g[6..8].copy_from_slice(&(n(12, 16)? as u16).to_le_bytes());
    for i in 0..8 {
        g[8 + i] = n(16 + 2 * i, 18 + 2 * i)? as u8;
    }
    Some(g)
}

pub fn format_guid(g: &[u8; 16]) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes(g[0..4].try_into().unwrap()),
        u16::from_le_bytes(g[4..6].try_into().unwrap()),
        u16::from_le_bytes(g[6..8].try_into().unwrap()),
        g[8], g[9], g[10], g[11], g[12], g[13], g[14], g[15]
    )
}

pub fn launch_instance() -> [u8; 16] {
    static ID: OnceLock<[u8; 16]> = OnceLock::new();
    *ID.get_or_init(|| {
        if let Some(g) = std::env::var(LAUNCH_ENV).ok().and_then(|s| parse_guid(&s)) {
            return g;
        }
        let mut g = [0u8; 16];
        unsafe { BCryptGenRandom(0, g.as_mut_ptr(), 16, 2) };
        g[7] = (g[7] & 0x0f) | 0x40;
        g[8] = (g[8] & 0x3f) | 0x80;
        compat_common::record_raw("console-sandbox", "XBOX://LI", &format!("launch instance {} issued by the emulated host", format_guid(&g)));
        g
    })
}

pub fn emulated() -> Vec<Attribute> {
    vec![Attribute { name: LAUNCH_INSTANCE.encode_utf16().collect(), kind: TYPE_OCTET_STRING, flags: 0, values: vec![Value::Bytes(launch_instance().to_vec())] }]
}

unsafe fn rd<T: Copy>(p: usize) -> T {
    (p as *const T).read_unaligned()
}

unsafe fn ustr(p: usize) -> Vec<u16> {
    let len = rd::<u16>(p) as usize / 2;
    let buf = rd::<usize>(p + 8);
    if buf == 0 || len == 0 {
        Vec::new()
    } else {
        std::slice::from_raw_parts(buf as *const u16, len).to_vec()
    }
}

pub unsafe fn parse(buf: usize) -> Option<Vec<Attribute>> {
    if rd::<u16>(buf) != 1 {
        return None;
    }
    let count = rd::<u32>(buf + 4) as usize;
    let arr = rd::<usize>(buf + 8);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let a = arr + i * 0x28;
        let kind = rd::<u16>(a + 0x10);
        let n = rd::<u32>(a + 0x18) as usize;
        let vals = rd::<usize>(a + 0x20);
        let mut values = Vec::with_capacity(n);
        for j in 0..n {
            values.push(match kind {
                TYPE_INT64 | TYPE_UINT64 | TYPE_BOOLEAN => Value::Int(rd::<u64>(vals + j * 8)),
                TYPE_STRING => Value::Str(ustr(vals + j * 16)),
                TYPE_FQBN => Value::Fqbn(rd::<u64>(vals + j * 24), ustr(vals + j * 24 + 8)),
                TYPE_SID | TYPE_OCTET_STRING => {
                    let p = rd::<usize>(vals + j * 16);
                    let l = rd::<u32>(vals + j * 16 + 8) as usize;
                    Value::Bytes(if p == 0 { Vec::new() } else { std::slice::from_raw_parts(p as *const u8, l).to_vec() })
                }
                _ => return None,
            });
        }
        out.push(Attribute { name: ustr(a), kind, flags: rd::<u32>(a + 0x14), values });
    }
    Some(out)
}

fn align(n: usize) -> usize {
    (n + 7) & !7
}

fn value_size(kind: u16) -> usize {
    match kind {
        TYPE_FQBN => 24,
        TYPE_STRING | TYPE_SID | TYPE_OCTET_STRING => 16,
        _ => 8,
    }
}

pub fn size_of(attrs: &[Attribute]) -> usize {
    let mut n = 0x10 + attrs.len() * 0x28;
    for a in attrs {
        n += align(a.name.len() * 2) + a.values.len() * value_size(a.kind);
        for v in &a.values {
            n += match v {
                Value::Int(_) => 0,
                Value::Str(s) | Value::Fqbn(_, s) => align(s.len() * 2),
                Value::Bytes(b) => align(b.len()),
            };
        }
    }
    n
}

pub unsafe fn write(attrs: &[Attribute], buf: usize) -> usize {
    let wr = |o: usize, v: &[u8]| core::ptr::copy_nonoverlapping(v.as_ptr(), (buf + o) as *mut u8, v.len());
    let base = 0x10;
    let mut tail = base + attrs.len() * 0x28;
    wr(0, &1u16.to_le_bytes());
    wr(2, &0u16.to_le_bytes());
    wr(4, &(attrs.len() as u32).to_le_bytes());
    wr(8, &(if attrs.is_empty() { 0 } else { buf + base }).to_le_bytes());
    let mut put = |tail: &mut usize, bytes: &[u8]| -> usize {
        let at = *tail;
        wr(at, bytes);
        *tail += align(bytes.len());
        buf + at
    };
    for (i, a) in attrs.iter().enumerate() {
        let at = base + i * 0x28;
        let name: Vec<u8> = a.name.iter().flat_map(|c| c.to_le_bytes()).collect();
        let np = put(&mut tail, &name);
        let vs = tail;
        tail += a.values.len() * value_size(a.kind);
        wr(at, &((name.len()) as u16).to_le_bytes());
        wr(at + 2, &((name.len()) as u16).to_le_bytes());
        wr(at + 4, &0u32.to_le_bytes());
        wr(at + 8, &np.to_le_bytes());
        wr(at + 0x10, &a.kind.to_le_bytes());
        wr(at + 0x12, &0u16.to_le_bytes());
        wr(at + 0x14, &a.flags.to_le_bytes());
        wr(at + 0x18, &(a.values.len() as u32).to_le_bytes());
        wr(at + 0x1c, &0u32.to_le_bytes());
        wr(at + 0x20, &(if a.values.is_empty() { 0 } else { buf + vs }).to_le_bytes());
        for (j, v) in a.values.iter().enumerate() {
            let vo = vs + j * value_size(a.kind);
            match v {
                Value::Int(x) => wr(vo, &x.to_le_bytes()),
                Value::Str(s) | Value::Fqbn(_, s) => {
                    let off = if let Value::Fqbn(ver, _) = v {
                        wr(vo, &ver.to_le_bytes());
                        8
                    } else {
                        0
                    };
                    let b: Vec<u8> = s.iter().flat_map(|c| c.to_le_bytes()).collect();
                    let p = put(&mut tail, &b);
                    wr(vo + off, &(b.len() as u16).to_le_bytes());
                    wr(vo + off + 2, &(b.len() as u16).to_le_bytes());
                    wr(vo + off + 4, &0u32.to_le_bytes());
                    wr(vo + off + 8, &p.to_le_bytes());
                }
                Value::Bytes(b) => {
                    let p = put(&mut tail, b);
                    wr(vo, &p.to_le_bytes());
                    wr(vo + 8, &(b.len() as u32).to_le_bytes());
                    wr(vo + 12, &0u32.to_le_bytes());
                }
            }
        }
    }
    tail
}

pub fn merge(mut real: Vec<Attribute>, extra: Vec<Attribute>) -> Vec<Attribute> {
    for e in extra {
        real.retain(|a| !String::from_utf16_lossy(&a.name).eq_ignore_ascii_case(&String::from_utf16_lossy(&e.name)));
        real.push(e);
    }
    real
}

unsafe fn token_id(h: usize) -> Option<u64> {
    let mut st = [0u8; 56];
    let mut ret = 0u32;
    (NtQueryInformationToken(h, TOKEN_STATISTICS, st.as_mut_ptr(), st.len() as u32, &mut ret) >= 0).then(|| u64::from_le_bytes(st[0..8].try_into().unwrap()))
}

pub unsafe fn is_process_token(h: usize) -> bool {
    if h == usize::MAX - 3 {
        return true;
    }
    match (token_id(h), token_id(usize::MAX - 3)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

unsafe fn real_attributes(h: usize) -> Result<Vec<Attribute>, i32> {
    let mut need = 0u32;
    let mut buf: Vec<u64> = vec![0; 64];
    loop {
        let st = NtQueryInformationToken(h, TOKEN_SECURITY_ATTRIBUTES, buf.as_mut_ptr() as *mut u8, (buf.len() * 8) as u32, &mut need);
        if st == STATUS_BUFFER_TOO_SMALL && need as usize > buf.len() * 8 {
            buf = vec![0; need as usize / 8 + 1];
            continue;
        }
        if st < 0 {
            return Err(st);
        }
        return parse(buf.as_ptr() as usize).ok_or(STATUS_INVALID_PARAMETER);
    }
}

pub unsafe extern "system" fn nt_query_information_token(h: usize, class: u32, buf: *mut u8, len: u32, ret: *mut u32) -> i32 {
    if class != TOKEN_SECURITY_ATTRIBUTES || !is_process_token(h) {
        return NtQueryInformationToken(h, class, buf, len, ret);
    }
    let attrs = match real_attributes(h) {
        Ok(a) => merge(a, emulated()),
        Err(e) if e == STATUS_INVALID_PARAMETER => return NtQueryInformationToken(h, class, buf, len, ret),
        Err(e) => return e,
    };
    let need = size_of(&attrs);
    if !ret.is_null() {
        *ret = need as u32;
    }
    if (len as usize) < need || buf.is_null() {
        return STATUS_BUFFER_TOO_SMALL;
    }
    write(&attrs, buf as usize);
    0
}
