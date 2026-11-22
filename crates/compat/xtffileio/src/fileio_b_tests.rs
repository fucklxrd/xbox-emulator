use super::{diff, xfer, E_UNEXPECTED};
use crate::file::FileInfo;

#[test]
fn transfer_leader_preserves_wire_layout() {
    let info = FileInfo {
        attributes: 0x20,
        creation: 0x0102_0304_0506_0708,
        access: 0x11,
        write: 0x22,
        size: 0,
    };
    let name: Vec<u16> = "D:\\é".encode_utf16().collect();
    let leader = xfer::leader_bytes(xfer::DIRECT_PROTOCOL_BLOCK_SIZE, 0x1_0000_0000, &info, &name);
    assert_eq!(leader.len(), xfer::LEADER_SIZE + name.len() * 2 + 2);
    assert_eq!(
        leader,
        [
            0, 0, 8, 0, 0, 0, 0, 0,
            0, 0, 0, 0, 1, 0, 0, 0,
            0x20, 0, 0, 0, 0, 0, 0, 0,
            8, 7, 6, 5, 4, 3, 2, 1,
            0x11, 0, 0, 0, 0, 0, 0, 0,
            0x22, 0, 0, 0, 0, 0, 0, 0,
            b'D', 0, b':', 0, b'\\', 0, 0xe9, 0, 0, 0,
        ]
    );
}

#[test]
fn transfer_path_matches_the_root_case_insensitively() {
    assert_eq!(xfer::relative_file_name("D:\\Games", "d:\\games\\Data\\file.bin").as_deref(), Ok("Data\\file.bin"));
    assert_eq!(xfer::relative_file_name("D:\\Games", "D:\\Games\\\\file.bin").as_deref(), Ok("file.bin"));
    assert_eq!(xfer::relative_file_name("D:\\Games", "E:\\Games\\file.bin"), Err(E_UNEXPECTED));
    assert_eq!(xfer::relative_file_name("D:\\Games", "D:\\Gam"), Err(E_UNEXPECTED));
}

#[test]
fn md5_matches_rfc_1321_vectors() {
    let hex = |d: [u8; 16]| d.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(hex(diff::md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(hex(diff::md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(hex(diff::md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
    assert_eq!(hex(diff::md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")), "57edf4a22be3c955ac49da2e2107b67a");
}

#[test]
fn delta_keys_hash_normalized_utf16le_paths() {
    let path: Vec<u16> = "D:\\\\DATA\\É.BIN".encode_utf16().collect();
    let normalized: Vec<u16> = "d:\\data\\é.bin".encode_utf16().collect();
    assert_eq!(diff::normalize_key(&path), normalized);
    assert_eq!(diff::hash_key(&path), [0x1b, 0xfe, 0x94, 0x3d, 0x20, 0x7e, 0x76, 0x0c, 0xc5, 0x9b, 0x56, 0xe4, 0x24, 0xb2, 0x0f, 0x62]);
}

#[test]
fn delta_entry_round_trips_its_32_byte_record() {
    let e = diff::DeltaEntry { hash: [0xab; 16], time: 0x1122_3344_5566_7788, less: 7, greater: 0xffff_ffff };
    let b = e.to_bytes();
    assert_eq!(&b[16..24], &0x1122_3344_5566_7788u64.to_le_bytes());
    assert_eq!(diff::DeltaEntry::from_bytes(&b), e);
}
