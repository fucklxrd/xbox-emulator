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

    assert_eq!(
        leader,
        [
            0, 0, 8, 0, 0, 0, 0, 0, // Block size
            0, 0, 0, 0, 1, 0, 0, 0, // File size
            0x20, 0, 0, 0, 0, 0, 0, 0, // Attributes and padding
            8, 7, 6, 5, 4, 3, 2, 1, // Creation time
            0x11, 0, 0, 0, 0, 0, 0, 0, // Access time
            0x22, 0, 0, 0, 0, 0, 0, 0, // Write time
            b'D', 0, b':', 0, b'\\', 0, 0xe9, 0, 0, 0, // UTF-16LE path and terminator
        ]
    );
}

#[test]
fn transfer_path_matches_the_root_case_insensitively() {
    assert_eq!(
        xfer::relative_file_name("D:\\Games", "d:\\games\\Data\\file.bin").as_deref(),
        Ok("Data\\file.bin")
    );
    assert_eq!(
        xfer::relative_file_name("D:\\Games", "E:\\Games\\file.bin"),
        Err(E_UNEXPECTED)
    );
}

#[test]
fn delta_keys_hash_normalized_utf16le_paths() {
    let path: Vec<u16> = "D:\\\\DATA\\É.BIN".encode_utf16().collect();
    let normalized: Vec<u16> = "d:\\data\\é.bin".encode_utf16().collect();

    assert_eq!(diff::normalize_key(&path), normalized);
    assert_eq!(
        diff::hash_key(&path),
        [0x1b, 0xfe, 0x94, 0x3d, 0x20, 0x7e, 0x76, 0x0c, 0xc5, 0x9b, 0x56, 0xe4, 0x24, 0xb2, 0x0f, 0x62]
    );
}
