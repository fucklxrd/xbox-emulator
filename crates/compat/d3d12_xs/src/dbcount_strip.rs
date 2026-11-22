use std::collections::HashSet;

pub const RO: &str = "__tbro";
pub const FAULT_UAV: &str = "tb_gfault";

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn ident_end(b: &[u8], i: usize) -> usize {
    let mut j = i;
    while j < b.len() && is_ident(b[j]) {
        j += 1;
    }
    j
}

fn skip_ws(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i
}

fn close_of(b: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = open;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn statement_end(b: &[u8], start: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut i = start;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth < 0 {
                    return None;
                }
            }
            b';' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
}

fn split_args(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => {
                out.push(s[start..i].trim());
                start = i + 1;
            }
            _ => {}
        }
    }
    if !s[start..].trim().is_empty() || !out.is_empty() {
        out.push(s[start..].trim());
    }
    out
}

fn statement_start(b: &[u8], i: usize) -> bool {
    let mut k = i;
    while k > 0 && b[k - 1].is_ascii_whitespace() {
        k -= 1;
    }
    if k == 0 {
        return true;
    }
    match b[k - 1] {
        b';' | b'{' | b'}' | b')' => true,
        c if is_ident(c) => {
            let mut s = k;
            while s > 0 && is_ident(b[s - 1]) {
                s -= 1;
            }
            matches!(&b[s..k], b"else" | b"do")
        }
        _ => false,
    }
}

fn assign_at(b: &[u8], m: usize) -> bool {
    let at = |k: usize| b.get(m + k).copied().unwrap_or(0);
    match at(0) {
        b'=' => at(1) != b'=',
        b'+' | b'-' | b'*' | b'/' | b'%' | b'&' | b'|' | b'^' => at(1) == b'=',
        b'<' => at(1) == b'<' && at(2) == b'=',
        b'>' => at(1) == b'>' && at(2) == b'=',
        _ => false,
    }
}

fn returns_value(func: &str, args: usize) -> bool {
    match func {
        "InterlockedCompareStore" => false,
        "InterlockedCompareExchange" => true,
        _ => args >= 3,
    }
}

pub fn uav_names(h: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut off = 0;
    for line in h.split_inclusive('\n') {
        let b = line.as_bytes();
        let mut i = skip_ws(b, 0);
        while line[i..].starts_with("globallycoherent") {
            i = skip_ws(b, i + "globallycoherent".len());
        }
        let ty = &line[i..];
        if (ty.starts_with("RW") || ty.starts_with("RasterizerOrdered")) && line.contains("register(u") {
            let mut j = ident_end(b, i);
            if j < b.len() && b[j] == b'<' {
                let mut d = 0;
                while j < b.len() {
                    if b[j] == b'<' {
                        d += 1;
                    }
                    if b[j] == b'>' {
                        d -= 1;
                        if d == 0 {
                            j += 1;
                            break;
                        }
                    }
                    j += 1;
                }
            }
            let n = skip_ws(b, j);
            let e = ident_end(b, n);
            if e > n {
                out.push((off + n, line[n..e].to_string()));
            }
        }
        off += line.len();
    }
    out
}

fn out_functions(h: &str) -> HashSet<String> {
    let b = h.as_bytes();
    let mut f = HashSet::new();
    let mut i = 0;
    while i < b.len() {
        if is_ident(b[i]) && (i == 0 || !is_ident(b[i - 1])) {
            let e = ident_end(b, i);
            let w = &h[i..e];
            if w == "out" || w == "inout" {
                let mut depth = 0i32;
                let mut k = i;
                while k > 0 {
                    k -= 1;
                    match b[k] {
                        b')' => depth += 1,
                        b'(' if depth == 0 => {
                            let mut t = k;
                            while t > 0 && b[t - 1].is_ascii_whitespace() {
                                t -= 1;
                            }
                            let mut s = t;
                            while s > 0 && is_ident(b[s - 1]) {
                                s -= 1;
                            }
                            if s < t {
                                f.insert(h[s..t].to_string());
                            }
                            break;
                        }
                        b'(' => depth -= 1,
                        _ => {}
                    }
                }
            }
            i = e;
        } else {
            i += 1;
        }
    }
    f
}

fn enclosing_calls(b: &[u8], pos: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut k = pos;
    while k > 0 {
        k -= 1;
        match b[k] {
            b')' | b']' => depth += 1,
            b'[' if depth > 0 => depth -= 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                let mut t = k;
                while t > 0 && b[t - 1].is_ascii_whitespace() {
                    t -= 1;
                }
                let mut s = t;
                while s > 0 && is_ident(b[s - 1]) {
                    s -= 1;
                }
                out.push(String::from_utf8_lossy(&b[s..t]).into_owned());
            }
            b'{' | b';' if depth == 0 => break,
            _ => {}
        }
    }
    out
}

pub fn strip_uav_writes(h: &str) -> Result<String, String> {
    let decls = uav_names(h);
    if decls.is_empty() {
        return Err("no UAV declarations in the pixel shader".into());
    }
    let at: HashSet<usize> = decls.iter().map(|d| d.0).collect();
    let names: HashSet<&str> = decls.iter().map(|d| d.1.as_str()).collect();
    let b = h.as_bytes();
    let mut out = String::with_capacity(h.len());
    let mut i = 0;
    while i < b.len() {
        if h[i..].starts_with("//") {
            let e = h[i..].find('\n').map(|x| i + x).unwrap_or(b.len());
            out.push_str(&h[i..e]);
            i = e;
            continue;
        }
        if h[i..].starts_with("/*") {
            let e = h[i + 2..].find("*/").map(|x| i + 2 + x + 2).unwrap_or(b.len());
            out.push_str(&h[i..e]);
            i = e;
            continue;
        }
        if !(is_ident(b[i]) && !b[i].is_ascii_digit() && (i == 0 || !is_ident(b[i - 1]))) {
            let c = h[i..].chars().next().unwrap();
            out.push(c);
            i += c.len_utf8();
            continue;
        }
        let j = ident_end(b, i);
        let id = &h[i..j];
        if at.contains(&i) {
            out.push_str(id);
            out.push_str(RO);
            i = j;
            continue;
        }
        let after = skip_ws(b, j);
        if id.starts_with("Interlocked") && b.get(after) == Some(&b'(') {
            let close = close_of(b, after).ok_or_else(|| format!("unbalanced {id} call"))?;
            let args = split_args(&h[after + 1..close]);
            let dest = args.first().map(|a| &a[..ident_end(a.as_bytes(), 0)]).unwrap_or("");
            if names.contains(dest) {
                if returns_value(id, args.len()) {
                    return Err(format!("{id} on UAV {dest} returns the old value, which a counting replay cannot reproduce"));
                }
                if !statement_start(b, i) {
                    return Err(format!("{id} on UAV {dest} is not a statement"));
                }
                let semi = skip_ws(b, close + 1);
                if b.get(semi) != Some(&b';') {
                    return Err(format!("{id} on UAV {dest} is not a statement"));
                }
                out.push_str("{}");
                i = semi + 1;
                continue;
            }
            out.push_str(id);
            i = j;
            continue;
        }
        if !names.contains(id) || (i > 0 && b[i - 1] == b'.') {
            out.push_str(id);
            i = j;
            continue;
        }
        match b.get(after) {
            Some(b'[') => {
                let close = close_of(b, after).ok_or_else(|| format!("unbalanced subscript of UAV {id}"))?;
                let m = skip_ws(b, close + 1);
                if assign_at(b, m) {
                    if !statement_start(b, i) {
                        return Err(format!("store to UAV {id} inside an expression"));
                    }
                    let e = statement_end(b, i).ok_or_else(|| format!("unterminated store to UAV {id}"))?;
                    out.push_str("{}");
                    i = e + 1;
                    continue;
                }
                if h[m..].starts_with("++") || h[m..].starts_with("--") {
                    return Err(format!("increment of UAV {id}"));
                }
                out.push_str(id);
                out.push_str(RO);
                i = j;
            }
            Some(b'.') => {
                let ms = skip_ws(b, after + 1);
                let me = ident_end(b, ms);
                let method = &h[ms..me];
                if method.starts_with("Store") {
                    if !statement_start(b, i) {
                        return Err(format!("{id}.{method} inside an expression"));
                    }
                    let e = statement_end(b, i).ok_or_else(|| format!("unterminated {id}.{method}"))?;
                    out.push_str("{}");
                    i = e + 1;
                } else if method.starts_with("Interlocked") {
                    let open = skip_ws(b, me);
                    if b.get(open) != Some(&b'(') {
                        return Err(format!("{id}.{method} without arguments"));
                    }
                    let close = close_of(b, open).ok_or_else(|| format!("unbalanced {id}.{method}"))?;
                    let args = split_args(&h[open + 1..close]);
                    let semi = skip_ws(b, close + 1);
                    if !statement_start(b, i) || b.get(semi) != Some(&b';') {
                        return Err(format!("{id}.{method} is not a statement"));
                    }
                    if !returns_value(method, args.len()) {
                        out.push_str("{}");
                    } else if id == FAULT_UAV && args.len() >= 3 {
                        out.push_str(&format!("{} = {id}{RO}.Load({});", args[args.len() - 1], args[0]));
                    } else {
                        return Err(format!("{id}.{method} returns the old value, which a counting replay cannot reproduce"));
                    }
                    i = semi + 1;
                } else if method.starts_with("Append") || method.starts_with("Consume") || method.contains("Counter") {
                    return Err(format!("{id}.{method} changes the UAV counter"));
                } else {
                    out.push_str(id);
                    out.push_str(RO);
                    i = j;
                }
            }
            _ => return Err(format!("UAV {id} used outside a subscript or method call")),
        }
    }
    let outs = out_functions(&out);
    let ob = out.as_bytes();
    let mut p = 0;
    while let Some(k) = out[p..].find(RO) {
        let pos = p + k;
        for c in enclosing_calls(ob, pos) {
            if c.starts_with("Interlocked") || outs.contains(&c) {
                return Err(format!("a UAV is passed to {c}, which may write it"));
            }
        }
        p = pos + RO.len();
    }
    Ok(out)
}
