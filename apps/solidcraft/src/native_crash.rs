//! Windows: what a crash in native code leaves behind (docs/crash-reports.md).
//!
//! A crash inside a graphics driver or another DLL (an access violation, a stack overflow) is no
//! Rust panic: the panic hook never runs and the next start only found "an unclean exit". An
//! unhandled-exception filter now writes, before the process goes, a small JSON file with the
//! exception code, the faulting module and the offset into it, and a minidump, next to the run's
//! crash journal (`solidcraft_engine::crash::native_paths`). The next start puts both into the
//! unclean exit's report, with the startup step the journal says the app was in.
//!
//! Installing an exception filter and asking the system which module holds an address have no
//! safe API, hence the `unsafe_code` allowance on this module (the app crate denies it; see
//! `open_documents.rs` for the other one). The filter runs on a thread that may have a broken
//! heap or almost no stack left: it doesn't allocate, formats into a buffer on the stack, and
//! skips the minidump after a stack overflow.
#![allow(unsafe_code)]

use std::fmt::Write as _;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, HMODULE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{CREATE_ALWAYS, CreateFileW, FILE_ATTRIBUTE_NORMAL, WriteFile};
use windows_sys::Win32::System::Diagnostics::Debug::{
    EXCEPTION_CONTINUE_SEARCH, EXCEPTION_POINTERS, MINIDUMP_EXCEPTION_INFORMATION, MiniDumpNormal, MiniDumpWithThreadInfo,
    MiniDumpWithUnloadedModules, MiniDumpWriteDump, SetUnhandledExceptionFilter,
};
use windows_sys::Win32::System::LibraryLoader::{
    GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, GetModuleFileNameW, GetModuleHandleExW,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};

/// Where the filter writes (NUL-terminated UTF-16), set once by [`install`].
struct Paths {
    info: Vec<u16>,
    dump: Vec<u16>,
}

static PATHS: OnceLock<Paths> = OnceLock::new();
/// The filter that was installed before ours (called after ours), as an address.
static PREVIOUS: AtomicUsize = AtomicUsize::new(0);
/// The filter has started writing (a second exception while it does is left alone).
static BUSY: AtomicBool = AtomicBool::new(false);

const STATUS_ACCESS_VIOLATION: u32 = 0xC000_0005;
const STATUS_IN_PAGE_ERROR: u32 = 0xC000_0006;
const STATUS_STACK_OVERFLOW: u32 = 0xC000_00FD;

/// Catch native crashes from now on (after `solidcraft_engine::crash::install`; nothing without it).
pub fn install() {
    let Some((info, dump)) = solidcraft_engine::crash::native_paths() else { return };
    let wide = |p: &Path| p.as_os_str().encode_wide().chain([0]).collect::<Vec<u16>>();
    if PATHS.set(Paths { info: wide(&info), dump: wide(&dump) }).is_ok() {
        set_filter();
    }
}

/// Put the filter back on top: a DLL loaded since (a graphics driver) may have set its own,
/// which is then called after ours.
pub fn reinstall() {
    if PATHS.get().is_some() {
        set_filter();
    }
}

fn set_filter() {
    // SAFETY: `filter` has the signature the system calls it with and stays valid for the
    // process's life.
    let previous = unsafe { SetUnhandledExceptionFilter(Some(filter)) };
    if let Some(p) = previous
        && p as usize != filter as *const () as usize
    {
        PREVIOUS.store(p as usize, Ordering::SeqCst);
    }
}

/// The unhandled-exception filter: record, then let the previous filter (or the system) go on.
unsafe extern "system" fn filter(pointers: *const EXCEPTION_POINTERS) -> i32 {
    if !BUSY.swap(true, Ordering::SeqCst) {
        // SAFETY: the system passes valid exception pointers for the duration of the call.
        unsafe { record(pointers) };
    }
    let previous = PREVIOUS.load(Ordering::SeqCst);
    if previous != 0 && previous != filter as *const () as usize {
        // SAFETY: the address came from `SetUnhandledExceptionFilter` as a filter of this type.
        let previous: unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32 = unsafe { std::mem::transmute(previous) };
        // SAFETY: as for this filter.
        return unsafe { previous(pointers) };
    }
    EXCEPTION_CONTINUE_SEARCH
}

/// A fixed buffer to format into without allocating (what doesn't fit is cut off).
struct Buf {
    bytes: [u8; 2048],
    len: usize,
}

impl std::fmt::Write for Buf {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        let room = self.bytes.len().saturating_sub(self.len);
        let n = s.len().min(room);
        if let (Some(to), Some(from)) = (self.bytes.get_mut(self.len..self.len + n), s.as_bytes().get(..n)) {
            to.copy_from_slice(from);
            self.len += n;
        }
        Ok(())
    }
}

/// The usual names of the exception codes a crash shows.
fn code_name(code: u32) -> &'static str {
    match code {
        STATUS_ACCESS_VIOLATION => "EXCEPTION_ACCESS_VIOLATION",
        STATUS_IN_PAGE_ERROR => "EXCEPTION_IN_PAGE_ERROR",
        STATUS_STACK_OVERFLOW => "EXCEPTION_STACK_OVERFLOW",
        0xC000_001D => "EXCEPTION_ILLEGAL_INSTRUCTION",
        0xC000_0094 => "EXCEPTION_INT_DIVIDE_BY_ZERO",
        0xC000_0096 => "EXCEPTION_PRIV_INSTRUCTION",
        0xC000_008C => "EXCEPTION_ARRAY_BOUNDS_EXCEEDED",
        0xC000_0374 => "STATUS_HEAP_CORRUPTION",
        0xC000_0409 => "STATUS_STACK_BUFFER_OVERRUN",
        0xC000_0602 => "STATUS_FAIL_FAST_EXCEPTION",
        0x8000_0003 => "EXCEPTION_BREAKPOINT",
        0xE06D_7363 => "C++ exception",
        _ => "exception",
    }
}

/// Write what `pointers` says into the info file, then the minidump.
unsafe fn record(pointers: *const EXCEPTION_POINTERS) {
    let Some(paths) = PATHS.get() else { return };
    if pointers.is_null() {
        return;
    }
    // SAFETY: checked non-null; the system keeps it valid during the filter.
    let rec = unsafe { (*pointers).ExceptionRecord };
    if rec.is_null() {
        return;
    }
    // SAFETY: as above.
    let rec = unsafe { &*rec };
    let code = rec.ExceptionCode as u32;
    let address = rec.ExceptionAddress as usize;
    let mut module: HMODULE = std::ptr::null_mut();
    let mut name = [0u16; 512];
    // SAFETY: FROM_ADDRESS takes the address in place of a name; UNCHANGED_REFCOUNT means no
    // handle to release; `name` is as long as we say.
    let len = unsafe {
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            address as *const u16,
            &mut module,
        ) != 0
        {
            GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize
        } else {
            0
        }
    };
    // SAFETY: no preconditions.
    let thread = unsafe { GetCurrentThreadId() };
    let mut b = Buf { bytes: [0; 2048], len: 0 };
    let _ = write!(b, "{{\"code\":\"0x{code:08X}\",\"name\":\"{}\",\"address\":\"0x{address:016X}\",\"thread\":{thread}", code_name(code));
    if len > 0 {
        let _ = write!(b, ",\"offset\":\"0x{:X}\",\"module\":\"", address.wrapping_sub(module as usize));
        for c in char::decode_utf16(name.iter().take(len).copied()) {
            let _ = match c.unwrap_or(char::REPLACEMENT_CHARACTER) {
                '"' => b.write_str("\\\""),
                '\\' => b.write_str("\\\\"),
                c if (c as u32) < 0x20 => b.write_str("?"),
                c => b.write_char(c),
            };
        }
        let _ = b.write_str("\"");
    }
    if matches!(code, STATUS_ACCESS_VIOLATION | STATUS_IN_PAGE_ERROR) && rec.NumberParameters >= 2 {
        let what = match rec.ExceptionInformation[0] {
            0 => "read of",
            1 => "write to",
            8 => "execution at",
            _ => "access to",
        };
        let _ = write!(b, ",\"detail\":\"{what} 0x{:016X}\"", rec.ExceptionInformation[1]);
    }
    let _ = b.write_str("}");
    let bytes = b.bytes.get(..b.len).unwrap_or_default();
    // SAFETY: NUL-terminated path; the handle is closed below.
    unsafe {
        let file = create(&paths.info);
        if !file.is_null() {
            let mut written = 0u32;
            WriteFile(file, bytes.as_ptr(), bytes.len() as u32, &mut written, std::ptr::null_mut());
            CloseHandle(file);
        }
    }
    // After a stack overflow there is no stack left for writing a minidump.
    if code == STATUS_STACK_OVERFLOW {
        return;
    }
    // SAFETY: as above; the exception information points at this thread's exception.
    unsafe {
        let file = create(&paths.dump);
        if !file.is_null() {
            let exception = MINIDUMP_EXCEPTION_INFORMATION { ThreadId: thread, ExceptionPointers: pointers as *mut _, ClientPointers: 0 };
            MiniDumpWriteDump(
                GetCurrentProcess(),
                GetCurrentProcessId(),
                file,
                MiniDumpNormal | MiniDumpWithThreadInfo | MiniDumpWithUnloadedModules,
                &exception,
                std::ptr::null(),
                std::ptr::null(),
            );
            CloseHandle(file);
        }
    }
}

/// A new file at `path` (NUL-terminated) for writing, or null.
unsafe fn create(path: &[u16]) -> HANDLE {
    // SAFETY: the caller passes a NUL-terminated path.
    let h = unsafe { CreateFileW(path.as_ptr(), GENERIC_WRITE, 0, std::ptr::null(), CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, std::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE { std::ptr::null_mut() } else { h }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    /// The child half: install crash reporting and the filter, then fault in native code.
    #[test]
    fn child() {
        let Some(dir) = std::env::var_os("SOLIDCRAFT_NATIVE_CRASH_CHILD") else { return };
        let build = solidcraft_engine::crash::Build { version: "9.9.9", commit: "abc1234", built: "2026-10-14" };
        let fresh = solidcraft_engine::crash::install(PathBuf::from(dir), build);
        println!("FRESH {}", serde_json::to_string(&fresh).unwrap_or_default());
        if std::env::var_os("SOLIDCRAFT_NATIVE_CRASH_FAULT").is_none() {
            solidcraft_engine::crash::end_session();
            return;
        }
        super::install();
        solidcraft_engine::crash::phase("device creation");
        // SAFETY: none; this is the crash under test (a read of address 16, as a null struct
        // pointer's field would be).
        let v = unsafe { std::ptr::read_volatile(16 as *const u64) };
        println!("unreachable {v}");
    }

    fn run(dir: &std::path::Path, fault: bool) -> std::process::Output {
        let mut c = std::process::Command::new(std::env::current_exe().unwrap());
        c.args(["--exact", "native_crash::tests::child", "--test-threads=1", "--nocapture"]).env("SOLIDCRAFT_NATIVE_CRASH_CHILD", dir);
        if fault {
            c.env("SOLIDCRAFT_NATIVE_CRASH_FAULT", "1");
        }
        c.output().unwrap()
    }

    #[test]
    fn an_access_violation_is_reported_with_its_module_step_and_minidump() {
        let dir = std::env::temp_dir().join(format!("solidcraft-native-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = run(&dir, true);
        assert_eq!(out.status.code().map(|c| c as u32), Some(super::STATUS_ACCESS_VIOLATION), "{out:?}");
        let sessions: Vec<PathBuf> = std::fs::read_dir(dir.join("sessions")).unwrap().flatten().map(|e| e.path()).collect();
        let info = sessions.iter().find(|p| p.to_string_lossy().ends_with(".native.json")).expect("the filter wrote what it caught");
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(info).unwrap()).unwrap();
        assert_eq!(v["code"], "0xC0000005", "{v}");
        assert_eq!(v["detail"], "read of 0x0000000000000010", "{v}");
        assert!(v["module"].as_str().unwrap().to_lowercase().contains("solidcraft"), "the fault is in the test binary: {v}");
        let dump = sessions.iter().find(|p| p.extension().is_some_and(|e| e == "dmp")).expect("a minidump");
        assert_eq!(std::fs::read(dump).unwrap().get(..4), Some(&b"MDMP"[..]));
        // The next start reports it.
        let out = run(&dir, false);
        assert!(out.status.success(), "{out:?}");
        let reports = solidcraft_engine::crash::list(&dir);
        assert_eq!(reports.len(), 1, "{reports:?}");
        let r = solidcraft_engine::crash::read(&dir, &reports[0].id).unwrap();
        assert!(r["message"].as_str().unwrap().contains("EXCEPTION_ACCESS_VIOLATION (0xC0000005) in solidcraft"), "{r}");
        assert!(r["message"].as_str().unwrap().contains("during startup (device creation"), "{r}");
        assert!(r.get("minidump_path").is_some(), "{r}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
