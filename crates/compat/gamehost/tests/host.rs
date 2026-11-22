use gamehost::host::*;
use gamehost::wire::*;
use gamehost::xcrd::Xcrd;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Log {
    calls: Vec<String>,
    open: Vec<Result<usize, i32>>,
    mount: Vec<Result<usize, i32>>,
    query: Vec<Result<Vec<u16>, i32>>,
    unmount: Vec<i32>,
    close: Vec<i32>,
}

struct Mock(Arc<Mutex<Log>>);

impl Xcrd for Mock {
    fn open_adapter(&mut self) -> Result<usize, i32> {
        let mut l = self.0.lock().unwrap();
        l.calls.push("open".into());
        if l.open.is_empty() { Ok(0xa0) } else { l.open.remove(0) }
    }
    fn close_adapter(&mut self, a: usize) -> i32 {
        let mut l = self.0.lock().unwrap();
        l.calls.push(format!("close {a:#x}"));
        if l.close.is_empty() { 0 } else { l.close.remove(0) }
    }
    fn mount_content_type(&mut self, a: usize, path: &[u16], ct: u32, flags: u32) -> Result<usize, i32> {
        let mut l = self.0.lock().unwrap();
        l.calls.push(format!("mount {a:#x} {} {ct:#x} {flags}", String::from_utf16_lossy(path)));
        if l.mount.is_empty() { Ok(0xd0) } else { l.mount.remove(0) }
    }
    fn query_device_path(&mut self, d: usize) -> Result<Vec<u16>, i32> {
        let mut l = self.0.lock().unwrap();
        l.calls.push(format!("query {d:#x}"));
        if l.query.is_empty() { Ok(w("\\Device\\Xvd1")) } else { l.query.remove(0) }
    }
    fn unmount(&mut self, a: usize, d: usize) -> i32 {
        let mut l = self.0.lock().unwrap();
        l.calls.push(format!("unmount {a:#x} {d:#x}"));
        if l.unmount.is_empty() { 0 } else { l.unmount.remove(0) }
    }
}

fn host(build: u32, cap: bool) -> (GameHost, Arc<Mutex<Log>>) {
    let l = Arc::new(Mutex::new(Log::default()));
    (GameHost::new(build, cap, Box::new(Mock(l.clone()))), l)
}

fn config(reserved: u64, flags: u64) -> [u8; 16] {
    let mut c = [0u8; 16];
    c[..8].copy_from_slice(&reserved.to_le_bytes());
    c[8..].copy_from_slice(&flags.to_le_bytes());
    c
}

fn st() -> SystemTime {
    SystemTime { year: 2026, month: 11, day_of_week: 6, day: 21, hour: 9, minute: 5, second: 7, milliseconds: 42 }
}

fn reply_info(m: &Message) -> CrashDumpInfo {
    assert_eq!(m.id, MSG_CRASH_DUMP_INFO);
    assert_eq!(m.payload.len(), 0x218);
    CrashDumpInfo::decode(&m.payload)
}

#[test]
fn crash_dump_info_default_reply_is_zeroed_0x218() {
    let (mut h, _) = host(22621, true);
    let m = h.dispatch(1, &Message::new(MSG_CRASH_DUMP_INFO, Vec::new())).unwrap().unwrap();
    assert_eq!(m.payload, vec![0u8; 0x218]);
    assert!(h.dump_requested);
}

#[test]
fn crash_dump_info_copies_launch_config_bytes() {
    let (mut h, _) = host(22621, true);
    h.set_launch(&w("Ident"), config(0x1122334455667788, 1));
    let m = h.crash_dump_info(1);
    assert_eq!(&m.payload[..8], &0x1122334455667788u64.to_le_bytes());
    assert_eq!(&m.payload[8..16], &1u64.to_le_bytes());
    assert!(m.payload[16..].iter().all(|&b| b == 0));
}

#[test]
fn hang_file_path_22621_capability_selects_local_dumps() {
    let (mut h, _) = host(22621, true);
    h.set_launch(&w("Ident"), config(0, 2));
    h.record_quiesce(Some(&w("C:\\Game\\Trackmania.exe")), st());
    let i = reply_info(&h.crash_dump_info(1));
    assert_eq!(String::from_utf16_lossy(&i.file), "D:\\LocalDumps\\Quiesce-Trackmania.exe-2026-11-21-9-5-7-42.etl");
    assert_eq!(&h.crash_dump_info(2).payload[0x10..0x12], &(b'D' as u16).to_le_bytes());
}

#[test]
fn hang_file_path_22621_without_capability_uses_s_temp() {
    let (mut h, _) = host(22621, false);
    h.set_launch(&w("Ident"), config(0, 3));
    let i = reply_info(&h.crash_dump_info(1));
    assert_eq!(String::from_utf16_lossy(&i.file), "s:\\temp\\.etl");
    assert_eq!(i.flags(), 3);
}

#[test]
fn hang_file_path_22000_is_fixed_local_dumps() {
    let (mut h, _) = host(22000, false);
    h.set_launch(&w("Ident"), config(0, 2));
    h.record_quiesce(None, st());
    let i = reply_info(&h.crash_dump_info(1));
    assert_eq!(String::from_utf16_lossy(&i.file), "D:\\LocalDumps\\Quiesce-CurrentActiveProcess-2026-11-21-9-5-7-42.etl");
}

#[test]
fn flag_bit1_clear_leaves_path_empty() {
    let (mut h, _) = host(22621, true);
    h.set_launch(&w("Ident"), config(0, 1));
    h.record_quiesce(None, st());
    assert!(reply_info(&h.crash_dump_info(1)).file.is_empty());
}

#[test]
fn quiesce_name_basename_and_reset_on_launch() {
    let (mut h, _) = host(22621, true);
    assert_eq!(h.record_quiesce(Some(&w("game.exe")), st()), 0);
    assert_eq!(String::from_utf16_lossy(&h.quiesce_name), "Quiesce-game.exe-2026-11-21-9-5-7-42");
    h.set_launch(&w("Ident"), [0; 16]);
    assert!(h.quiesce_name.is_empty());
}

#[test]
fn long_paths_truncate_like_string_cch_printf() {
    let (mut h, _) = host(22621, true);
    let long = w(&format!("C:\\{}", "a".repeat(400)));
    assert_eq!(h.record_quiesce(Some(&long), st()), HR_INSUFFICIENT_BUFFER);
    assert_eq!(h.quiesce_name.len(), 0x103);
    h.set_launch(&w("I"), config(0, 2));
    h.record_quiesce(Some(&long), st());
    let i = reply_info(&h.crash_dump_info(1));
    assert_eq!(i.file.len(), 0x103);
    assert!(h.errors.contains(&(0xd72, HR_INSUFFICIENT_BUFFER)));
}

#[test]
fn first_request_initiates_and_last_disconnect_completes() {
    let (mut h, _) = host(22621, true);
    h.crash_dump_info(7);
    assert_eq!(h.dumps_in_flight, 1);
    assert_eq!(h.wnf_crash_dump_state, 1);
    assert_eq!(h.debug, vec!["GameHostService: Crash dump initiated.\n".to_string()]);
    h.crash_dump_info(8);
    assert_eq!(h.debug.len(), 1);
    h.disconnect(7);
    assert_eq!(h.wnf_crash_dump_state, 1);
    h.disconnect(8);
    assert_eq!(h.wnf_crash_dump_state, 0);
    assert_eq!(h.debug.last().unwrap(), "GameHostService: Crash dump completed.\n");
    let c = h.controller.as_ref().unwrap();
    assert_eq!(c, &vec![ControllerMessage { id: CTL_DUMP_IN_PROGRESS, payload: vec![1] }, ControllerMessage { id: CTL_DUMP_IN_PROGRESS, payload: vec![0] }]);
}

#[test]
fn two_requests_one_client_one_disconnect_stays_in_flight() {
    let (mut h, _) = host(22621, true);
    h.crash_dump_info(3);
    h.crash_dump_info(3);
    h.disconnect(3);
    assert_eq!(h.dumps_in_flight, 1);
    assert_eq!(h.dump_clients, vec![3]);
    h.disconnect(9);
    assert_eq!(h.dumps_in_flight, 1);
}

#[test]
fn controller_absent_logs_but_still_replies() {
    let (mut h, _) = host(22621, true);
    h.controller = None;
    let m = h.crash_dump_info(1);
    assert_eq!(m.payload.len(), 0x218);
    assert!(h.errors.contains(&(0xd88, HR_NO_CONNECTION)));
}

#[test]
fn progress_events_forward_to_controller() {
    let (mut h, _) = host(22621, true);
    for k in 0..3u64 {
        let r = h.dispatch(1, &Message::new(MSG_CRASH_DUMP_PROGRESS, k.to_le_bytes().to_vec())).unwrap();
        assert!(r.is_none());
    }
    assert_eq!(h.debug, PROGRESS_NAMES.iter().map(|n| format!("GameHostService: Crash dump Progress -> {n}\n")).collect::<Vec<_>>());
    let c = h.controller.as_ref().unwrap();
    assert_eq!(c[2], ControllerMessage { id: CTL_DUMP_PROGRESS, payload: 2u64.to_le_bytes().to_vec() });
    assert_eq!(h.dumps_in_flight, 0);
}

#[test]
fn progress_without_controller_is_logged_only() {
    let (mut h, _) = host(22621, true);
    h.controller = None;
    assert_eq!(h.crash_dump_progress(1), Ok(HR_NO_CONNECTION));
    assert!(h.errors.contains(&(0xdc0, HR_NO_CONNECTION)));
}

#[test]
fn progress_index_outside_console_table() {
    let (mut h, _) = host(22621, true);
    assert_eq!(h.crash_dump_progress(3), Err(Fault::OutOfTable(3)));
}

#[test]
fn progress_message_unknown_to_22000_host_fails_fast() {
    let (mut h, _) = host(22000, true);
    assert_eq!(h.dispatch(1, &Message::new(MSG_CRASH_DUMP_PROGRESS, vec![0; 8])), Err(Fault::FailFast { file: "clientproxy.cpp", line: 0x94 }));
}

#[test]
fn unknown_message_fails_fast() {
    let (mut h, _) = host(22621, true);
    assert!(matches!(h.dispatch(1, &Message::new(0x99, vec![])), Err(Fault::FailFast { .. })));
}

#[test]
fn controller_notify_forwards_0x94_bytes() {
    let (mut h, _) = host(22621, true);
    let p: Vec<u8> = (0..0xa0u32).map(|i| i as u8).collect();
    assert_eq!(h.dispatch(1, &Message::new(MSG_CONTROLLER_NOTIFY, p.clone())), Ok(None));
    assert_eq!(h.controller.as_ref().unwrap()[0], ControllerMessage { id: CTL_NOTIFY, payload: p[..0x94].to_vec() });
    assert_eq!(h.dispatch(1, &Message::new(MSG_CONTROLLER_NOTIFY, vec![0; 0x10])), Err(Fault::Malformed(MSG_CONTROLLER_NOTIFY)));
    h.controller = None;
    assert_eq!(h.dispatch(1, &Message::new(MSG_CONTROLLER_NOTIFY, p)), Ok(None));
}

#[test]
fn package_mount_success_records_entry_and_replies() {
    let (mut h, l) = host(22621, true);
    h.set_launch(&w("Ident!App"), [0; 16]);
    let req = PackageMount { name: w("DLC1"), path: w("[XUC:]\\dlc1.xvd") };
    let r = h.dispatch(1, &req.encode()).unwrap().unwrap();
    assert_eq!(r.id, MSG_PACKAGE_MOUNT);
    let resp = PackageMountResponse::decode(&r.payload).unwrap();
    assert_eq!(resp, PackageMountResponse { hr: 0, name: w("DLC1"), device_path: w("\\Device\\Xvd1") });
    assert_eq!(h.mounts, vec![Mount { identity: w("Ident!App"), name: w("DLC1"), device_path: w("\\Device\\Xvd1"), device: 0xd0 }]);
    assert_eq!(l.lock().unwrap().calls, vec!["open", "mount 0xa0 [XUC:]\\dlc1.xvd 0x19 1", "query 0xd0", "close 0xa0"]);
}

#[test]
fn package_mount_duplicate_name_is_access_denied() {
    let (mut h, l) = host(22621, true);
    h.package_mount(&PackageMount { name: w("DLC1"), path: w("a") });
    l.lock().unwrap().calls.clear();
    let r = h.package_mount(&PackageMount { name: w("dlc1"), path: w("b") });
    assert_eq!(r.hr, HR_ACCESS_DENIED);
    assert_eq!(r.name, w("dlc1"));
    assert!(r.device_path.is_empty());
    assert!(l.lock().unwrap().calls.is_empty());
    assert_eq!(h.mounts.len(), 1);
}

#[test]
fn package_mount_failures_clean_up() {
    let (mut h, l) = host(22621, true);
    l.lock().unwrap().open.push(Err(0x80070002u32 as i32));
    assert_eq!(h.package_mount(&PackageMount { name: w("A"), path: w("p") }).hr, 0x80070002u32 as i32);
    assert!(h.mounts.is_empty());
    assert_eq!(l.lock().unwrap().calls, vec!["open"]);
    l.lock().unwrap().calls.clear();
    l.lock().unwrap().mount.push(Err(0x80070003u32 as i32));
    assert_eq!(h.package_mount(&PackageMount { name: w("A"), path: w("p") }).hr, 0x80070003u32 as i32);
    assert_eq!(l.lock().unwrap().calls, vec!["open", "mount 0xa0 p 0x19 1", "close 0xa0"]);
    l.lock().unwrap().calls.clear();
    l.lock().unwrap().query.push(Err(0x8007000eu32 as i32));
    let r = h.package_mount(&PackageMount { name: w("A"), path: w("p") });
    assert_eq!(r.hr, 0x8007000eu32 as i32);
    assert_eq!(l.lock().unwrap().calls, vec!["open", "mount 0xa0 p 0x19 1", "query 0xd0", "unmount 0xa0 0xd0", "close 0xa0"]);
    assert!(h.mounts.is_empty());
}

#[test]
fn package_unmount_found_and_not_found() {
    let (mut h, l) = host(22621, true);
    let r = h.dispatch(1, &PackageUnmount { name: w("X") }.encode()).unwrap().unwrap();
    assert_eq!(PackageUnmountResponse::decode(&r.payload).unwrap(), PackageUnmountResponse { hr: HR_NOT_FOUND, name: w("X") });
    h.package_mount(&PackageMount { name: w("X"), path: w("p") });
    l.lock().unwrap().calls.clear();
    l.lock().unwrap().unmount.push(0x80070015u32 as i32);
    let r = h.package_unmount(&PackageUnmount { name: w("x") });
    assert_eq!(r.hr, 0x80070015u32 as i32);
    assert!(h.mounts.is_empty());
    assert_eq!(l.lock().unwrap().calls, vec!["open", "unmount 0xa0 0xd0", "close 0xa0"]);
}

#[test]
fn package_unmount_adapter_failure_keeps_entry() {
    let (mut h, l) = host(22621, true);
    h.package_mount(&PackageMount { name: w("X"), path: w("p") });
    l.lock().unwrap().open.push(Err(0x80070005u32 as i32));
    assert_eq!(h.package_unmount(&PackageUnmount { name: w("X") }).hr, 0x80070005u32 as i32);
    assert_eq!(h.mounts.len(), 1);
}

#[test]
fn malformed_mount_is_logged_without_reply() {
    let (mut h, _) = host(22621, true);
    assert_eq!(h.dispatch(1, &Message::new(MSG_PACKAGE_MOUNT, vec![9, 0, 0, 0])), Ok(None));
    assert_eq!(h.errors, vec![(0x48, HR_INVALID_DATA)]);
}

#[test]
fn mount_with_trailing_bytes_fails_fast() {
    let (mut h, _) = host(22621, true);
    let mut m = PackageMount { name: w("a"), path: w("b") }.encode();
    m.payload.extend_from_slice(&[0, 0, 0, 0]);
    assert_eq!(h.dispatch(1, &m), Err(Fault::FailFast { file: "MessageQueueMessageGenerator.h", line: 0x3f6 }));
}

#[test]
fn launch_message_publishes_plan_before_terminating_title() {
    let (mut h, _) = host(22621, true);
    h.set_title(&w("current.exe"), &[]);
    let request = LaunchNewGame { exe: w("game.exe"), args: w("-x") };
    let plan = Launch { exe: request.exe.clone(), args: request.args.clone(), mode: LAUNCH_MODE_NEW_GAME };

    assert_eq!(h.dispatch(1, &request.encode()), Ok(None));
    assert_eq!(h.actions, [Action::ExitPlan(Some(plan.clone())), Action::TerminateTitle]);
    assert_eq!(h.title_exited(), Some(plan));
    assert_eq!(h.exit_plan(), None);
}

#[test]
fn restart_message_queues_a_plan_for_crash_exit() {
    let (mut h, _) = host(22621, true);
    h.set_title(&w("game.exe"), &[]);
    let request = LaunchRestartOnCrash { args: w("-r"), value: 1 };
    let plan = Launch { exe: w("game.exe"), args: request.args.clone(), mode: LAUNCH_MODE_RESTART };

    assert_eq!(h.dispatch(1, &request.encode()), Ok(None));
    assert!(h.actions.is_empty());
    assert_eq!(h.exit_plan(), None);
    h.crash_dump_info(1);
    assert_eq!(h.exit_plan(), Some(plan.clone()));
    assert_eq!(h.title_exited(), Some(plan));
    assert_eq!(h.exit_plan(), None);
}
