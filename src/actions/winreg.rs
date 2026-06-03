//! Windows registry cleaning and shell-change notification.
//!
//! Ports `Windows.delete_registry_value`, `Windows.delete_registry_key`
//! (CleanerML always passes an empty exclude list, so that branch is dropped),
//! `Windows.split_registry_key`, and `Windows.shell_change_notify`.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, HKEY,
    HKEY_CLASSES_ROOT, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_READ, KEY_SET_VALUE,
};
use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};

/// Split `HKLM\Software\Foo` into a predefined hive handle and the sub-key path.
fn split_key(full: &str) -> Result<(HKEY, &str), String> {
    let (root, sub) = full
        .split_once('\\')
        .ok_or_else(|| format!("invalid registry key: {full}"))?;
    let hive = match root {
        "HKCR" => HKEY_CLASSES_ROOT,
        "HKCU" => HKEY_CURRENT_USER,
        "HKLM" => HKEY_LOCAL_MACHINE,
        "HKU" => HKEY_USERS,
        other => return Err(format!("invalid registry hive '{other}'")),
    };
    Ok((hive, sub))
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether the key exists (preview for a key deletion).
pub fn key_exists(keyname: &str) -> bool {
    let Ok((hive, sub)) = split_key(keyname) else {
        return false;
    };
    let wide = to_wide(sub);
    let mut hkey = HKEY::default();
    unsafe {
        let rc = RegOpenKeyExW(hive, PCWSTR(wide.as_ptr()), 0, KEY_READ, &mut hkey);
        if rc == ERROR_SUCCESS {
            let _ = RegCloseKey(hkey);
            true
        } else {
            false
        }
    }
}

/// Whether a named value exists under the key (preview for a value deletion).
pub fn value_exists(keyname: &str, valuename: &str) -> bool {
    let Ok((hive, sub)) = split_key(keyname) else {
        return false;
    };
    let wide_sub = to_wide(sub);
    let wide_val = to_wide(valuename);
    let mut hkey = HKEY::default();
    unsafe {
        if RegOpenKeyExW(hive, PCWSTR(wide_sub.as_ptr()), 0, KEY_READ, &mut hkey) != ERROR_SUCCESS {
            return false;
        }
        let rc = RegQueryValueExW(hkey, PCWSTR(wide_val.as_ptr()), None, None, None, None);
        let _ = RegCloseKey(hkey);
        rc == ERROR_SUCCESS
    }
}

/// Delete a registry key and everything under it. Returns whether it existed.
pub fn delete_key(keyname: &str) -> Result<bool, String> {
    if !key_exists(keyname) {
        return Ok(false);
    }
    let (hive, sub) = split_key(keyname)?;
    let wide = to_wide(sub);
    unsafe {
        // RegDeleteTreeW with a non-null subkey removes the subkey and all of its
        // descendants — equivalent to BleachBit's recursive delete.
        let rc = RegDeleteTreeW(hive, PCWSTR(wide.as_ptr()));
        if rc != ERROR_SUCCESS {
            return Err(format!("RegDeleteTreeW failed ({}) for {keyname}", rc.0));
        }
    }
    Ok(true)
}

/// Delete a single named value under a key. Returns whether it existed.
pub fn delete_value(keyname: &str, valuename: &str) -> Result<bool, String> {
    if !value_exists(keyname, valuename) {
        return Ok(false);
    }
    let (hive, sub) = split_key(keyname)?;
    let wide_sub = to_wide(sub);
    let wide_val = to_wide(valuename);
    let mut hkey = HKEY::default();
    unsafe {
        if RegOpenKeyExW(hive, PCWSTR(wide_sub.as_ptr()), 0, KEY_SET_VALUE, &mut hkey)
            != ERROR_SUCCESS
        {
            return Err(format!("cannot open {keyname} for write"));
        }
        let rc = RegDeleteValueW(hkey, PCWSTR(wide_val.as_ptr()));
        let _ = RegCloseKey(hkey);
        if rc != ERROR_SUCCESS {
            return Err(format!("RegDeleteValueW failed ({})", rc.0));
        }
    }
    Ok(true)
}

/// Tell the Windows shell that file associations changed (refresh Explorer).
pub fn shell_change_notify() {
    unsafe {
        SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None);
    }
}
