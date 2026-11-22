#[allow(unused_imports)]
pub use xgs_kmd_emu::{gpu_frequency_hz, MHZ};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console_slots_dev2::{console_info, family, kmd_info, kmd_reply, Family};

    fn u64_at(b: &[u8], o: usize) -> u64 {
        u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
    }

    fn u32_at(b: &[u8], o: usize) -> u32 {
        u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
    }

    #[test]
    fn frequency_per_console() {
        let f: Vec<u64> = (2..=8).map(gpu_frequency_hz).collect();
        assert_eq!(f, vec![853_333_333, 914_285_714, 1_172_000_000, 1_172_000_000, 1_563_000_000, 1_825_000_000, 1_825_000_000]);
        assert_eq!(gpu_frequency_hz(0), 1_825_000_000);
        assert_eq!(gpu_frequency_hz(1), 1_825_000_000);
    }

    #[test]
    fn kmd_reply_carries_frequency() {
        for t in 0..=9 {
            assert_eq!(kmd_info(t).word0, gpu_frequency_hz(t));
            assert_eq!(u64_at(&kmd_reply(t), 0), gpu_frequency_hz(t));
        }
    }

    #[test]
    fn hardware_configuration_struct_per_console() {
        let expect: [(u32, u64, u32, u32); 7] = [
            (2, 853_333_333, 0, 12),
            (3, 914_285_714, 1, 12),
            (4, 1_172_000_000, 2, 40),
            (5, 1_172_000_000, 3, 44),
            (6, 1_563_000_000, 4, 0),
            (7, 1_825_000_000, 5, 0),
            (8, 1_825_000_000, 6, 0),
        ];
        for (t, freq, version, cus) in expect {
            let i = console_info(family(t), t);
            assert_eq!(u64_at(&i, 0), freq, "device type {t}");
            assert_eq!(u32_at(&i, 8), version, "device type {t}");
            assert_eq!(u32_at(&i, 12), cus, "device type {t}");
        }
    }

    #[test]
    fn slot68_copies_reply_frequency_and_scarlett_drops_cu_count() {
        for t in 0..=9 {
            let r = kmd_reply(t);
            let i = console_info(family(t), t);
            assert_eq!(&i[0..8], &r[0..8]);
            if family(t) == Family::XboxOne {
                assert_eq!(&i[12..16], &r[12..16]);
            } else {
                assert_eq!(&i[12..16], &[0; 4]);
            }
        }
    }
}
