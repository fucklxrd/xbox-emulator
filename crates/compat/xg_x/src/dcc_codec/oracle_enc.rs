use super::oracle_dec::{block, cases, known, params, prefill, pull, reorder_stage, seeded, set_scratch, shim_format, stage, EncF, St, Tally};
use super::run::{self, fresh, FILLS};
use super::tests::{console, ours, same, Con, Rng};
use super::{dec, enc, Dcc, R};

fn enc_stage_chain(c: &Con, d: &mut Dcc, label: &str, t: &mut Tally) {
    d.o[0x418..0x420].fill(0);
    d.p(0xb0, 0);
    reorder_stage(c, d, 2, 0, 1, &format!("{label} reorder 1"), t);
    let a: [(usize, fn(&mut Dcc) -> R<()>, &str); 2] = [(0x18c70, enc::channel, "channel 18c70"), (0x19570, dec::signfix, "signfix 19570")];
    for (rva, f, n) in a {
        if !stage(c, d, rva, f, &format!("{label} {n}"), t) {
            return;
        }
    }
    reorder_stage(c, d, 2, 2, 2, &format!("{label} reorder 2"), t);
    if !stage(c, d, 0x19660, enc::color, &format!("{label} color 19660"), t) {
        return;
    }
    reorder_stage(c, d, 2, 2, 3, &format!("{label} reorder 3"), t);
    if !stage(c, d, 0x19980, enc::delta, &format!("{label} delta 19980"), t) {
        return;
    }
    enc::transform(d);
    let b: [(usize, fn(&mut Dcc) -> R<()>, &str); 4] = [(0x19e20, |d| Ok(enc::stats(d)), "stats 19e20"), (0x1a100, enc::tid, "tid 1a100"), (0x1a340, enc::plan, "plan 1a340"), (0x1a770, enc::pack, "pack 1a770")];
    for (rva, f, n) in b {
        if !stage(c, d, rva, f, &format!("{label} {n}"), t) {
            return;
        }
    }
}

#[test]
fn oracle_xgx_dcc_enc_stages() {
    let Some((_orc, ref or)) = console() else { return };
    let mut r = Rng(0x5eed_e1c0_4321);
    let mut t = Tally::default();
    let cfg = cases(or);
    let mut blocks = 0;
    for &(f, tm, _) in &cfg {
        for _ in 0..24 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            d.s[0] = block(&mut r);
            if r.next() % 3 == 0 {
                d.p(0x34, r.next() & 0xff);
            }
            enc_stage_chain(&c, &mut d, &format!("fmt {f} tile {tm} params {p:x?}"), &mut t);
            blocks += 1;
        }
    }
    println!("xg_x dcc enc stages: {} configs, {} blocks, {} stage calls matched the console byte-for-byte, {} stopped at a console fault, {} at undefined console state", cfg.len(), blocks, t.ok, t.fault, t.undef);
    assert!(t.ok > 10_000);
}

#[test]
fn oracle_xgx_dcc_enc_e2e() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
    let mut r = Rng(0xe2e_e1c_99);
    let cfg = cases(or);
    let (mut ok, mut stopped) = (0usize, 0usize);
    for &(f, tm, _) in &cfg {
        for _ in 0..12 {
            let p = params(&mut r);
            let (c, _) = Con::new(or, f, tm, p);
            let mut d = seeded(f, tm, p, &mut r);
            prefill(&c, &d);
            for blk in 0..(1 + r.next() % 4) {
                let s0 = block(&mut r);
                d.s[0] = s0;
                c.set_surf(0, &s0);
                let mut n = d.clone();
                if enc::run(&mut n).is_err() {
                    stopped += 1;
                    break;
                }
                drv(c.ptr());
                same(&n, &c, &format!("e2e fmt {f} tile {tm} params {p:x?} block {blk}"));
                d = n;
                ok += 1;
            }
        }
    }
    println!("xg_x dcc enc e2e: {} blocks through console driver 0x189b0 matched native object, surfaces and scratch; {} chains stopped at a console fault or undefined state", ok, stopped);
    assert!(ok > 3000);
}

#[test]
fn oracle_xgx_dcc_encode_export_shim() {
    let Some((_orc, ref or)) = console() else { return };
    let cenc: EncF = unsafe { core::mem::transmute(or.f("XGEncodeDCC")) };
    let mut r = Rng(0x5a1d_e1c0_7e11);
    let cfg = super::tests::configs();
    let (mut cases_n, mut blocks, mut same_hr, mut rejected, mut setup_rej, mut no_sizes) = (0usize, 0usize, 0usize, 0usize, 0usize, 0usize);
    for _ in 0..6000 {
        let (f, tm) = shim_format(&mut r, &cfg);
        let p = params(&mut r);
        let n = 1 + (r.next() % 6) as usize;
        let mut src = vec![0u8; n * 256];
        for i in 0..n {
            src[i * 256..i * 256 + 256].copy_from_slice(&block(&mut r));
        }
        let with_sizes = r.next() % 4 != 0;
        let sc = known(&mut r);
        set_scratch(or, &sc);
        let (mut nd, mut nm, mut ns) = (vec![0x3cu8; n * 256], vec![0x5au8; n], vec![0xdead_beefu32; n]);
        let nsz = if with_sizes { ns.as_mut_ptr() } else { core::ptr::null_mut() };
        let nh = unsafe { crate::dcc::XGEncodeDCC(n as u32, src.as_ptr(), nd.as_mut_ptr(), nm.as_mut_ptr(), nsz, f, tm, p.as_ptr() as *const crate::dcc::DccParams) };
        cases_n += 1;
        if nh != 0 && f > 0 && f <= 0x78 && ours(f, tm, p).1 == 0 {
            rejected += 1;
            assert_eq!(nh, super::E_INVALIDARG);
            continue;
        }
        let (mut cd, mut cm, mut cs) = (vec![0x3cu8; n * 256], vec![0x5au8; n], vec![0xdead_beefu32; n]);
        let csz = if with_sizes { cs.as_mut_ptr() } else { core::ptr::null_mut() };
        let ch = unsafe { cenc(n as u32, src.as_ptr(), cd.as_mut_ptr(), cm.as_mut_ptr(), csz, f, tm, p.as_ptr()) };
        assert_eq!(nh, ch, "hr fmt {f} tile {tm} params {p:x?} n {n}");
        if nh != 0 {
            setup_rej += 1;
        }
        assert_eq!(nm, cm, "meta fmt {f} tile {tm} params {p:x?}");
        assert_eq!(ns, cs, "sizes fmt {f} tile {tm} params {p:x?}");
        assert_eq!(nd, cd, "bytes fmt {f} tile {tm} params {p:x?}");
        if !with_sizes {
            no_sizes += 1;
        }
        same_hr += 1;
        blocks += n;
    }
    println!("xg_x dcc encode export shim: {} cases, {} compared with console XGEncodeDCC ({} encoded blocks, {} without a sizes array, {} argument/setup rejections with equal HRESULT), 0 mismatches in HRESULT, data, meta and sizes; {} native E_INVALIDARG for console faults or undefined output (console not called)", cases_n, same_hr, blocks, no_sizes, setup_rej, rejected);
    assert!(same_hr > 4000);
}

#[test]
fn oracle_xgx_dcc_enc_undefined_classes() {
    let Some((_orc, ref or)) = console() else { return };
    let drv: St = unsafe { core::mem::transmute(or.rva(0x189b0)) };
    let mut r = Rng(0x0dd_e1c0);
    let (mut undef, mut varied, mut faults) = (0usize, 0usize, std::collections::BTreeMap::<&str, usize>::new());
    for &(f, t, _) in &cases(or) {
        for _ in 0..30 {
            let p = params(&mut r);
            let s0 = block(&mut r);
            let sc = known(&mut r);
            let mut runs = match run::setup_runs(f, t, p, &sc) {
                Ok(v) => v,
                Err(_) => continue,
            };
            let (mut dst, mut meta) = ([0u8; 256], [0u8; 1]);
            match run::encode_blocks(&mut runs, &s0, &mut dst, &mut meta, None) {
                run::Outcome::Stopped { why, .. } if why.contains("never wrote") => {}
                run::Outcome::Stopped { why, .. } => {
                    *faults.entry(if why.contains("faults") { "fault" } else { "undefined" }).or_default() += 1;
                    continue;
                }
                run::Outcome::Done => continue,
            }
            undef += 1;
            let mut outs = Vec::new();
            for fl in 0..FILLS {
                let (c, _) = Con::new(or, f, t, p);
                let mut d = fresh(fl, &sc);
                d.setup(f, t, p[0], p[1]);
                prefill(&c, &d);
                c.set_surf(0, &s0);
                drv(c.ptr());
                let q = pull(&c, &d);
                outs.push((q.s[1], q.o[0x34]));
            }
            if outs.iter().any(|o| *o != outs[0]) {
                varied += 1;
            }
        }
    }
    println!("xg_x dcc enc undefined output: {undef} single-block cases where native sees output bytes the console never writes; console driver 0x189b0 output changed with the prefill in {varied} of them; other stops {faults:?}");
    assert_eq!(undef, varied);
}
