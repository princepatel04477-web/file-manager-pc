//! Windows shell verbs used by the context menu: open, open with, reveal, properties.

use crate::error::AppError;
#[cfg(windows)]
use crate::ops;
use std::path::Path;

#[cfg(windows)]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Open with the file's default handler.
pub fn open(path: &Path) -> Result<(), AppError> {
    verb(path, "open")
}

/// Show the Windows "Open with" picker for this file.
pub fn open_with(path: &Path) -> Result<(), AppError> {
    verb(path, "openas")
}

/// Show the Windows properties sheet for this item.
pub fn properties(path: &Path) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let file = wide(&ops::display_path(path));
        let verb_name = wide("properties");
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_FLAG_NO_UI,
            lpVerb: PCWSTR(verb_name.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        unsafe { ShellExecuteExW(&mut info) }.map_err(|_| AppError::Unavailable)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(AppError::Unavailable)
    }
}

/// Select the item in Windows Explorer, opening its parent folder.
pub fn reveal(path: &Path) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let program = wide("explorer.exe");
        let open_verb = wide("open");
        let arguments = wide(&format!("/select,\"{}\"", ops::display_path(path)));
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(open_verb.as_ptr()),
                PCWSTR(program.as_ptr()),
                PCWSTR(arguments.as_ptr()),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(AppError::Unavailable);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(AppError::Unavailable)
    }
}

fn verb(path: &Path, verb_name: &str) -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::UI::Shell::ShellExecuteW;
        use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

        let verb = wide(verb_name);
        let target = wide(&ops::display_path(path));
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(target.as_ptr()),
                PCWSTR::null(),
                PCWSTR::null(),
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize <= 32 {
            return Err(AppError::Unavailable);
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (path, verb_name);
        Err(AppError::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_verbs_fail_cleanly_off_windows() {
        // On non-Windows hosts (including CI) these must return an error, never panic.
        let result = reveal(Path::new("/tmp/does-not-matter.txt"));
        if cfg!(windows) {
            let _ = result;
        } else {
            assert!(matches!(result, Err(AppError::Unavailable)));
            assert!(matches!(open(Path::new("/tmp/x")), Err(AppError::Unavailable)));
            assert!(matches!(open_with(Path::new("/tmp/x")), Err(AppError::Unavailable)));
            assert!(matches!(properties(Path::new("/tmp/x")), Err(AppError::Unavailable)));
        }
    }
}
