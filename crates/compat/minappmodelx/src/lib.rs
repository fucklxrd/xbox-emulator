use compat_common::os_host::{self, plm};

pub const EXPORTS_TXT: &str = include_str!(env!("MAM_EXPORTS"));
pub const DLL_FILE: &str = "minappmodelx.dll";

fn owner() {
    os_host::ensure();
}

#[no_mangle]
pub unsafe extern "system" fn mam_RegisterAppStateChangeNotification(routine: usize, context: usize, registration: *mut u64) -> u32 {
    owner();
    plm::register(plm::Kind::AppState, routine, context, registration)
}

#[no_mangle]
pub unsafe extern "system" fn mam_UnregisterAppStateChangeNotification(registration: u64) -> u32 {
    owner();
    plm::unregister(plm::Kind::AppState, registration)
}

#[no_mangle]
pub unsafe extern "system" fn mam_RegisterAppConstrainedChangeNotification(routine: usize, context: usize, registration: *mut u64) -> u32 {
    owner();
    plm::register(plm::Kind::Constrained, routine, context, registration)
}

#[no_mangle]
pub unsafe extern "system" fn mam_UnregisterAppConstrainedChangeNotification(registration: u64) -> u32 {
    owner();
    plm::unregister(plm::Kind::Constrained, registration)
}

#[cfg(test)]
mod tests;
