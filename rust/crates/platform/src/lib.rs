use seraphine_protocol::ProcessInfo;
use std::ffi::c_void;
use std::fmt;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, WaitForSingleObject, PROCESS_BASIC_INFORMATION,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};
use zeroize::Zeroizing;

#[derive(Debug)]
pub enum PlatformError {
    SnapshotFailed,
    EnumerationFailed,
    OpenProcessFailed,
    CommandLineFailed,
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SnapshotFailed => formatter.write_str("failed to create process snapshot"),
            Self::EnumerationFailed => formatter.write_str("failed to enumerate processes"),
            Self::OpenProcessFailed => formatter.write_str("failed to open process"),
            Self::CommandLineFailed => formatter.write_str("failed to read process command line"),
        }
    }
}

impl std::error::Error for PlatformError {}

struct SnapshotHandle(*mut c_void);

impl Drop for SnapshotHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn process_name(entry: &PROCESSENTRY32W) -> String {
    let length = entry
        .szExeFile
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(entry.szExeFile.len());
    String::from_utf16_lossy(&entry.szExeFile[..length])
}

pub fn list_processes() -> Result<Vec<ProcessInfo>, PlatformError> {
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE {
        return Err(PlatformError::SnapshotFailed);
    }
    let handle = SnapshotHandle(handle);
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    let mut processes = Vec::new();
    let mut has_entry = unsafe { Process32FirstW(handle.0, &mut entry) } != 0;
    while has_entry {
        processes.push(ProcessInfo {
            pid: entry.th32ProcessID,
            name: process_name(&entry),
        });
        has_entry = unsafe { Process32NextW(handle.0, &mut entry) } != 0;
    }

    if processes.is_empty() {
        return Err(PlatformError::EnumerationFailed);
    }
    Ok(processes)
}

pub fn list_league_client_pids() -> Result<Vec<u32>, PlatformError> {
    Ok(list_processes()?
        .into_iter()
        .filter(|process| process.name.eq_ignore_ascii_case("LeagueClientUx.exe"))
        .map(|process| process.pid)
        .collect())
}

fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub fn atomic_replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    let temporary = wide_path(temporary);
    let destination = wide_path(destination);
    let success = unsafe {
        MoveFileExW(
            temporary.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if success == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MemoryInfo {
    pub working_set_bytes: u64,
    pub peak_working_set_bytes: u64,
    pub private_usage_bytes: u64,
    pub peak_pagefile_usage_bytes: u64,
}

pub fn current_process_memory() -> Result<MemoryInfo, PlatformError> {
    let mut counters: PROCESS_MEMORY_COUNTERS_EX = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    let success = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
            counters.cb,
        )
    };
    if success == 0 {
        return Err(PlatformError::EnumerationFailed);
    }
    Ok(MemoryInfo {
        working_set_bytes: counters.WorkingSetSize as u64,
        peak_working_set_bytes: counters.PeakWorkingSetSize as u64,
        private_usage_bytes: counters.PrivateUsage as u64,
        peak_pagefile_usage_bytes: counters.PeakPagefileUsage as u64,
    })
}

pub struct ProcessHandle(*mut c_void);

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

impl ProcessHandle {
    pub fn raw(&self) -> *mut c_void {
        self.0
    }

    pub fn wait_for_exit(&self, timeout_ms: u32) -> bool {
        let result = unsafe { WaitForSingleObject(self.0, timeout_ms) };
        result != WAIT_TIMEOUT
    }
}

pub fn open_process(pid: u32, access: u32) -> Result<ProcessHandle, PlatformError> {
    let handle = unsafe { OpenProcess(access, 0, pid) };
    if handle.is_null() {
        return Err(PlatformError::OpenProcessFailed);
    }
    Ok(ProcessHandle(handle))
}

fn read_process_memory(
    handle: &ProcessHandle,
    address: *const c_void,
    size: usize,
) -> Result<Vec<u8>, PlatformError> {
    let mut buffer = vec![0_u8; size];
    let mut bytes_read = 0_usize;
    let success = unsafe {
        ReadProcessMemory(
            handle.raw(),
            address,
            buffer.as_mut_ptr() as *mut c_void,
            size,
            &mut bytes_read,
        )
    };
    if success == 0 || bytes_read != size {
        return Err(PlatformError::CommandLineFailed);
    }
    Ok(buffer)
}

#[derive(Clone)]
pub struct ProcessCommandLine {
    pub pid: u32,
    pub port: Option<u16>,
    pub token: Option<Zeroizing<String>>,
    pub server: Option<String>,
}

impl ProcessCommandLine {
    pub fn redacted(&self) -> ProcessInfo {
        ProcessInfo {
            pid: self.pid,
            name: "LeagueClientUx.exe".to_string(),
        }
    }
}

pub fn parse_command_line(pid: u32, command_line: &str) -> ProcessCommandLine {
    let mut port = None;
    let mut token = None;
    let mut server = None;
    for argument in command_line.split_whitespace() {
        let argument = argument.trim_matches('"');
        if let Some(value) = argument.strip_prefix("--app-port=") {
            port = value.parse().ok();
        } else if let Some(value) = argument.strip_prefix("--remoting-auth-token=") {
            token = Some(Zeroizing::new(value.to_string()));
        } else if let Some(value) = argument.strip_prefix("--rso_platform_id=") {
            server = Some(value.to_string());
        }
    }
    ProcessCommandLine {
        pid,
        port,
        token,
        server,
    }
}

pub fn read_process_command_line(pid: u32) -> Result<ProcessCommandLine, PlatformError> {
    let handle = open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)?;
    let mut basic: PROCESS_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
    let status = unsafe {
        NtQueryInformationProcess(
            handle.raw(),
            ProcessBasicInformation,
            &mut basic as *mut PROCESS_BASIC_INFORMATION as *mut c_void,
            std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
            std::ptr::null_mut(),
        )
    };
    if status != 0 {
        return Err(PlatformError::CommandLineFailed);
    }

    let peb = basic.PebBaseAddress as *const c_void;
    let process_parameters_bytes = read_process_memory(&handle, unsafe { peb.add(0x20) }, 8)?;
    let process_parameters =
        usize::from_ne_bytes(process_parameters_bytes.try_into().unwrap()) as *const c_void;
    let command_line_bytes =
        read_process_memory(&handle, unsafe { process_parameters.add(0x70) }, 16)?;
    let length = u16::from_ne_bytes([command_line_bytes[0], command_line_bytes[1]]) as usize;
    if length == 0 || length > 64 * 1024 {
        return Err(PlatformError::CommandLineFailed);
    }
    let buffer = usize::from_ne_bytes(command_line_bytes[8..16].try_into().unwrap()) as *const u16;
    let wide = read_process_memory(&handle, buffer as *const c_void, length)?;
    let mut units = Vec::with_capacity(length / 2);
    for chunk in wide.chunks_exact(2) {
        units.push(u16::from_ne_bytes([chunk[0], chunk[1]]));
    }
    let command_line = String::from_utf16_lossy(&units);
    Ok(parse_command_line(pid, &command_line))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_current_process_command_line() {
        let pid = std::process::id();
        let parsed = read_process_command_line(pid).unwrap();
        assert_eq!(parsed.pid, pid);
    }

    #[test]
    fn atomically_replaces_existing_file() {
        let directory = std::env::temp_dir().join(format!(
            "seraphine-atomic-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let destination = directory.join("cache.json");
        let temporary = directory.join("cache.json.tmp");
        std::fs::write(&destination, b"old").unwrap();
        std::fs::write(&temporary, b"new").unwrap();
        atomic_replace_file(&temporary, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!temporary.exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn parses_lcu_arguments_without_exposing_token() {
        let parsed = parse_command_line(
            42,
            "LeagueClientUx.exe \"--app-port=55123\" \"--remoting-auth-token=test-token\" \"--rso_platform_id=HN1\"",
        );
        assert_eq!(parsed.pid, 42);
        assert_eq!(parsed.port, Some(55123));
        assert_eq!(
            parsed.token.as_ref().map(|token| token.as_str()),
            Some("test-token")
        );
        assert_eq!(parsed.server.as_deref(), Some("HN1"));
        assert!(!format!("{:?}", parsed.redacted()).contains("test-token"));
    }
}
