use crate::api::get_server_port;
use crate::base::{log_error_args, log_info_args};
use crate::conn::{TcpConnection, TcpConnection_InitializeServer_sp, XbtpConnection};
use crate::dispatcher::*;
use crate::dispatcher_com::{wrap_dispatcher, DispatcherCom};
use crate::link_conn::link_initialize_server;
use crate::msvc::{cpp_delete, cpp_new, SharedPtr};
use crate::registry::{discover_registry_extensions, XbtpExtensionRegistry, XbtpExtensionRegistry_Initialize, XbtpExtensionRegistry_dtor};
use crate::service_info::{create_service_info_extension_metadata, log_pipe_failure, SERVICE_INFO_EXTENSION};
use crate::sys::*;
use crate::xbtp2_ext::com_release;
use core::ffi::c_void;
use core::ptr::null_mut;

pub const XVM_PIPE_NAMES: [&str; 3] = ["\\\\.\\SEVPIPE\\XTF_SYSTEM_TO_TITLE", "\\\\.\\pipe\\XtfLoopback", "\\\\.\\HEVPIPE\\XTF_HOST_TO_TITLE"];

unsafe fn drop_registry(p: *mut c_void) {
    XbtpExtensionRegistry_dtor(p as *mut XbtpExtensionRegistry);
    cpp_delete(p);
}

unsafe fn drop_dispatcher(p: *mut c_void) {
    XbtpConnectionDispatcher_dtor(p as *mut XbtpConnectionDispatcher);
    cpp_delete(p);
}

pub unsafe fn add_vm_named_pipe_endpoints(dispatcher: *mut XbtpConnectionDispatcher) -> HRESULT {
    let mut hr = S_OK;
    for name in XVM_PIPE_NAMES.iter() {
        let w = wide(name);
        hr = XbtpConnectionDispatcher_AddPipeEndpoint(dispatcher, w.as_ptr());
        if hr < 0 {
            break;
        }
    }
    hr
}

#[no_mangle]
pub unsafe extern "C" fn XbtpStartDispatcherEx(port: u32, is_system_server: i32, iid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    if ppv.is_null() {
        return E_INVALIDARG;
    }
    let hr = crate::xbtp2_sys::XbtpInitialize();
    if hr < 0 {
        return hr;
    }
    let mut tcp = SharedPtr::<TcpConnection>::default();
    let hr = TcpConnection_InitializeServer_sp(port, &mut tcp);
    if hr < 0 {
        return hr;
    }
    let d = cpp_new(8) as *mut XbtpConnectionDispatcher;
    if d.is_null() {
        tcp.release();
        return E_OUTOFMEMORY;
    }
    (*d).m_impl = core::ptr::null();
    let mut disp = match SharedPtr::new_with(d, drop_dispatcher) {
        Some(p) => p,
        None => {
            cpp_delete(d as *mut c_void);
            tcp.release();
            return E_OUTOFMEMORY;
        }
    };
    let r = cpp_new(8) as *mut XbtpExtensionRegistry;
    if r.is_null() {
        tcp.release();
        disp.release();
        return E_OUTOFMEMORY;
    }
    (*r).m_impl = null_mut();
    let mut reg = match SharedPtr::new_with(r, drop_registry) {
        Some(p) => p,
        None => {
            cpp_delete(r as *mut c_void);
            tcp.release();
            disp.release();
            return E_OUTOFMEMORY;
        }
    };
    let hr = 'run: {
        let hr = XbtpExtensionRegistry_Initialize(reg.ptr);
        if hr < 0 {
            break 'run hr;
        }
        let mut link = SharedPtr::<XbtpConnection>::default();
        let hr = link_initialize_server(port, &mut link);
        if hr < 0 {
            break 'run hr;
        }
        let mut reg_arg = reg.clone_ref();
        let mut tcp_arg: SharedPtr<XbtpConnection> = tcp.clone_ref().cast();
        let hr = XbtpConnectionDispatcher_Initialize(d, &mut reg_arg, &mut tcp_arg, &mut link);
        if hr < 0 {
            break 'run hr;
        }
        if is_system_server == 1 {
            let hr = discover_registry_extensions(reg.ptr);
            if hr < 0 {
                break 'run hr;
            }
            let hr = add_vm_named_pipe_endpoints(d);
            if hr < 0 {
                log_pipe_failure(hr);
            }
        }
        let mut com: *mut c_void = null_mut();
        let hr = wrap_dispatcher(disp.clone_ref(), &IID_XBTP_DISPATCHER, &mut com);
        if hr < 0 || com.is_null() {
            break 'run if hr < 0 { hr } else { E_OUTOFMEMORY };
        }
        let c = com as *mut DispatcherCom;
        let hr = ((*(*c).vtbl).query_interface)(c, iid, ppv);
        if hr < 0 {
            com_release(com);
            break 'run hr;
        }
        let mut md: *mut c_void = null_mut();
        let hr = create_service_info_extension_metadata(dispatcher_impl(d).as_ref(), &mut md);
        if hr < 0 {
            com_release(com);
            break 'run hr;
        }
        let hr = ((*(*c).vtbl).add_extension)(c, &SERVICE_INFO_EXTENSION, md);
        com_release(md);
        if hr < 0 {
            com_release(com);
            break 'run hr;
        }
        let hr = start_running(d);
        com_release(com);
        hr
    };
    if hr < 0 {
        log_error_args(b"Failed 0x%X to start dispatcher\0", &[hr as u32 as usize]);
    }
    reg.release();
    tcp.release();
    disp.release();
    hr
}

#[no_mangle]
pub unsafe extern "C" fn XbtpStartDispatcher(server_port: u32, is_system_server: i32, iid: *const GUID, ppv: *mut *mut c_void) -> HRESULT {
    let hr = crate::xbtp2_sys::XbtpInitialize();
    if hr < 0 {
        return hr;
    }
    let port = match get_server_port(server_port) {
        Ok(p) => p,
        Err(hr) => return hr,
    };
    let hr = XbtpStartDispatcherEx(port, is_system_server, iid, ppv);
    if hr >= 0 {
        log_info_args(b"Dispatcher started on port %d\0", &[port as usize]);
    }
    hr
}
