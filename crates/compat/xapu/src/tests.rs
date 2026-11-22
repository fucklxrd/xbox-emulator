use super::*;

static SERIAL: Mutex<()> = Mutex::new(());

const FRAME: usize = 512;

fn connect(streams: u32, options: u32) -> (usize, [u8; 16]) {
    let p = [2u32, streams, 0xE, 0x10000, FRAME as u32, options];
    let mut region = [0u8; 16];
    let mut h = 0usize;
    assert_eq!(XApuConnect(p.as_ptr(), region.as_mut_ptr(), &mut h), S_OK);
    (h, region)
}

fn cmd(kind: u32, stream: u32) -> [u8; 0x40] {
    let mut c = [0u8; 0x40];
    c[0..4].copy_from_slice(&kind.to_le_bytes());
    c[4..8].copy_from_slice(&stream.to_le_bytes());
    c[8..12].copy_from_slice(&0x1234u32.to_le_bytes());
    c
}

fn start_cmd(stream: u32, ch: u32, gain: f32) -> [u8; 0x40] {
    let mut c = cmd(CMD_START, stream);
    c[0x18..0x1c].copy_from_slice(&ch.to_le_bytes());
    c[0x28..0x2c].copy_from_slice(&gain.to_le_bytes());
    c[0x2c..0x30].copy_from_slice(&(ch * (FRAME as u32 * 4 + 0x1e00)).to_le_bytes());
    c
}

fn data_cmd(stream: u32, pkt: Option<&[u8]>, skip: u32, out: &mut [f32], step: f32, target: f32, flags: u32) -> [u8; 0x40] {
    let mut c = cmd(CMD_DATA, stream);
    c[0xc..0x10].copy_from_slice(&flags.to_le_bytes());
    c[0x18..0x1c].copy_from_slice(&step.to_le_bytes());
    c[0x1c..0x20].copy_from_slice(&target.to_le_bytes());
    c[0x20..0x24].copy_from_slice(&skip.to_le_bytes());
    c[0x24..0x28].copy_from_slice(&u32::MAX.to_le_bytes());
    if let Some(p) = pkt {
        c[0x28..0x2c].copy_from_slice(&(p.len() as u32).to_le_bytes());
        c[0x2c..0x34].copy_from_slice(&(p.as_ptr() as u64).to_le_bytes());
    }
    c[0x34..0x38].copy_from_slice(&((out.len() * 4) as u32).to_le_bytes());
    c[0x38..0x40].copy_from_slice(&(out.as_mut_ptr() as u64).to_le_bytes());
    c
}

fn settle(h: usize) {
    obj(h).unwrap().wait_idle();
}

fn result(h: usize) -> Option<[u8; RESULT_SIZE]> {
    settle(h);
    let mut r = [0u8; RESULT_SIZE];
    (XApuDequeueResult(h, r.as_mut_ptr()) == S_OK).then_some(r)
}

fn u32_at(r: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(r[o..o + 4].try_into().unwrap())
}

#[test]
fn connect_validation() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut region = [0u8; 16];
    let mut h = 0usize;
    let good = [2u32, 0x140, 0xE, 0x10000, 0x200, 4];
    assert_eq!(XApuConnect(good.as_ptr(), core::ptr::null_mut(), &mut h), E_POINTER);
    assert_eq!(XApuConnect(core::ptr::null(), region.as_mut_ptr(), &mut h), E_INVALIDARG);
    for (i, v) in [(0usize, 4u32), (1, 0), (1, 501), (2, 17), (4, 0x7f), (4, 0x201), (5, 0x10)] {
        let mut p = good;
        p[i] = v;
        assert_eq!(XApuConnect(p.as_ptr(), region.as_mut_ptr(), &mut h), E_INVALIDARG, "field {i}={v:#x}");
    }
    let mut big = good;
    big[3] = 0x2000_0001;
    assert_eq!(XApuConnect(big.as_ptr(), region.as_mut_ptr(), &mut h), E_OUTOFMEMORY);
    let mut c1 = good;
    c1[0] = 1;
    c1[4] = 0;
    assert_eq!(XApuConnect(c1.as_ptr(), region.as_mut_ptr(), &mut h), E_NOTIMPL);
    let mut c3 = good;
    c3[0] = 3;
    assert_eq!(XApuConnect(c3.as_ptr(), region.as_mut_ptr(), &mut h), E_NOTIMPL);
    assert_eq!(h, 0);
    let row = include_str!("../exports.txt").lines().find(|l| l.split_whitespace().nth(1) == Some("XApuConnect")).unwrap();
    assert_eq!(row.split_whitespace().nth(2), Some("stub"), "XApuConnect must stay labelled stub while processTypes 1 and 3 are blocked");
    assert_eq!(XApuConnect(good.as_ptr(), region.as_mut_ptr(), &mut h), S_OK);
    assert_ne!(u64::from_le_bytes(region[0..8].try_into().unwrap()), 0);
    assert_eq!(u64::from_le_bytes(region[8..16].try_into().unwrap()), 0);
    let mut util = [0u32; 2];
    assert_eq!(XApuGetUtilization(2, 1, util.as_mut_ptr()), S_OK);
    assert_eq!(util, [500 - 0x140, 500]);
    let mut too_many = good;
    too_many[1] = 0x100;
    let mut h0 = 0usize;
    assert_eq!(XApuConnect(too_many.as_ptr(), region.as_mut_ptr(), &mut h0), XAPU_E_NO_STREAMS);
    assert_eq!(h0, 0);
    let (h2, region2) = connect(4, OPT_RESULT_EVENT);
    assert_ne!(u64::from_le_bytes(region2[8..16].try_into().unwrap()), 0);
    assert_eq!(XApuDisconnect(h2), S_OK);
    let mut h1 = 0usize;
    assert_eq!(XApuConnect(good.as_ptr(), region.as_mut_ptr(), &mut h1), XAPU_E_NO_STREAMS);
    assert_eq!(XApuGetUtilization(4, 1, util.as_mut_ptr()), E_INVALIDARG);
    assert_eq!(XApuDisconnect(h), S_OK);
    assert_eq!(XApuGetUtilization(2, 1, util.as_mut_ptr()), S_OK);
    assert_eq!(util, [500, 500]);
}

#[test]
fn command_flow_and_stop_on_dequeue() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (h, _) = connect(4, 4);
    let mut seq = 0;
    let mut out = vec![0f32; FRAME];
    assert_eq!(XApuEnqueueCommand(h, data_cmd(0, None, 0, &mut out, 0.0, 1.0, 0).as_ptr(), &mut seq), XAPU_E_BAD_STREAM);
    assert_eq!(XApuEnqueueCommand(h, start_cmd(9, 1, 1.0).as_ptr(), &mut seq), XAPU_E_BAD_STREAM);
    assert_eq!(XApuEnqueueCommand(h, cmd(7, 0).as_ptr(), &mut seq), E_INVALIDARG);
    let mut small = start_cmd(2, 1, 1.0);
    small[0x2c..0x30].copy_from_slice(&(FRAME as u32 * 4 + 100).to_le_bytes());
    assert_eq!(XApuEnqueueCommand(h, small.as_ptr(), &mut seq), XAPU_E_START_BUFFER);
    assert_eq!(XApuEnqueueCommand(h, start_cmd(2, 1, 1.0).as_ptr(), &mut seq), S_OK);
    assert_eq!(seq, 1);
    assert_eq!(XApuEnqueueCommand(h, start_cmd(2, 1, 1.0).as_ptr(), &mut seq), XAPU_E_STREAM_ACTIVE);
    assert_eq!(XApuEnqueueCommand(h, data_cmd(2, None, 0, &mut out, 0.0, 1.0, DATA_FLAG_END).as_ptr(), &mut seq), S_OK);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 2).as_ptr(), &mut seq), S_OK);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_STOP, 2).as_ptr(), &mut seq), S_OK);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 2).as_ptr(), &mut seq), S_OK);
    assert_eq!(seq, 5);
    let mut dq = 0;
    settle(h);
    assert_eq!(XApuGetResultCount(h, &mut dq), 5);
    assert_eq!(dq, 0);
    let r = result(h).unwrap();
    assert_eq!((u32_at(&r, 0), u32_at(&r, 4), u32_at(&r, 8), u32_at(&r, 0x2c)), (1, 2, 0x1234, 0));
    let r = result(h).unwrap();
    assert_eq!((u32_at(&r, 0), u32_at(&r, 0xc), u32_at(&r, 0x2c), u32_at(&r, 0x30)), (3, DATA_FLAG_END, 0, 0));
    assert_eq!(u64::from_le_bytes(r[0x38..0x40].try_into().unwrap()), out.as_ptr() as u64);
    assert_eq!(u32_at(&result(h).unwrap(), 0), CMD_RESET);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 2).as_ptr(), &mut seq), S_OK);
    assert_eq!(u32_at(&result(h).unwrap(), 0), CMD_STOP);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 2).as_ptr(), &mut seq), XAPU_E_BAD_STREAM);
    assert!(result(h).is_some());
    assert!(result(h).is_some());
    let mut r = [0u8; RESULT_SIZE];
    assert_eq!(XApuDequeueResult(h, r.as_mut_ptr()), XAPU_E_NO_RESULT);
    assert_eq!(XApuGetResultCount(h, &mut dq), 0);
    assert_eq!(dq, 6);
    assert_eq!(XApuDisconnect(h), S_OK);
}

#[test]
fn per_stream_queue_depth() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (h, _) = connect(2, 4);
    let mut seq = 0;
    assert_eq!(XApuEnqueueCommand(h, start_cmd(0, 1, 1.0).as_ptr(), &mut seq), S_OK);
    assert_eq!(XApuEnqueueCommand(h, start_cmd(1, 1, 1.0).as_ptr(), &mut seq), S_OK);
    for _ in 1..0xE {
        assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 0).as_ptr(), &mut seq), S_OK);
    }
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 0).as_ptr(), &mut seq), XAPU_E_QUEUE_FULL);
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 1).as_ptr(), &mut seq), S_OK);
    assert!(result(h).is_some());
    assert_eq!(XApuEnqueueCommand(h, cmd(CMD_RESET, 0).as_ptr(), &mut seq), S_OK);
    while result(h).is_some() {}
    assert_eq!(XApuDisconnect(h), S_OK);
}

#[test]
fn gain_ramp_applies() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::PathBuf::from(r"D:\xbox-pc-port\emulator\third_party\opus-refs");
    let f = opusdec::ogg::parse_opus_file(&std::fs::read(dir.join("celtmono.opus")).expect("opus ref")).unwrap();
    let (h, _) = connect(1, 4);
    let mut seq = 0;
    assert_eq!(XApuEnqueueCommand(h, start_cmd(0, 1, 0.0).as_ptr(), &mut seq), S_OK);
    let mut out = vec![0f32; FRAME];
    let mut gains = Vec::new();
    for p in f.packets.iter().take(400) {
        assert_eq!(XApuEnqueueCommand(h, data_cmd(0, Some(p), 0, &mut out, 1.0 / FRAME as f32, 1.0, 0).as_ptr(), &mut seq), S_OK);
        let r = result(h).unwrap();
        assert_eq!(u32_at(&r, 0x2c), 0);
        gains.push(f32::from_bits(u32_at(&r, 0x40)));
    }
    assert!(gains[0] < 1.0);
    assert!(gains.windows(2).all(|w| w[1] >= w[0]));
    assert_eq!(*gains.last().unwrap(), 1.0);
    assert_eq!(XApuDisconnect(h), S_OK);
}

#[test]
fn opus_stream_decodes_like_libopus() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::PathBuf::from(r"D:\xbox-pc-port\emulator\third_party\opus-refs");
    for name in ["celt", "hybrid", "silk"] {
        let f = opusdec::ogg::parse_opus_file(&std::fs::read(dir.join(format!("{name}.opus"))).expect("opus ref")).unwrap();
        let reference: Vec<f32> = std::fs::read(dir.join(format!("{name}.f32"))).unwrap().chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
        let ch = f.channels;
        let (h, _) = connect(1, 4);
        let mut seq = 0;
        assert_eq!(XApuEnqueueCommand(h, start_cmd(0, ch as u32, 1.0).as_ptr(), &mut seq), S_OK);
        assert_eq!(u32_at(&result(h).unwrap(), 0x2c), 0);
        let mut out = vec![0f32; FRAME * ch];
        let mut got: Vec<f32> = Vec::new();
        let collect = |r: [u8; RESULT_SIZE], out: &[f32], got: &mut Vec<f32>| {
            assert_eq!(u32_at(&r, 0x2c), 0);
            let n = u32_at(&r, 0x30) as usize;
            assert_eq!(u32_at(&r, 0x34) as usize, n * ch * 4);
            for i in 0..n {
                for c in 0..ch {
                    got.push(out[c * FRAME + i]);
                }
            }
            n
        };
        for (k, p) in f.packets.iter().enumerate() {
            let skip = if k == 0 { f.pre_skip as u32 } else { 0 };
            assert_eq!(XApuEnqueueCommand(h, data_cmd(0, Some(p), skip, &mut out, 2.29e-5, 1.0, 0).as_ptr(), &mut seq), S_OK);
            let r = result(h).unwrap();
            collect(r, &out, &mut got);
        }
        loop {
            assert_eq!(XApuEnqueueCommand(h, data_cmd(0, None, 0, &mut out, 2.29e-5, 1.0, DATA_FLAG_END).as_ptr(), &mut seq), S_OK);
            let r = result(h).unwrap();
            if collect(r, &out, &mut got) == 0 {
                break;
            }
        }
        let n = got.len().min(reference.len());
        assert!(n * 100 >= reference.len() * 99, "{name}: {} vs {}", got.len(), reference.len());
        let q = |v: f32| (v * 32768.0).round().clamp(-32768.0, 32767.0) as i32;
        let max = (0..n).map(|i| (q(got[i]) - q(reference[i])).abs()).max().unwrap();
        eprintln!("xapu {name}: {n} samples, max diff {max} LSB");
        assert!(max <= 1, "{name}: {max}");
        assert_eq!(XApuEnqueueCommand(h, cmd(CMD_STOP, 0).as_ptr(), &mut seq), S_OK);
        assert!(result(h).is_some());
        assert_eq!(XApuDisconnect(h), S_OK);
    }
}

#[test]
fn garbage_packets_report_errors_without_panicking() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (h, _) = connect(1, 4);
    let mut seq = 0;
    assert_eq!(XApuEnqueueCommand(h, start_cmd(0, 2, 1.0).as_ptr(), &mut seq), S_OK);
    result(h);
    let mut out = vec![0f32; FRAME * 2];
    let mut x = 0x9e37_79b9u32;
    for k in 0..3000 {
        let len = (k % 300) + 1;
        let pkt: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        assert_eq!(XApuEnqueueCommand(h, data_cmd(0, Some(&pkt), (x % 2000) as u32, &mut out, f32::NAN, f32::INFINITY, 0).as_ptr(), &mut seq), S_OK);
        let r = result(h).unwrap();
        let st = u32_at(&r, 0x2c);
        assert!(st == 0 || st == E_FAIL as u32);
        assert!(u32_at(&r, 0x30) as usize <= FRAME);
    }
    assert_eq!(XApuDisconnect(h), S_OK);
}

#[link(name = "kernel32")]
extern "system" {
    fn WaitForSingleObject(h: usize, ms: u32) -> u32;
}

#[test]
fn async_results_commit_in_submission_order() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let dir = std::path::PathBuf::from(r"D:\xbox-pc-port\emulator\third_party\opus-refs");
    let f = opusdec::ogg::parse_opus_file(&std::fs::read(dir.join("hybrid.opus")).expect("opus ref")).unwrap();
    let ch = f.channels;
    const S: usize = 6;
    let run = |sync: bool| -> (Vec<[u8; RESULT_SIZE]>, Vec<Vec<f32>>) {
        SYNC.store(sync, Ordering::SeqCst);
        let (h, region) = connect(S as u32, 4 | OPT_RESULT_EVENT);
        let ev = u64::from_le_bytes(region[8..16].try_into().unwrap()) as usize;
        let mut seq = 0;
        let mut last = 0;
        for s in 0..S {
            assert_eq!(XApuEnqueueCommand(h, start_cmd(s as u32, ch as u32, 1.0).as_ptr(), &mut seq), S_OK);
            assert_eq!(seq, last + 1);
            last = seq;
        }
        let mut outs: Vec<Vec<f32>> = (0..S * 8).map(|_| vec![0f32; FRAME * ch]).collect();
        let mut results = Vec::new();
        let mut pcm: Vec<Vec<f32>> = vec![Vec::new(); S];
        let mut k = 0;
        for chunk in f.packets.chunks(8).take(40) {
            for (i, p) in chunk.iter().enumerate() {
                for s in 0..S {
                    let o = &mut outs[(i * S + s) % (S * 8)];
                    let c = data_cmd(s as u32, Some(p), 0, o, 0.0, 1.0, 0);
                    assert_eq!(XApuEnqueueCommand(h, c.as_ptr(), &mut seq), S_OK);
                    assert_eq!(seq, last + 1);
                    last = seq;
                }
            }
            let want = (k == 0) as usize * S + chunk.len() * S;
            let mut got = 0;
            let t0 = std::time::Instant::now();
            while got < want {
                if XApuGetResultCount(h, core::ptr::null_mut()) == 0 {
                    unsafe { WaitForSingleObject(ev, 50) };
                    assert!(t0.elapsed().as_secs() < 10, "results never arrived");
                    continue;
                }
                let mut r = [0u8; RESULT_SIZE];
                assert_eq!(XApuDequeueResult(h, r.as_mut_ptr()), S_OK);
                if u32_at(&r, 0) == CMD_DATA {
                    let s = u32_at(&r, 4) as usize;
                    let n = u32_at(&r, 0x30) as usize;
                    let ptr = u64::from_le_bytes(r[0x38..0x40].try_into().unwrap()) as *const f32;
                    let o = unsafe { core::slice::from_raw_parts(ptr, FRAME * ch) };
                    for i in 0..n {
                        for c in 0..ch {
                            pcm[s].push(o[c * FRAME + i]);
                        }
                    }
                }
                results.push(r);
                got += 1;
            }
            k += 1;
        }
        assert_eq!(XApuDisconnect(h), S_OK);
        SYNC.store(false, Ordering::SeqCst);
        (results, pcm)
    };
    let (ra, pa) = run(false);
    let (rs, ps) = run(true);
    assert_eq!(ra.len(), rs.len());
    for (a, b) in ra.iter().zip(&rs) {
        assert_eq!(a[0..0x18], b[0..0x18]);
        assert_eq!(a[0x2c..0x38], b[0x2c..0x38]);
        assert_eq!(a[0x40..0x44], b[0x40..0x44]);
    }
    for s in 1..S {
        assert_eq!(pa[s], pa[0]);
    }
    assert_eq!(pa, ps);
    assert!(pa[0].len() > FRAME * ch * 100);
}
