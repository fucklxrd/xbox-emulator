mod common_process_debug;
use common_process_debug::*;

#[test]
fn debug_flag_selects_debug_driver_and_requests_pc_debug_layer() {
    let d = load("on");
    assert_eq!(d.set(0xffff_ffff), 0);
    assert_eq!(d.get(), 0x83f0_0001);
    assert_eq!(d.set(0x0000_0001), 0);
    assert_eq!(d.get(), 1);
    let (hr, dev) = d.create(0x0000_0001);
    assert_eq!(hr, 0, "D3D12XboxCreateDevice");
    assert!(!dev.is_null());
    assert_eq!(d.get(), 1);
    let log = d.log_text();
    assert!(log.contains("ProcessDebugFlags=0x00000001 console UMD=xgs12_d.dll (Debug) pc debug layer requested=true enable_now=true"), "{log}");
    if sdk_layers_present() {
        assert!(log.contains("D3D12 debug layer ENABLED"), "{log}");
    } else {
        assert!(log.contains("D3D12GetDebugInterface hr="), "{log}");
        eprintln!("d3d12SDKLayers.dll not installed: debug layer request reached D3D12GetDebugInterface, which failed; layer activation itself is unproven on this PC");
    }
    let (hr2, _) = d.create(0x0000_0001);
    assert_eq!(hr2, 0);
    assert!(d.log_text().contains("enable_now=false device_already_created=true"));
}
