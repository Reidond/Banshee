//! Windows job object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
//!
//! UC-03 step 2 / E3: the child is assigned to a kill-on-close job object so
//! that when the host (or this crate's owning process) dies, the entire child
//! tree is torn down and no orphaned `pwsh`/`conhost` processes survive.
//!
//! Ordering guarantee (see [`super::conpty`]): the child is created *suspended*,
//! assigned to the job, and only then resumed. This closes the race where a
//! child could spawn its own descendants before the job assignment lands.

#![cfg(windows)]

use std::io;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// An owned job-object handle configured to kill all assigned processes when
/// the handle (and thus every duplicate of it) is closed.
pub struct JobObject {
    handle: HANDLE,
}

// SAFETY: the wrapped HANDLE is a kernel object handle; it is only ever closed
// once (in Drop) and the API calls on it are all thread-safe kernel calls.
unsafe impl Send for JobObject {}
unsafe impl Sync for JobObject {}

impl JobObject {
    /// Create an anonymous job object with the kill-on-close limit set.
    pub fn new() -> io::Result<Self> {
        // SAFETY: null attributes/name => anonymous default job object.
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        // SAFETY: `info` is a correctly sized, initialized struct of the class we pass.
        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of!(info).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            // SAFETY: handle is valid and owned here.
            unsafe { CloseHandle(handle) };
            return Err(err);
        }

        Ok(Self { handle })
    }

    /// Assign a (typically suspended) process to this job object.
    pub fn assign(&self, process: HANDLE) -> io::Result<()> {
        // SAFETY: both handles are valid kernel handles owned by the caller/self.
        let ok = unsafe { AssignProcessToJobObject(self.handle, process) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for JobObject {
    fn drop(&mut self) {
        // Closing the last handle to a kill-on-close job terminates every
        // process still assigned to it (UC-03 E3).
        // SAFETY: handle is valid and owned; dropped exactly once.
        unsafe { CloseHandle(self.handle) };
    }
}
