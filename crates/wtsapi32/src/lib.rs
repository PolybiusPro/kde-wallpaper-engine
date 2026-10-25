//! Proxy wtsapi32.dll for Wallpaper Engine under Wine.
//!
//! The engine converts each monitor's QueryDisplayConfig source mode into its
//! own bounds, swapping width and height when the path reports a 90/270 degree
//! rotation. Wine already reports the rotated desktop size in the source mode,
//! so a portrait monitor ends up as landscape. This patches the engine's import
//! of QueryDisplayConfig to report identity rotation.
//!
//! The engine only uses two functions from the real library (session lock
//! notifications), which mean nothing under Wine, so they are stubbed.

use std::ffi::{c_void, CStr};
use std::mem::transmute;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows_sys::Win32::Devices::Display::{
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_ROTATION_IDENTITY,
    DISPLAYCONFIG_TOPOLOGY_ID,
};
use windows_sys::core::BOOL;
use windows_sys::Win32::Foundation::{ERROR_SUCCESS, HWND, TRUE, WIN32_ERROR};
use windows_sys::Win32::System::Diagnostics::Debug::{
    IMAGE_DIRECTORY_ENTRY_IMPORT, IMAGE_NT_HEADERS64,
};
use windows_sys::Win32::System::LibraryLoader::{DisableThreadLibraryCalls, GetModuleHandleW};
use windows_sys::Win32::System::Memory::{VirtualProtect, PAGE_READWRITE};
use windows_sys::Win32::System::SystemServices::{
    DLL_PROCESS_ATTACH, IMAGE_DOS_HEADER, IMAGE_IMPORT_BY_NAME, IMAGE_IMPORT_DESCRIPTOR,
    IMAGE_ORDINAL_FLAG64,
};
use windows_sys::Win32::System::WindowsProgramming::IMAGE_THUNK_DATA64;

type QdcFn = unsafe extern "system" fn(
    u32,
    *mut u32,
    *mut DISPLAYCONFIG_PATH_INFO,
    *mut u32,
    *mut DISPLAYCONFIG_MODE_INFO,
    *mut DISPLAYCONFIG_TOPOLOGY_ID,
) -> WIN32_ERROR;

static REAL_QDC: AtomicUsize = AtomicUsize::new(0);

unsafe extern "system" fn hooked_qdc(
    flags: u32,
    np: *mut u32,
    paths: *mut DISPLAYCONFIG_PATH_INFO,
    nm: *mut u32,
    modes: *mut DISPLAYCONFIG_MODE_INFO,
    topo: *mut DISPLAYCONFIG_TOPOLOGY_ID,
) -> WIN32_ERROR {
    let real: QdcFn = transmute(REAL_QDC.load(Ordering::Relaxed));
    let r = real(flags, np, paths, nm, modes, topo);
    if r == ERROR_SUCCESS && !paths.is_null() && !np.is_null() {
        for i in 0..*np as usize {
            (*paths.add(i)).targetInfo.rotation = DISPLAYCONFIG_ROTATION_IDENTITY;
        }
    }
    r
}

unsafe fn patch_iat() {
    let base = GetModuleHandleW(std::ptr::null()) as *const u8;
    let dos = &*(base as *const IMAGE_DOS_HEADER);
    let nt = &*(base.offset(dos.e_lfanew as isize) as *const IMAGE_NT_HEADERS64);
    let dir = &nt.OptionalHeader.DataDirectory[IMAGE_DIRECTORY_ENTRY_IMPORT as usize];
    if dir.VirtualAddress == 0 {
        return;
    }
    let mut d = base.add(dir.VirtualAddress as usize) as *const IMAGE_IMPORT_DESCRIPTOR;
    while (*d).Name != 0 {
        let lib = CStr::from_ptr(base.add((*d).Name as usize) as *const _);
        if lib.to_bytes().eq_ignore_ascii_case(b"user32.dll") {
            let mut names = base.add((*d).Anonymous.OriginalFirstThunk as usize)
                as *const IMAGE_THUNK_DATA64;
            let mut slots = base.add((*d).FirstThunk as usize) as *mut IMAGE_THUNK_DATA64;
            while (*names).u1.AddressOfData != 0 {
                if (*names).u1.Ordinal & IMAGE_ORDINAL_FLAG64 == 0 {
                    let by_name = base.add((*names).u1.AddressOfData as usize)
                        as *const IMAGE_IMPORT_BY_NAME;
                    let name = CStr::from_ptr((*by_name).Name.as_ptr() as *const _);
                    if name.to_bytes() == b"QueryDisplayConfig" {
                        let slot = &mut (*slots).u1.Function as *mut u64 as *mut c_void;
                        let mut old = 0;
                        if VirtualProtect(slot, size_of::<usize>(), PAGE_READWRITE, &mut old) == 0 {
                            return;
                        }
                        REAL_QDC.store((*slots).u1.Function as usize, Ordering::Relaxed);
                        (*slots).u1.Function = hooked_qdc as *const () as usize as u64;
                        VirtualProtect(slot, size_of::<usize>(), old, &mut old);
                        return;
                    }
                }
                names = names.add(1);
                slots = slots.add(1);
            }
        }
        d = d.add(1);
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllMain(inst: *mut c_void, reason: u32, _reserved: *mut c_void) -> BOOL {
    if reason == DLL_PROCESS_ATTACH {
        DisableThreadLibraryCalls(inst);
        patch_iat();
    }
    TRUE
}

#[no_mangle]
pub extern "system" fn WTSRegisterSessionNotification(_h: HWND, _flags: u32) -> BOOL {
    TRUE
}

#[no_mangle]
pub extern "system" fn WTSUnRegisterSessionNotification(_h: HWND) -> BOOL {
    TRUE
}
