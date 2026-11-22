use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};
use xtfmsg::sys::*;

pub const SE_SHUTDOWN_NAME: &str = "SeShutdownPrivilege";
pub const SE_DEBUG_NAME: &str = "SeDebugPrivilege";
pub const ERROR_NO_SUCH_PRIVILEGE: u32 = 1313;
pub const SE_PRIVILEGE_ENABLED: u32 = 2;

pub const KNOWN: &[(&str, u32)] = &[
    ("SeCreateTokenPrivilege", 2),
    ("SeAssignPrimaryTokenPrivilege", 3),
    ("SeLockMemoryPrivilege", 4),
    ("SeIncreaseQuotaPrivilege", 5),
    ("SeTcbPrivilege", 7),
    ("SeSecurityPrivilege", 8),
    ("SeTakeOwnershipPrivilege", 9),
    ("SeLoadDriverPrivilege", 10),
    ("SeSystemProfilePrivilege", 11),
    ("SeSystemtimePrivilege", 12),
    ("SeProfileSingleProcessPrivilege", 13),
    ("SeIncreaseBasePriorityPrivilege", 14),
    ("SeCreatePagefilePrivilege", 15),
    ("SeCreatePermanentPrivilege", 16),
    ("SeBackupPrivilege", 17),
    ("SeRestorePrivilege", 18),
    ("SeShutdownPrivilege", 19),
    ("SeDebugPrivilege", 20),
    ("SeAuditPrivilege", 21),
    ("SeSystemEnvironmentPrivilege", 22),
    ("SeChangeNotifyPrivilege", 23),
    ("SeRemoteShutdownPrivilege", 24),
    ("SeUndockPrivilege", 25),
    ("SeManageVolumePrivilege", 28),
    ("SeImpersonatePrivilege", 29),
    ("SeCreateGlobalPrivilege", 30),
    ("SeIncreaseWorkingSetPrivilege", 33),
    ("SeTimeZonePrivilege", 34),
    ("SeCreateSymbolicLinkPrivilege", 35),
];

fn token() -> MutexGuard<'static, BTreeMap<u32, u32>> {
    static T: Mutex<BTreeMap<u32, u32>> = Mutex::new(BTreeMap::new());
    let mut g = T.lock().unwrap_or_else(|e| e.into_inner());
    if g.is_empty() {
        for (_, luid) in KNOWN {
            g.insert(*luid, if *luid == 23 { SE_PRIVILEGE_ENABLED } else { 0 });
        }
    }
    g
}

pub fn lookup(name: &str) -> Option<u32> {
    KNOWN.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, l)| *l)
}

pub fn is_enabled(name: &str) -> bool {
    lookup(name).and_then(|l| token().get(&l).copied()).is_some_and(|a| a & SE_PRIVILEGE_ENABLED != 0)
}

pub fn set_privilege(name: &str, enable: bool, restore: Option<&mut bool>) -> HRESULT {
    let Some(luid) = lookup(name) else {
        return hr_from_win32(ERROR_NO_SUCH_PRIVILEGE);
    };
    let mut t = token();
    let prev = t.get(&luid).copied().unwrap_or(0);
    let want = if enable { SE_PRIVILEGE_ENABLED } else { 0 };
    let changed = (prev & SE_PRIVILEGE_ENABLED) != want;
    t.insert(luid, (prev & !SE_PRIVILEGE_ENABLED) | want);
    if let Some(r) = restore {
        *r = changed && prev & SE_PRIVILEGE_ENABLED != 0;
    }
    S_OK
}
