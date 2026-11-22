use super::*;

pub(crate) static LOCK: Mutex<()> = Mutex::new(());
static INITIALIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub(crate) unsafe fn init_hix(log: usize) -> bool {
    assert_eq!(InitializeHix(log), 0);
    !INITIALIZED.swap(true, Ordering::SeqCst)
}

#[test]
fn save_developer_blob_info_fails_without_kmd_region_and_fills_64_slots() {
    let _g = LOCK.lock().unwrap();
    unsafe {
        set_region1(core::ptr::null_mut());
        assert_eq!(SaveDeveloperBlobInfo(0x1234, 8), E_FAIL);
        let mut region = vec![0u8; REGION1_SIZE];
        set_region1(region.as_mut_ptr());
        for i in 0..BLOB_SLOTS {
            assert_eq!(SaveDeveloperBlobInfo(0x1000 + i as u64, i * 2), 0);
        }
        assert_eq!(SaveDeveloperBlobInfo(0xdead, 1), 1);
        set_region1(core::ptr::null_mut());
        let r = region.as_ptr();
        assert_eq!((r.add(BLOB_COUNT) as *const u32).read_unaligned(), 64);
        assert_eq!((r.add(BLOB_PTRS + 5 * 8) as *const u64).read_unaligned(), 0x1005);
        assert_eq!((r.add(BLOB_SIZES + 5 * 4) as *const u32).read_unaligned(), 10);
    }
}

#[test]
fn add_streaming_api_object_copies_0x2dc_bytes() {
    let _g = LOCK.lock().unwrap();
    let before = streaming_objects().len();
    let mut obj = [0u8; STREAMING_OBJECT_SIZE + 4];
    for (i, b) in obj.iter_mut().enumerate() {
        *b = i as u8;
    }
    assert_eq!(unsafe { AddStreamingApiObject(obj.as_ptr()) }, 0);
    let all = streaming_objects();
    assert_eq!(all.len(), before + 1);
    let last = all.last().unwrap();
    assert_eq!(last[0], 0);
    assert_eq!(last[STREAMING_OBJECT_SIZE - 1], (STREAMING_OBJECT_SIZE - 1) as u8);
}

#[test]
fn add_data_assigns_sequential_ids_and_reuses_keyed_ids() {
    let _g = LOCK.lock().unwrap();
    unsafe {
        let blob = [1u8, 2, 3];
        let start = next_data_id();
        let mut a = 0u32;
        assert_eq!(AddData(core::ptr::null(), blob.as_ptr(), 3, &mut a), 0);
        assert_eq!(a, start);
        let key = 0xfeed_beef_u64 ^ start as u64;
        let mut b = 0u32;
        assert_eq!(AddData(&key, blob.as_ptr(), 3, &mut b), 0);
        assert_eq!(b, start + 1);
        let mut c = 0u32;
        assert_eq!(AddData(&key, blob.as_ptr(), 3, &mut c), 0);
        assert_eq!(c, b);
        assert_eq!(next_data_id(), start + 2);
        let mut d = 0u32;
        assert_eq!(AddData(core::ptr::null(), blob.as_ptr(), 3, &mut d), 0);
        assert_eq!(d, start + 2);
    }
}
