// A generic loaded shared library — LoadLibrary/GetProcAddress on Windows, dlopen/dlsym on Unix.
// Direct port of lowarc/Bootstrap's dylib.rs: this part is fully generic (load a library, look
// up a symbol), nothing CLR-specific about it, so it's identical here even though this crate has
// no CLR loader at all.

use std::ffi::c_void;
use std::path::Path;

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::CString;

    pub type NativeString = Vec<u16>;

    // Unused on Windows today — symbol() takes a plain CString directly since GetProcAddress
    // wants narrow strings, not wide ones. Kept for parity with the Unix module (which does use
    // its own to_native internally) and for whatever eventually needs a wide string here.
    #[allow(dead_code)]
    pub fn to_native(s: &str) -> NativeString {
        use std::os::windows::ffi::OsStrExt;
        std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
    }
    pub fn path_to_native(p: &Path) -> NativeString {
        use std::os::windows::ffi::OsStrExt;
        p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
    }

    mod win {
        use std::ffi::c_void;
        #[link(name = "kernel32")]
        extern "system" {
            pub fn LoadLibraryW(path: *const u16) -> *mut c_void;
            pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
            pub fn FreeLibrary(module: *mut c_void) -> i32;
        }
    }

    pub struct Library(*mut c_void);
    impl Library {
        pub fn open(path: &Path) -> Result<Self, String> {
            let wide = path_to_native(path);
            let h = unsafe { win::LoadLibraryW(wide.as_ptr()) };
            if h.is_null() {
                return Err(format!("LoadLibraryW failed for {}", path.display()));
            }
            Ok(Self(h))
        }
        pub fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
            let cname = CString::new(name).unwrap();
            let p = unsafe { win::GetProcAddress(self.0, cname.as_ptr() as *const u8) };
            if p.is_null() {
                return Err(format!("GetProcAddress failed for {name}"));
            }
            Ok(p)
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe { win::FreeLibrary(self.0) };
        }
    }
}

#[cfg(unix)]
mod platform {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::{c_char, c_int};

    pub type NativeString = CString;

    pub fn to_native(s: &str) -> NativeString {
        CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
    }
    pub fn path_to_native(p: &Path) -> NativeString {
        use std::os::unix::ffi::OsStrExt;
        CString::new(p.as_os_str().as_bytes()).unwrap_or_else(|_| CString::new("").unwrap())
    }

    const RTLD_NOW: c_int = 2;

    #[cfg_attr(target_os = "linux", link(name = "dl"))]
    extern "C" {
        fn dlopen(filename: *const c_char, flag: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
        fn dlclose(handle: *mut c_void) -> c_int;
        fn dlerror() -> *mut c_char;
    }

    fn last_dlerror() -> String {
        unsafe {
            let p = dlerror();
            if p.is_null() { "unknown error".to_string() } else { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() }
        }
    }

    pub struct Library(*mut c_void);
    impl Library {
        pub fn open(path: &Path) -> Result<Self, String> {
            let cpath = path_to_native(path);
            unsafe { dlerror() };
            let h = unsafe { dlopen(cpath.as_ptr(), RTLD_NOW) };
            if h.is_null() {
                return Err(format!("dlopen failed for {}: {}", path.display(), last_dlerror()));
            }
            Ok(Self(h))
        }
        pub fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
            let cname = to_native(name);
            unsafe { dlerror() };
            let p = unsafe { dlsym(self.0, cname.as_ptr()) };
            if p.is_null() {
                return Err(format!("dlsym failed for {name}: {}", last_dlerror()));
            }
            Ok(p)
        }
    }
    impl Drop for Library {
        fn drop(&mut self) {
            unsafe { dlclose(self.0) };
        }
    }
}

pub use platform::Library;
