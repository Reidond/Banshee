//! Toolhelp process-tree walking, for the UC-03 E3 orphan check.
//!
//! Given a parent PID recorded *before* a hard kill, enumerate the live process
//! table and return every descendant still running. A non-empty result after
//! the parent was terminated means the kill-on-close job object failed to reap
//! the tree (an orphan leak).

#![cfg(windows)]

use std::collections::HashMap;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// A single live process observed in the snapshot.
#[derive(Debug, Clone)]
pub struct ProcInfo {
    pub pid: u32,
    pub parent_pid: u32,
    pub name: String,
}

/// Snapshot the current process table.
pub fn snapshot() -> Vec<ProcInfo> {
    let mut out = Vec::new();
    // SAFETY: snapshot of all processes; 0 PID ignored for SNAPPROCESS.
    let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snap.is_null() {
        return out;
    }

    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    // SAFETY: valid snapshot + entry with dwSize set.
    let mut ok = unsafe { Process32FirstW(snap, &mut entry) };
    while ok != 0 {
        let name_end = entry
            .szExeFile
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(entry.szExeFile.len());
        let name = String::from_utf16_lossy(&entry.szExeFile[..name_end]);
        out.push(ProcInfo {
            pid: entry.th32ProcessID,
            parent_pid: entry.th32ParentProcessID,
            name,
        });
        // SAFETY: same valid snapshot + entry.
        ok = unsafe { Process32NextW(snap, &mut entry) };
    }

    // SAFETY: valid snapshot handle, closed once.
    unsafe { CloseHandle(snap) };
    out
}

/// Return all descendants (transitive children) of `root_pid` currently alive.
///
/// Note: PIDs can be recycled, so a definitive parent-PID walk is only sound
/// immediately after a snapshot. The orphan test snapshots quickly after the
/// kill + settle window, which is sufficient for a spike-grade check.
pub fn descendants(root_pid: u32) -> Vec<ProcInfo> {
    let procs = snapshot();
    let by_parent: HashMap<u32, Vec<&ProcInfo>> = {
        let mut m: HashMap<u32, Vec<&ProcInfo>> = HashMap::new();
        for p in &procs {
            m.entry(p.parent_pid).or_default().push(p);
        }
        m
    };

    let mut result = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if let Some(children) = by_parent.get(&pid) {
            for child in children {
                // Guard against a recycled PID pointing back at an ancestor.
                if child.pid != root_pid && !result.iter().any(|r: &ProcInfo| r.pid == child.pid) {
                    result.push((*child).clone());
                    stack.push(child.pid);
                }
            }
        }
    }
    result
}
