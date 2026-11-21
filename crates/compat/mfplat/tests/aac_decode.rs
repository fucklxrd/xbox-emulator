use core::mem::ManuallyDrop;
use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

#[path = "common/media.rs"]
mod media;
const CLSID_MSAAC_DECODER: GUID = GUID::from_u128(0x32d186a7_218f_4c75_8876_dd77273a8999);
const MF_E_NOTACCEPTING: i32 = 0xC00D_36B5u32 as i32;
const MF_E_TRANSFORM_NEED_MORE_INPUT: i32 = 0xC00D_6D72u32 as i32;
const MF_E_TRANSFORM_STREAM_CHANGE: i32 = 0xC00D_6D61u32 as i32;

fn be32(b: &[u8], o: usize) -> usize {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap()) as usize
}

fn boxes(b: &[u8], start: usize, end: usize) -> Vec<(String, usize, usize)> {
    let mut out = Vec::new();
    let mut o = start;
    while o + 8 <= end {
        let size = be32(b, o);
        let ty = String::from_utf8_lossy(&b[o + 4..o + 8]).into_owned();
        if size < 8 || o + size > end {
            break;
        }
        out.push((ty, o, size));
        o += size;
    }
    out
}

fn find(b: &[u8], path: &[&str]) -> Option<(usize, usize)> {
    let (mut s, mut e) = (0, b.len());
    for (i, name) in path.iter().enumerate() {
        let skip = match *name {
            "stsd" => 8,
            "mp4a" => 28,
            "esds" => 4,
            _ => 0,
        };
        let (_, o, size) = boxes(b, s, e).into_iter().find(|(t, _, _)| t == name)?;
        if i + 1 == path.len() {
            return Some((o, size));
        }
        s = o + 8 + skip;
        e = o + size;
        if *name == "stsd" {
            s = o + 16;
        }
    }
    None
}

fn audio_specific_config(clip: &[u8]) -> Vec<u8> {
    let (o, size) = find(clip, &["moov", "trak", "mdia", "minf", "stbl", "stsd", "mp4a", "esds"]).expect("esds");
    let d = &clip[o + 12..o + size];
    let mut i = 0;
    while i < d.len() {
        let tag = d[i];
        i += 1;
        let mut len = 0usize;
        loop {
            let c = d[i];
            i += 1;
            len = (len << 7) | (c & 0x7f) as usize;
            if c & 0x80 == 0 {
                break;
            }
        }
        match tag {
            0x03 => i += 3,
            0x04 => i += 13,
            0x05 => return d[i..i + len].to_vec(),
            _ => i += len,
        }
    }
    panic!("no DecoderSpecificInfo in esds");
}

fn access_units(clip: &[u8]) -> Vec<Vec<u8>> {
    let (moof, _) = find(clip, &["moof"]).expect("moof");
    let (tfhd, _) = find(clip, &["moof", "traf", "tfhd"]).expect("tfhd");
    let (trun, _) = find(clip, &["moof", "traf", "trun"]).expect("trun");
    let tf = be32(clip, tfhd + 8) & 0xff_ffff;
    let mut p = tfhd + 16;
    for bit in [0x1usize, 0x2, 0x8] {
        if tf & bit != 0 {
            p += if bit == 0x1 { 8 } else { 4 };
        }
    }
    let default_size = if tf & 0x10 != 0 { be32(clip, p) } else { 0 };
    let flags = be32(clip, trun + 8) & 0xff_ffff;
    let count = be32(clip, trun + 12);
    let mut q = trun + 16;
    let mut data = moof;
    if flags & 0x1 != 0 {
        data = moof + be32(clip, q);
        q += 4;
    }
    if flags & 0x4 != 0 {
        q += 4;
    }
    let mut out = Vec::new();
    for _ in 0..count {
        if flags & 0x100 != 0 {
            q += 4;
        }
        let size = if flags & 0x200 != 0 {
            let s = be32(clip, q);
            q += 4;
            s
        } else {
            default_size
        };
        if flags & 0x400 != 0 {
            q += 4;
        }
        if flags & 0x800 != 0 {
            q += 4;
        }
        out.push(clip[data..data + size].to_vec());
        data += size;
    }
    out
}

fn sample(bytes: &[u8], t: i64) -> IMFSample {
    unsafe {
        let buf = MFCreateMemoryBuffer(bytes.len() as u32).unwrap();
        let mut p: *mut u8 = core::ptr::null_mut();
        buf.Lock(&mut p, None, None).unwrap();
        core::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        buf.Unlock().unwrap();
        buf.SetCurrentLength(bytes.len() as u32).unwrap();
        let s = MFCreateSample().unwrap();
        s.AddBuffer(&buf).unwrap();
        s.SetSampleTime(t).unwrap();
        s.SetSampleDuration(213_333).unwrap();
        s
    }
}

fn pull(t: &IMFTransform, pcm: &mut Vec<u8>) -> bool {
    loop {
        let info = unsafe { t.GetOutputStreamInfo(0) }.unwrap();
        let provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
        let pre = (!provides).then(|| unsafe {
            let b = MFCreateMemoryBuffer(info.cbSize.max(65536)).unwrap();
            let s = MFCreateSample().unwrap();
            s.AddBuffer(&b).unwrap();
            s
        });
        let mut out = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: ManuallyDrop::new(pre), dwStatus: 0, pEvents: ManuallyDrop::new(None) }];
        let mut status = 0u32;
        let hr = unsafe { t.ProcessOutput(0, &mut out, &mut status) };
        let s = unsafe { ManuallyDrop::take(&mut out[0].pSample) };
        let _ = unsafe { ManuallyDrop::take(&mut out[0].pEvents) };
        match hr {
            Ok(()) => {
                if let Some(s) = s {
                    let b = unsafe { s.ConvertToContiguousBuffer() }.unwrap();
                    let mut p: *mut u8 = core::ptr::null_mut();
                    let mut len = 0u32;
                    unsafe { b.Lock(&mut p, None, Some(&mut len)) }.unwrap();
                    pcm.extend_from_slice(unsafe { core::slice::from_raw_parts(p, len as usize) });
                    unsafe { b.Unlock() }.unwrap();
                }
            }
            Err(e) if e.code().0 == MF_E_TRANSFORM_NEED_MORE_INPUT => return true,
            Err(e) if e.code().0 == MF_E_TRANSFORM_STREAM_CHANGE => {
                let ty = unsafe { t.GetOutputAvailableType(0, 0) }.unwrap();
                unsafe { t.SetOutputType(0, &ty, 0) }.unwrap();
            }
            Err(e) => panic!("ProcessOutput {e:?}"),
        }
    }
}

#[test]
fn served_aac_rendition_decodes_like_electra() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(0x0002_0070, MFSTARTUP_FULL).unwrap();
    }
    let clip = media::staged_fragment("audio_init.mp4", "audio_seg0.m4s")
        .unwrap_or_else(media::silent_aac);
    let asc = audio_specific_config(&clip);
    assert_eq!(asc[..2], [0x11, 0x90], "AAC-LC 48 kHz stereo AudioSpecificConfig");
    let aus = access_units(&clip);
    assert!(aus.len() >= 40, "{} access units", aus.len());
    let t: IMFTransform = unsafe { CoCreateInstance(&CLSID_MSAAC_DECODER, None, CLSCTX_INPROC_SERVER) }.expect("CMSAACDecMFT");
    let mt = unsafe { MFCreateMediaType() }.unwrap();
    let mut user = vec![0u8; 12];
    user.extend_from_slice(&asc);
    unsafe {
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio).unwrap();
        mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).unwrap();
        mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).unwrap();
        mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48000).unwrap();
        mt.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).unwrap();
        mt.SetBlob(&MF_MT_USER_DATA, &user).unwrap();
        t.SetInputType(0, &mt, 0).expect("SetInputType raw AAC");
    }
    let mut chosen = None;
    for i in 0.. {
        let Ok(ty) = (unsafe { t.GetOutputAvailableType(0, i) }) else { break };
        let sub = unsafe { ty.GetGUID(&MF_MT_SUBTYPE) }.unwrap();
        let bits = unsafe { ty.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE) }.unwrap_or(0);
        if sub == MFAudioFormat_PCM && bits == 16 {
            unsafe { t.SetOutputType(0, &ty, 0) }.unwrap();
            chosen = Some(ty);
            break;
        }
    }
    assert!(chosen.is_some(), "16-bit PCM output type");
    unsafe {
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).unwrap();
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).unwrap();
    }
    let mut pcm = Vec::new();
    for (i, au) in aus.iter().enumerate() {
        let s = sample(au, i as i64 * 213_333);
        loop {
            match unsafe { t.ProcessInput(0, &s, 0) } {
                Ok(()) => break,
                Err(e) if e.code().0 == MF_E_NOTACCEPTING => {
                    pull(&t, &mut pcm);
                }
                Err(e) => panic!("ProcessInput AU {i}: {e:?}"),
            }
        }
        pull(&t, &mut pcm);
    }
    unsafe {
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0).unwrap();
        t.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0).unwrap();
    }
    pull(&t, &mut pcm);
    let frames = pcm.len() / 4;
    let peak = pcm.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]).unsigned_abs()).max().unwrap_or(0);
    eprintln!("aac: {} access units -> {frames} stereo PCM frames ({:.3} s at 48 kHz), peak {peak}", aus.len(), frames as f64 / 48000.0);
    assert!(frames >= 46_000, "decoded only {frames} frames");
    assert!(peak <= 4, "silent track decoded with peak {peak}");
}
