//! Native Windows system-cleaning operations that CleanerML cannot express with
//! file or registry actions: emptying the clipboard and the recycle bin.
//!
//! Ports `Windows.empty_recycle_bin` / `get_clipboard_paths`-style clearing from
//! BleachBit's `System` cleaner (which are Python `Command.Function`s upstream).

use windows::core::PCWSTR;
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard};
use windows::Win32::UI::Shell::{
    SHEmptyRecycleBinW, SHQueryRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHERB_NOSOUND, SHQUERYRBINFO,
};

/// Empty the desktop clipboard. Mirrors BleachBit clearing the clipboard's
/// contents so copied text/files no longer linger in memory.
pub fn empty_clipboard() -> Result<(), String> {
    unsafe {
        OpenClipboard(None).map_err(|e| format!("OpenClipboard: {e}"))?;
        let res = EmptyClipboard().map_err(|e| format!("EmptyClipboard: {e}"));
        let _ = CloseClipboard();
        res
    }
}

/// Bytes currently held in the recycle bin across all drives (preview size).
/// Returns 0 if the query fails (e.g. nothing recycled).
pub fn recycle_bin_size() -> u64 {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        i64Size: 0,
        i64NumItems: 0,
    };
    // PCWSTR::null() => query every drive's recycle bin.
    let hr = unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) };
    if hr.is_ok() && info.i64Size > 0 {
        info.i64Size as u64
    } else {
        0
    }
}

/// Empty the recycle bin on all drives, silently. An "already empty" result is
/// not treated as an error.
pub fn empty_recycle_bin() -> Result<(), String> {
    let flags = SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI | SHERB_NOSOUND;
    match unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), flags) } {
        Ok(()) => Ok(()),
        // E_UNEXPECTED (0x8000FFFF) is returned when the bin is already empty.
        Err(e) if e.code().0 as u32 == 0x8000_FFFF => Ok(()),
        Err(e) => Err(format!("SHEmptyRecycleBin failed: {e}")),
    }
}
