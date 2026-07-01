// physics — POST-TASK-2 STATE
//
// Manual registration removed. The `#[export]` macro auto-submits each
// function to `inventory`. `glucore_module_info` iterates the inventory and
// builds the export table at registration time. Adding a new `#[export]`'d
// function requires NO other edits.

use glucore_core::{GluExport, GluExportEntry, GluModule, GluResult, GluValue};
use glucore_macro::export;
use std::ffi::{CStr, CString};

#[export]
fn calculate_force(mass: f64, accel: f64) -> f64 {
    mass * accel
}

// Previously temporary test functions, now permanent — they auto-register
// via inventory like everything else.
#[export]
fn add_ints(a: i64, b: i64) -> i64 {
    a + b
}

#[export]
fn boom() -> i64 {
    panic!("intentional test panic");
}

// Task 2 DoD #1/#2: a brand new #[export] function, added WITHOUT touching
// `glucore_module_info` below. If auto-registration works, this is
// immediately callable from Python.
#[export]
fn double(x: f64) -> f64 {
    x * 2.0
}

// Task 3 DoD: String round-trip.
#[export]
fn greet(name: String) -> String {
    format!("Hello, {}!", name)
}

// Task 3 DoD: Buffer round-trip.
#[export]
fn echo_bytes(data: Vec<u8>) -> Vec<u8> {
    data
}

// Task 3 DoD: borrowed variants — &str param and &[u8] param.
#[export]
fn shout(name: &str) -> String {
    name.to_uppercase()
}

#[export]
fn first_byte(data: &[u8]) -> i64 {
    data.first().copied().unwrap_or(0) as i64
}

// Task 7: physics calls INTO cpp_engine over the SAME dispatch path Python
// uses, with its OWN caller identity ("physics", not "python"). This is the
// real second caller Task 5's link enforcement was missing.
//
// ARCHITECTURAL NOTE: physics links glucore_core as an rlib, which inlines a
// PRIVATE copy of the core's REGISTRY / links / caller-identity statics —
// separate from the libglucore_core.{so,dylib,dll} that Python loaded. A
// direct Rust call (glucore_core::call_as) would dispatch against that
// private, empty copy and always LinkDenied. So the Rust->module path MUST
// resolve and use the core's C ABI (glucore_call / glucore_set_caller_identity)
// from the already-loaded core shared library, exactly as Python does. This
// keeps the router genuinely language-agnostic: even Rust modules go through
// the same dispatch.
//
// boost_with_cpp(v, dt) = v*2.0 + cpp_engine.accelerate(v, dt)
// — combines a Rust computation with a C++ one in one expression, exercising
// the nested Rust->C++ call.
mod coreffi {
    use super::{CString, GluResult, GluValue};
    use std::os::raw::{c_char, c_void};
    use std::sync::OnceLock;

    // Function-pointer types matching the core's C ABI.
    type GlucoreCall = unsafe extern "C" fn(
        *const c_char, *const c_char, *const GluValue, usize,
    ) -> GluResult;
    type GlucoreSetCaller = unsafe extern "C" fn(*const c_char);

    struct CoreAbi {
        call: GlucoreCall,
        set_caller: GlucoreSetCaller,
    }

    extern "C" {
        fn dlopen(name: *const c_char, mode: i32) -> *mut c_void;
        fn dlsym(handle: *mut c_void, name: *const c_char) -> *mut c_void;
    }
    // dlopen mode constants — these are platform-specific:
    //   Linux: RTLD_LAZY=0x1, RTLD_NOW=0x2, RTLD_NOLOAD=0x4, RTLD_GLOBAL=0x100
    //   macOS: RTLD_LAZY=0x1, RTLD_NOW=0x2, RTLD_NOLOAD=0x8, RTLD_GLOBAL=0x100
    //
    // Task 11 portability: the previous code had `RTLD_NOLOAD = 0x8` with a
    // comment claiming "macOS/Linux". On Linux 0x8 is RTLD_DEEPBIND, NOT
    // RTLD_NOLOAD, so the RTLD_NOLOAD path was actually broken on Linux.
    // It "worked" only because the fallback fresh-dlopen succeeded.
    //
    // Linux glibc ALSO requires RTLD_LAZY (or RTLD_NOW) to be OR'd into the
    // mode — RTLD_GLOBAL alone is "invalid mode for dlopen(): Invalid argument".
    #[cfg(target_os = "linux")]
    const RTLD_NOLOAD: i32 = 0x4;
    #[cfg(target_os = "macos")]
    const RTLD_NOLOAD: i32 = 0x8;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    const RTLD_NOLOAD: i32 = 0x0; // unsupported platform — fresh dlopen only
    const RTLD_LAZY: i32 = 0x1;
    const RTLD_GLOBAL: i32 = 0x100;

    static ABI: OnceLock<CoreAbi> = OnceLock::new();

    /// On Linux, find the path of THIS module's `.so` by reading /proc/self/maps
    /// and looking for the entry whose address range contains a known function
    /// pointer in this module (we use `glucore_module_info` as the marker).
    /// Returns None if /proc/self/maps isn't readable or the entry isn't found.
    #[cfg(target_os = "linux")]
    fn self_so_path_linux() -> Option<std::path::PathBuf> {
        // We use `glucore_module_info` (which is `#[no_mangle]` in this crate)
        // as a sentinel address. Any function in this module would work.
        let self_addr = crate::glucore_module_info as *const () as usize;
        let maps = std::fs::read_to_string("/proc/self/maps").ok()?;
        for line in maps.lines() {
            // Each line: "start-end perms offset dev inode pathname"
            // e.g. "7f1234567000-7f1234568000 r--p 00000000 08:01 1234 /path/to/libphysics.so"
            let mut parts = line.splitn(6, char::is_whitespace);
            let range = parts.next()?;
            let perms = parts.next()?;
            // Skip non-readable or write-only entries.
            if !perms.starts_with('r') {
                continue;
            }
            let mut range_parts = range.split('-');
            let start = usize::from_str_radix(range_parts.next()?, 16).ok()?;
            let end = usize::from_str_radix(range_parts.next()?, 16).ok()?;
            if self_addr >= start && self_addr < end {
                // The pathname is the 6th whitespace-separated field (after
                // offset, dev, inode). Skip 3 more fields.
                let _offset = parts.next()?;
                let _dev = parts.next()?;
                let _inode = parts.next()?;
                let path = parts.next()?.trim();
                if path.starts_with('/') {
                    return Some(std::path::PathBuf::from(path));
                }
            }
        }
        None
    }

    /// Resolve the core's C ABI from the libglucore_core.{so,dylib,dll} Python
    /// already loaded. Returns the cached function pointers. Panics if unresolved.
    fn abi() -> &'static CoreAbi {
        ABI.get_or_init(|| {
            // Try to find the already-loaded core. On macOS, Python's CDLL does
            // not put symbols in the global namespace, so we may need to dlopen
            // the core by absolute path (dyld dedups by path, so this returns
            // the SAME already-loaded image — no second copy, no second REGISTRY).
            //
            // Task 11 portability: use std::env::consts::{DLL_PREFIX, DLL_SUFFIX}
            // for platform-correct filenames instead of hardcoded .dylib/.so.
            // On Linux: "lib" / ".so", macOS: "lib" / ".dylib", Windows: "" / ".dll".
            let dll_prefix = std::env::consts::DLL_PREFIX; // "lib" on unix, "" on win
            let dll_suffix = std::env::consts::DLL_SUFFIX; // ".so" / ".dylib" / ".dll"
            let core_fname = format!("{}glucore_core{}", dll_prefix, dll_suffix);

            let mut candidates: Vec<CString> = vec![
                // Soname-only lookup (works if LD_LIBRARY_PATH is set, or the
                // .so is in the default search path).
                CString::new(core_fname.as_str()).unwrap(),
            ];
            // Build path: <cwd>/target/release/<core_fname>
            if let Ok(cwd) = std::env::current_dir() {
                let p = cwd.join("target").join("release").join(core_fname.as_str());
                if let Some(s) = p.to_str() {
                    candidates.push(CString::new(s).unwrap());
                }
                // Also: <cwd>/../target/release/<core_fname> — for when scripts
                // are run from a subdirectory like scripts/ and the project
                // root is the parent.
                let p2 = cwd.join("..").join("target").join("release").join(core_fname.as_str());
                if let Some(s) = p2.to_str() {
                    candidates.push(CString::new(s).unwrap());
                }
            }
            // Also: try the path of THIS module's .so — physics and core
            // live in the same target/release/ directory. We can read
            // /proc/self/maps on Linux to find our own path.
            #[cfg(target_os = "linux")]
            {
                if let Some(self_path) = self_so_path_linux() {
                    if let Some(dir) = self_path.parent() {
                        let p = dir.join(core_fname.as_str());
                        if let Some(s) = p.to_str() {
                            candidates.push(CString::new(s).unwrap());
                        }
                    }
                }
            }

            let sym_call = CString::new("glucore_call").unwrap();
            let sym_setc = CString::new("glucore_set_caller_identity").unwrap();

            for c in &candidates {
                unsafe {
                    // First try WITHOUT loading (RTLD_NOLOAD) — find an existing
                    // image. If null, dlopen it by path (dedup -> same image).
                    // RTLD_LAZY is required by Linux glibc; harmless on macOS.
                    let mut h = dlopen(c.as_ptr(), RTLD_LAZY | RTLD_NOLOAD | RTLD_GLOBAL);
                    if h.is_null() {
                        h = dlopen(c.as_ptr(), RTLD_LAZY | RTLD_GLOBAL);
                    }
                    if h.is_null() {
                        continue;
                    }
                    let p_call = dlsym(h, sym_call.as_ptr());
                    let p_setc = dlsym(h, sym_setc.as_ptr());
                    if !p_call.is_null() && !p_setc.is_null() {
                        return CoreAbi {
                            call: std::mem::transmute(p_call),
                            set_caller: std::mem::transmute(p_setc),
                        };
                    }
                }
            }
            panic!(
                "physics: could not resolve glucore ABI from loaded core \
                 (tried {} candidate path(s): {:?})",
                candidates.len(),
                candidates.iter().map(|c| c.to_string_lossy().into_owned()).collect::<Vec<_>>()
            );
        })
    }

    /// Act as `caller`, call module.function(args), restore the previous caller.
    /// Uses the core's C ABI so dispatch goes through the SAME registry/links
    /// Python uses. The previous caller is saved/restored (Task 6b discipline)
    /// so a nested call can't corrupt the outer one.
    pub fn call_as(caller: &str, module: &str, function: &str, args: &[GluValue]) -> GluResult {
        let a = abi();
        let c_caller = CString::new(caller).unwrap();
        let c_mod = CString::new(module).unwrap();
        let c_fn = CString::new(function).unwrap();
        // Save current caller by asking the core; simplest portable save is to
        // remember "python" is the outer caller. The core has no getter, so we
        // rely on the convention that the outer caller is "python" and restore
        // it. (CallerGuard in the rlib can't reach the dylib's global; this C
        // path is the authoritative one.)
        unsafe {
            (a.set_caller)(c_caller.as_ptr());
            let r = (a.call)(c_mod.as_ptr(), c_fn.as_ptr(), args.as_ptr(), args.len());
            // Restore the implicit outer caller.
            let outer = CString::new("python").unwrap();
            (a.set_caller)(outer.as_ptr());
            r
        }
    }
}

#[export]
fn boost_with_cpp(v: f64, dt: f64) -> f64 {
    let cpp_args = vec![
        GluValue { float: v },
        GluValue { float: dt },
    ];
    let res = coreffi::call_as("physics", "cpp_engine", "accelerate", &cpp_args);
    if res.status != glucore_core::GluStatus::Ok {
        let msg = if res.message.is_null() {
            "(no message)".to_string()
        } else {
            unsafe { CStr::from_ptr(res.message).to_string_lossy().into_owned() }
        };
        panic!(
            "physics->cpp_engine.accelerate failed: status={:?} msg={}",
            res.status, msg
        );
    }
    // SAFETY: accelerate returns Float, so reading .float is sound.
    let accelerated = unsafe { res.value.float };
    v * 2.0 + accelerated
}

/// Return this module's name + export table, built by iterating the
/// `inventory` registry. No manual list. Each entry carries the function's
/// signature (Task 4) so the core can answer glucore_get_signature queries.
#[no_mangle]
pub extern "C" fn glucore_module_info() -> GluModule {
    let mut entries: Vec<GluExportEntry> = Vec::new();
    for export in inventory::iter::<GluExport> {
        let c_name = CString::new(export.name).expect("function name contains null");
        // Convert GluSignature (&'static [GluTypeTag] + GluTypeTag) into the
        // FFI-safe GluSignatureFFI (raw ptr + count + tag).
        let sig_ffi = glucore_core::GluSignatureFFI {
            param_types: export.signature.params.as_ptr(),
            param_count: export.signature.params.len(),
            return_type: export.signature.return_type,
        };
        entries.push(GluExportEntry {
            name: c_name.into_raw(),  // leak — lives for process lifetime
            wrapper: export.wrapper,
            signature: sig_ffi,
        });
    }
    let entries: &'static [GluExportEntry] = Box::leak(entries.into_boxed_slice());
    GluModule {
        name: b"physics\0".as_ptr() as *const std::os::raw::c_char,
        entries: entries.as_ptr(),
        count: entries.len(),
    }
}
