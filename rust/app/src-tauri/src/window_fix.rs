use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::RECT;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetSystemMetrics, GetWindowPlacement, SendMessageW, SetWindowPos,
    SM_CXSCREEN, SM_CYSCREEN, SWP_SHOWWINDOW, SW_SHOWMINIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT,
    WM_DPICHANGED,
};

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Requests elevation and launches `--fix-client-window <zoom>`.
pub fn launch_elevated(zoom: f64) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let operation = wide("runas");
    let file = wide(&exe.to_string_lossy());
    let parameters = wide(&format!("--fix-client-window {zoom}"));
    let result = unsafe {
        ShellExecuteW(
            null_mut(),
            operation.as_ptr(),
            file.as_ptr(),
            parameters.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        return Err("elevation was cancelled or failed".into());
    }
    Ok(())
}

/// Runs in the elevated child process and patches the League client window.
pub fn apply(zoom: f64) -> bool {
    let class = wide("RCLIENT");
    let title = wide("League of Legends");
    let cef_class = wide("CefBrowserWindow");
    let window = unsafe { FindWindowW(class.as_ptr(), title.as_ptr()) };
    if window.is_null() {
        return false;
    }
    let cef = unsafe { FindWindowExW(window, null_mut(), cef_class.as_ptr(), null()) };
    if cef.is_null() {
        return false;
    }
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    if unsafe { GetWindowPlacement(window, &mut placement) } == 0 {
        return false;
    }
    if placement.showCmd == SW_SHOWMINIMIZED as u32 {
        return false;
    }

    let dpi = unsafe { GetDpiForWindow(window) } as usize;
    let wparam = (dpi & 0xFFFF) | ((dpi & 0xFFFF) << 16);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    for target in [window, cef] {
        unsafe {
            SendMessageW(
                target,
                WM_DPICHANGED,
                wparam,
                &mut rect as *mut RECT as isize,
            );
        }
    }

    let screen_width = unsafe { GetSystemMetrics(SM_CXSCREEN) };
    let screen_height = unsafe { GetSystemMetrics(SM_CYSCREEN) };
    let width = (1280.0 * zoom) as i32;
    let height = (720.0 * zoom) as i32;
    unsafe {
        SetWindowPos(
            window,
            null_mut(),
            (screen_width - width) / 2,
            (screen_height - height) / 2,
            width,
            height,
            SWP_SHOWWINDOW,
        );
        SetWindowPos(cef, null_mut(), 0, 0, width, height, SWP_SHOWWINDOW);
    }
    true
}
