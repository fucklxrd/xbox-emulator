mod common_process_debug;
use common_process_debug::*;

#[test]
fn console_build_19041_uses_the_19041_debug_flag_mask() {
    std::env::set_var("TOMBOX_CONSOLE_OSBUILD", "19041");
    let d = load("b19041");
    assert_eq!(d.set(0xffff_ffff), 0);
    assert_eq!(d.get(), 0x81f0_0001);
    assert_eq!(d.set(0x0200_0001), 0);
    assert_eq!(d.get(), 0x0000_0001);
    d.set(0);
}
