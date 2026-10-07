//! Minimal dynamic binding to FTDI's `ftd3xx.dll` (FT600/FT601 USB 3 FIFO bridge).
//!
//! Loaded at runtime so the app still starts (with the simulator) on machines without the
//! driver. Signatures follow FTDI's public `FTD3XX.h`.

use std::ffi::c_void;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Result};
use libloading::{Library, Symbol};

pub type Handle = *mut c_void;
type Status = u32;

pub const FT_OK: Status = 0;
pub const FT_OPEN_BY_INDEX: u32 = 0x10;
pub const FT_FLAGS_SUPERSPEED: u32 = 0x4;

type FnCreateDeviceInfoList = unsafe extern "system" fn(*mut u32) -> Status;
type FnGetDeviceInfoDetail = unsafe extern "system" fn(
    u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut u32,
    *mut c_void,
    *mut c_void,
    *mut Handle,
) -> Status;
type FnCreate = unsafe extern "system" fn(*mut c_void, u32, *mut Handle) -> Status;
type FnClose = unsafe extern "system" fn(Handle) -> Status;
type FnPipe = unsafe extern "system" fn(Handle, u8, *mut u8, u32, *mut u32, *mut c_void) -> Status;
type FnAbortPipe = unsafe extern "system" fn(Handle, u8) -> Status;
type FnClearStreamPipe = unsafe extern "system" fn(Handle, i32, i32, u8) -> Status;
type FnSetPipeTimeout = unsafe extern "system" fn(Handle, u8, u32) -> Status;
type FnGpio = unsafe extern "system" fn(Handle, u32, u32) -> Status;

/// Resolved function table. The `Library` is kept alive alongside the raw pointers.
pub struct Ftd3xx {
    _lib: Library,
    create_device_info_list: FnCreateDeviceInfoList,
    get_device_info_detail: FnGetDeviceInfoDetail,
    create: FnCreate,
    close: FnClose,
    write_pipe: FnPipe,
    read_pipe: FnPipe,
    abort_pipe: FnAbortPipe,
    clear_stream_pipe: FnClearStreamPipe,
    set_pipe_timeout: FnSetPipeTimeout,
    enable_gpio: FnGpio,
    write_gpio: FnGpio,
}

// The function pointers are plain C entry points; FTD3XX handles are usable across threads.
unsafe impl Send for Ftd3xx {}
unsafe impl Sync for Ftd3xx {}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct DeviceInfo {
    pub index: u32,
    pub flags: u32,
    pub id: u32,
    pub serial: String,
    pub description: String,
}

fn candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("ftd3xx.dll")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.join("ftd3xx.dll"));
        }
    }
    for var in ["ProgramFiles", "ProgramW6432"] {
        if let Ok(p) = std::env::var(var) {
            v.push(PathBuf::from(p).join("DataRay").join("FTD3XX.dll"));
        }
    }
    v
}

fn cstr(buf: &[u8]) -> String {
    let end = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

fn check(st: Status, what: &str) -> Result<()> {
    if st == FT_OK {
        Ok(())
    } else {
        bail!("{what} failed (FT_STATUS {st})")
    }
}

impl Ftd3xx {
    pub fn load() -> Result<Self> {
        let mut last = None;
        for path in candidates() {
            match unsafe { Library::new(&path) } {
                Ok(lib) => return unsafe { Self::from_lib(lib) },
                Err(e) => last = Some(e),
            }
        }
        Err(anyhow!("could not load ftd3xx.dll: {last:?}"))
    }

    unsafe fn from_lib(lib: Library) -> Result<Self> {
        unsafe fn sym<T: Copy>(lib: &Library, name: &[u8]) -> Result<T> {
            let s: Symbol<T> = lib.get(name)?;
            Ok(*s)
        }
        Ok(Self {
            create_device_info_list: sym(&lib, b"FT_CreateDeviceInfoList\0")?,
            get_device_info_detail: sym(&lib, b"FT_GetDeviceInfoDetail\0")?,
            create: sym(&lib, b"FT_Create\0")?,
            close: sym(&lib, b"FT_Close\0")?,
            write_pipe: sym(&lib, b"FT_WritePipe\0")?,
            read_pipe: sym(&lib, b"FT_ReadPipe\0")?,
            abort_pipe: sym(&lib, b"FT_AbortPipe\0")?,
            clear_stream_pipe: sym(&lib, b"FT_ClearStreamPipe\0")?,
            set_pipe_timeout: sym(&lib, b"FT_SetPipeTimeout\0")?,
            enable_gpio: sym(&lib, b"FT_EnableGPIO\0")?,
            write_gpio: sym(&lib, b"FT_WriteGPIO\0")?,
            _lib: lib,
        })
    }

    pub fn devices(&self) -> Result<Vec<DeviceInfo>> {
        let mut n = 0u32;
        check(unsafe { (self.create_device_info_list)(&mut n) }, "FT_CreateDeviceInfoList")?;
        let mut out = Vec::new();
        for index in 0..n {
            let (mut flags, mut ty, mut id, mut loc) = (0u32, 0u32, 0u32, 0u32);
            let mut serial = [0u8; 32];
            let mut desc = [0u8; 64];
            let mut h: Handle = std::ptr::null_mut();
            let st = unsafe {
                (self.get_device_info_detail)(
                    index,
                    &mut flags,
                    &mut ty,
                    &mut id,
                    &mut loc,
                    serial.as_mut_ptr().cast(),
                    desc.as_mut_ptr().cast(),
                    &mut h,
                )
            };
            if st == FT_OK {
                out.push(DeviceInfo {
                    index,
                    flags,
                    id,
                    serial: cstr(&serial),
                    description: cstr(&desc),
                });
            }
        }
        Ok(out)
    }

    pub fn open_by_index(&self, index: u32) -> Result<Handle> {
        let mut h: Handle = std::ptr::null_mut();
        check(
            unsafe { (self.create)(index as usize as *mut c_void, FT_OPEN_BY_INDEX, &mut h) },
            "FT_Create",
        )?;
        Ok(h)
    }

    pub fn close(&self, h: Handle) {
        unsafe { (self.close)(h) };
    }

    pub fn write_pipe(&self, h: Handle, pipe: u8, data: &[u8]) -> Result<u32> {
        let mut n = 0u32;
        let st = unsafe {
            (self.write_pipe)(h, pipe, data.as_ptr() as *mut u8, data.len() as u32, &mut n, std::ptr::null_mut())
        };
        check(st, "FT_WritePipe")?;
        Ok(n)
    }

    pub fn read_pipe(&self, h: Handle, pipe: u8, buf: &mut [u8]) -> Result<u32> {
        let mut n = 0u32;
        let st = unsafe {
            (self.read_pipe)(h, pipe, buf.as_mut_ptr(), buf.len() as u32, &mut n, std::ptr::null_mut())
        };
        check(st, "FT_ReadPipe")?;
        Ok(n)
    }

    pub fn abort_pipe(&self, h: Handle, pipe: u8) {
        unsafe { (self.abort_pipe)(h, pipe) };
    }

    pub fn clear_stream_pipe(&self, h: Handle, pipe: u8) {
        unsafe { (self.clear_stream_pipe)(h, 0, 0, pipe) };
    }

    pub fn set_pipe_timeout(&self, h: Handle, pipe: u8, ms: u32) -> Result<()> {
        check(unsafe { (self.set_pipe_timeout)(h, pipe, ms) }, "FT_SetPipeTimeout")
    }

    pub fn enable_gpio(&self, h: Handle, mask: u32, dir: u32) -> Result<()> {
        check(unsafe { (self.enable_gpio)(h, mask, dir) }, "FT_EnableGPIO")
    }

    pub fn write_gpio(&self, h: Handle, mask: u32, data: u32) -> Result<()> {
        check(unsafe { (self.write_gpio)(h, mask, data) }, "FT_WriteGPIO")
    }
}
