//! Windows process introspection shared by the Computer Use adapter (which
//! needs an executable path to name and gate a target) and the daemon's
//! helper reaper (which checks a registered pid still runs the helper).

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt as _;
use std::path::PathBuf;

use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, GetLastError};
use windows_sys::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};

/// The full path of `pid`'s executable image, or `None` when the process is
/// gone or cannot be queried at all.
pub fn executable_path(pid: u32) -> Option<PathBuf> {
    // SAFETY: plain Win32 calls on a handle opened and closed here; the
    // buffer outlives the query and `length` is set to what was written.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return None;
        }
        let mut buffer = vec![0_u16; 32 * 1024];
        let mut length = buffer.len() as u32;
        let queried = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            buffer.as_mut_ptr(),
            &mut length,
        );
        CloseHandle(handle);
        if queried == 0 {
            return None;
        }
        buffer.truncate(length as usize);
        Some(PathBuf::from(OsString::from_wide(&buffer)))
    }
}

/// Whether `pid` refuses an ordinary `PROCESS_QUERY_INFORMATION` open.
///
/// Another process of the same user grants that right unless it runs at a
/// higher integrity level — elevated, or a protected system process — and
/// those are exactly the windows User Interface Privilege Isolation makes
/// input to silently vanish. Refusing them up front beats a chain of no-ops.
pub fn is_protected(pid: u32) -> bool {
    // SAFETY: a plain open/close; the error code is read immediately after
    // the failing call on the same thread.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if handle.is_null() {
            return GetLastError() == ERROR_ACCESS_DENIED;
        }
        CloseHandle(handle);
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_process_resolves_to_its_own_executable() {
        let path = executable_path(std::process::id()).expect("own image name");
        let expected = std::env::current_exe().expect("current_exe");
        assert_eq!(
            path.file_name().unwrap().to_string_lossy().to_lowercase(),
            expected.file_name().unwrap().to_string_lossy().to_lowercase()
        );
        assert!(!is_protected(std::process::id()));
    }

    #[test]
    fn a_dead_pid_resolves_to_nothing() {
        assert_eq!(executable_path(u32::MAX - 1), None);
    }
}
