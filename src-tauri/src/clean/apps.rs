//! Unused apps: reading the Uninstall keys and turning the raw registry values into
//! something the UI can show. Every parsing rule is a pure function so it is unit
//! tested; only the registry reads themselves are Windows-only.

use crate::error::AppError;
use serde::Serialize;
use specta::Type;

/// A row straight out of an Uninstall key — before any cleaning or filtering.
/// The fields mirror the registry 1:1, so a few of them are only read on Windows.
#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct RawApp {
    pub name: String,
    pub display_name: String,
    pub publisher: String,
    pub version: String,
    pub install_date: String,
    pub uninstall_string: String,
    pub quiet_uninstall_string: String,
    pub icon_location: String,
    pub system_component: u32,
    pub parent_key_name: String,
    /// `EstimatedSize`, in kilobytes, as an unparsed string.
    pub estimated_size: String,
    pub is_64bit: bool,
    pub is_current_user: bool,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct InstalledApp {
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// yyyy-MM-dd when the registry gave a usable date.
    pub install_date: Option<String>,
    pub size_bytes: u64,
    pub uninstall_command: Option<UninstallCommand>,
    /// True when the entry came from `HKCU` (per-user install).
    pub per_user: bool,
    pub source: AppSource,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct UninstallCommand {
    pub executable: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum AppSource {
    /// 64-bit view: `SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall`.
    Machine64,
    /// 32-bit view under `WOW6432Node`.
    Machine32,
    /// `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall`.
    CurrentUser,
}

/// Names Windows uses for update and runtime sub-entries that should not be offered
/// to the user as removable apps.
const HIDDEN_NAME_MARKERS: &[&str] = &[
    "update for",
    "hotfix for",
    "security update for",
    "cumulative update for",
    ".net framework",
    "microsoft visual c++ 2",
    "microsoft edge update",
    "windows sdk",
    "windows software development kit",
    "redistributable",
];

/// An entry is hidden when Windows flagged it as a system component, when it has no
/// display name, when there is nothing to run to remove it, or when it is a patch
/// registered against another product.
pub fn is_hidden(app: &RawApp) -> bool {
    if app.system_component != 0 {
        return true;
    }
    if app.display_name.trim().is_empty() {
        return true;
    }
    if app.uninstall_string.trim().is_empty() && app.quiet_uninstall_string.trim().is_empty() {
        return true;
    }
    // `ParentKeyName` marks an update installed on top of another product.
    if !app.parent_key_name.is_empty() {
        return true;
    }
    let name = app.display_name.to_lowercase();
    HIDDEN_NAME_MARKERS.iter().any(|marker| name.contains(marker))
}

/// `20260924` → `2026-09-24`. Anything else is `None` rather than a guess.
pub fn parse_install_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let digits: String = trimmed.chars().filter(char::is_ascii_digit).collect();
    if digits.len() == 8 {
        let year = &digits[0..4];
        let month = &digits[4..6];
        let day = &digits[6..8];
        let month_value: u32 = month.parse().ok()?;
        let day_value: u32 = day.parse().ok()?;
        if (1..=12).contains(&month_value) && (1..=31).contains(&day_value) {
            return Some(format!("{year}-{month}-{day}"));
        }
        return None;
    }
    if trimmed.is_empty() {
        return None;
    }
    // Some entries store a locale-formatted date; pass it through untouched.
    Some(trimmed.to_owned())
}

/// `EstimatedSize` is documented as kilobytes; a handful of writers put bytes in it.
/// Anything that is not a plain number is treated as unknown.
pub fn parse_estimated_size_bytes(raw: &str) -> u64 {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let Ok(value) = trimmed.parse::<u64>() else { return 0 };
    // Heuristic guard: above ~1 TiB the writer almost certainly meant bytes.
    if value > 1_099_511_627_776 / 1024 {
        return value;
    }
    value.saturating_mul(1024)
}

/// Split an `UninstallString` into the executable and its arguments, honouring quotes.
/// Returns `None` when there is nothing to run.
pub fn split_uninstall_command(raw: &str) -> Option<UninstallCommand> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let bytes: Vec<char> = trimmed.chars().collect();
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut index = 0;
    while index < bytes.len() {
        let character = bytes[index];
        match character {
            '"' => quoted = !quoted,
            ' ' if !quoted => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(character),
        }
        index += 1;
    }
    if !current.is_empty() {
        parts.push(current);
    }
    let mut iterator = parts.into_iter();
    let executable = iterator.next()?;
    Some(UninstallCommand { executable, arguments: iterator.collect() })
}

/// Turn a raw registry row into the shape the UI renders.
pub fn to_installed_app(raw: &RawApp) -> InstalledApp {
    let uninstall = split_uninstall_command(if raw.uninstall_string.trim().is_empty() {
        raw.quiet_uninstall_string.as_str()
    } else {
        raw.uninstall_string.as_str()
    });
    InstalledApp {
        name: raw.display_name.trim().to_owned(),
        publisher: raw.publisher.trim().to_owned(),
        version: raw.version.trim().to_owned(),
        install_date: parse_install_date(&raw.install_date),
        size_bytes: parse_estimated_size_bytes(&raw.estimated_size),
        uninstall_command: uninstall,
        per_user: raw.is_current_user,
        source: if raw.is_current_user {
            AppSource::CurrentUser
        } else if raw.is_64bit {
            AppSource::Machine64
        } else {
            AppSource::Machine32
        },
    }
}

/// Deduplicate the same app registered in more than one hive/view, then sort by size
/// (largest first, unknown sizes last) and name.
pub fn dedupe_and_sort(mut apps: Vec<InstalledApp>) -> Vec<InstalledApp> {
    apps.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| right.size_bytes.cmp(&left.size_bytes))
            .then_with(|| left.publisher.cmp(&right.publisher))
    });
    apps.dedup_by(|left, right| {
        left.name.to_lowercase() == right.name.to_lowercase() && left.size_bytes == right.size_bytes
    });
    apps.sort_by(|left, right| {
        match (left.size_bytes, right.size_bytes) {
            (0, 0) => std::cmp::Ordering::Equal,
            (0, _) => std::cmp::Ordering::Greater,
            (_, 0) => std::cmp::Ordering::Less,
            _ => right.size_bytes.cmp(&left.size_bytes),
        }
        .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    apps
}

const UNINSTALL_PATHS: [(&str, AppSource); 3] = [
    (r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall", AppSource::Machine64),
    (r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall", AppSource::Machine32),
    (r"Software\Microsoft\Windows\CurrentVersion\Uninstall", AppSource::CurrentUser),
];

#[cfg(windows)]
pub fn installed_apps() -> Result<Vec<InstalledApp>, AppError> {
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY,
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
        REG_OPTION_NON_VOLATILE, REG_SZ,
    };

    fn to_wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn read_string(root: HKEY, sub_key: &str, name: &str) -> String {
        unsafe {
            let mut handle = HKEY::default();
            let sub_key = to_wide(sub_key);
            let opened = RegOpenKeyExW(root, PCWSTR(sub_key.as_ptr()), 0, KEY_READ, &mut handle);
            if opened.is_err() {
                return String::new();
            }
            let value_name = to_wide(name);
            let mut buffer = vec![0_u16; 1024];
            let mut size = (buffer.len() * 2) as u32;
            let mut kind = REG_SZ;
            let queried = RegQueryValueExW(
                handle,
                PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut kind),
                Some(buffer.as_mut_ptr() as *mut u8),
                Some(&mut size),
            );
            let _ = RegCloseKey(handle);
            if queried.is_err() {
                return String::new();
            }
            let length = (size as usize / 2).saturating_sub(1);
            String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
                .trim_end_matches('\0')
                .to_owned()
        }
    }

    fn read_dword(root: HKEY, sub_key: &str, name: &str) -> u32 {
        unsafe {
            let mut handle = HKEY::default();
            let sub_key = to_wide(sub_key);
            let opened = RegOpenKeyExW(root, PCWSTR(sub_key.as_ptr()), 0, KEY_READ, &mut handle);
            if opened.is_err() {
                return 0;
            }
            let value_name = to_wide(name);
            let mut value: u32 = 0;
            let mut size = std::mem::size_of::<u32>() as u32;
            let queried = RegQueryValueExW(
                handle,
                PCWSTR(value_name.as_ptr()),
                None,
                None,
                Some(&mut value as *mut u32 as *mut u8),
                Some(&mut size),
            );
            let _ = RegCloseKey(handle);
            if queried.is_err() { 0 } else { value }
        }
    }

    fn sub_keys(root: HKEY, path: &str, flags: windows::Win32::System::Registry::REG_SAM_FLAGS) -> Vec<String> {
        unsafe {
            let mut handle = HKEY::default();
            let wide = to_wide(path);
            let opened = RegOpenKeyExW(
                root,
                PCWSTR(wide.as_ptr()),
                REG_OPTION_NON_VOLATILE.0,
                KEY_READ | flags,
                &mut handle,
            );
            if opened.is_err() {
                return Vec::new();
            }
            let mut found = Vec::new();
            let mut index = 0_u32;
            loop {
                let mut buffer = vec![0_u16; 256];
                let mut length = buffer.len() as u32;
                let enumerated = RegEnumKeyExW(
                    handle,
                    index,
                    PWSTR(buffer.as_mut_ptr()),
                    &mut length,
                    None,
                    PWSTR::null(),
                    None,
                );
                if enumerated.is_err() {
                    break;
                }
                let name = String::from_utf16_lossy(&buffer[..length as usize])
                    .trim_end_matches('\0')
                    .to_owned();
                if !name.is_empty() {
                    found.push(name);
                }
                index += 1;
            }
            let _ = RegCloseKey(handle);
            found
        }
    }

    let mut raw_apps: Vec<RawApp> = Vec::new();
    for (path, source) in UNINSTALL_PATHS {
        let hive = match source {
            AppSource::CurrentUser => HKEY_CURRENT_USER,
            _ => HKEY_LOCAL_MACHINE,
        };
        let flags = match source {
            AppSource::Machine64 => KEY_WOW64_64KEY,
            AppSource::Machine32 => KEY_WOW64_32KEY,
            AppSource::CurrentUser => KEY_WOW64_64KEY,
        };
        for key in sub_keys(hive, path, flags) {
            let full = format!("{path}\\{key}");
            raw_apps.push(RawApp {
                name: key,
                display_name: read_string(hive, &full, "DisplayName"),
                publisher: read_string(hive, &full, "Publisher"),
                version: read_string(hive, &full, "DisplayVersion"),
                install_date: read_string(hive, &full, "InstallDate"),
                uninstall_string: read_string(hive, &full, "UninstallString"),
                quiet_uninstall_string: read_string(hive, &full, "QuietUninstallString"),
                icon_location: read_string(hive, &full, "DisplayIcon"),
                system_component: read_dword(hive, &full, "SystemComponent"),
                parent_key_name: read_string(hive, &full, "ParentKeyName"),
                estimated_size: read_dword(hive, &full, "EstimatedSize").to_string(),
                is_64bit: !matches!(source, AppSource::Machine32),
                is_current_user: matches!(source, AppSource::CurrentUser),
            });
        }
    }

    Ok(dedupe_and_sort(
        raw_apps
            .iter()
            .filter(|app| !is_hidden(app))
            .map(to_installed_app)
            .collect(),
    ))
}

#[cfg(not(windows))]
pub fn installed_apps() -> Result<Vec<InstalledApp>, AppError> {
    // The registry does not exist here; the card reports nothing rather than erroring.
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(display_name: &str, uninstall: &str) -> RawApp {
        RawApp {
            name: display_name.to_owned(),
            display_name: display_name.to_owned(),
            publisher: "Acme".to_owned(),
            version: "1.0".to_owned(),
            install_date: "20260101".to_owned(),
            uninstall_string: uninstall.to_owned(),
            quiet_uninstall_string: String::new(),
            icon_location: String::new(),
            system_component: 0,
            parent_key_name: String::new(),
            estimated_size: "2048".to_owned(),
            is_64bit: true,
            is_current_user: false,
        }
    }

    #[test]
    fn registry_dates_become_iso_and_junk_is_rejected() {
        assert_eq!(parse_install_date("20260924").as_deref(), Some("2026-09-24"));
        assert_eq!(parse_install_date(" 20251231 ").as_deref(), Some("2025-12-31"));
        assert_eq!(parse_install_date("20261301"), None, "month 13 does not exist");
        assert_eq!(parse_install_date("2026090"), None, "too short to be a date");
        assert_eq!(parse_install_date(""), None);
        assert_eq!(parse_install_date("12/05/2025").as_deref(), Some("12/05/2025"), "locale strings pass through");
    }

    #[test]
    fn estimated_size_is_kilobytes() {
        assert_eq!(parse_estimated_size_bytes("2048"), 2_097_152);
        assert_eq!(parse_estimated_size_bytes("0"), 0);
        assert_eq!(parse_estimated_size_bytes(""), 0);
        assert_eq!(parse_estimated_size_bytes("not-a-number"), 0);
    }

    #[test]
    fn uninstall_commands_keep_quoted_paths_intact() {
        let command = split_uninstall_command(r#""C:\Program Files\Acme App\unins000.exe" /SILENT /LANG=en"#).expect("command");
        assert_eq!(command.executable, r"C:\Program Files\Acme App\unins000.exe");
        assert_eq!(command.arguments, vec!["/SILENT".to_owned(), "/LANG=en".to_owned()]);

        let plain = split_uninstall_command("MsiExec.exe /X{1234-ABCD}").expect("command");
        assert_eq!(plain.executable, "MsiExec.exe");
        assert_eq!(plain.arguments, vec!["/X{1234-ABCD}".to_owned()]);

        assert!(split_uninstall_command("").is_none());
        assert!(split_uninstall_command("   ").is_none());
    }

    #[test]
    fn system_components_and_unremovable_entries_are_hidden() {
        let mut component = raw("Windows Component", "uninstall.exe");
        component.system_component = 1;
        assert!(is_hidden(&component));

        let unnamed = raw("", "uninstall.exe");
        assert!(is_hidden(&unnamed));

        let stuck = raw("No Way Out", "");
        assert!(is_hidden(&stuck), "no uninstall string means nothing to run");

        let mut patch = raw("Update for Acme", "uninstall.exe");
        patch.parent_key_name = "Acme".to_owned();
        assert!(is_hidden(&patch));
    }

    #[test]
    fn a_normal_user_app_is_not_hidden() {
        let app = raw("Handbrake", r#""C:\Program Files\Handbrake\uninstall.exe""#);
        assert!(!is_hidden(&app));
        let converted = to_installed_app(&app);
        assert_eq!(converted.name, "Handbrake");
        assert_eq!(converted.install_date.as_deref(), Some("2026-01-01"));
        assert_eq!(converted.size_bytes, 2_048 * 1024);
        assert!(!converted.per_user);
        assert!(matches!(converted.source, AppSource::Machine64));
    }

    #[test]
    fn quiet_uninstall_is_used_when_the_normal_one_is_missing() {
        let mut app = raw("Quiet Only", "");
        app.quiet_uninstall_string = "C:\\Apps\\Quiet\\remove.exe /Q".to_owned();
        assert!(!is_hidden(&app), "a quiet uninstaller still counts as removable");
        let converted = to_installed_app(&app);
        let command = converted.uninstall_command.expect("command");
        assert_eq!(command.executable, r"C:\Apps\Quiet\remove.exe");
        assert_eq!(command.arguments, vec!["/Q".to_owned()]);
    }

    #[test]
    fn the_same_app_in_two_hives_appears_once() {
        let sixty_four = to_installed_app(&raw("Blender", "uninstall.exe"));
        let mut thirty_two = to_installed_app(&raw("Blender", "uninstall.exe"));
        thirty_two.is_64bit = false;
        thirty_two.source = AppSource::Machine32;
        let list = dedupe_and_sort(vec![sixty_four, thirty_two]);
        assert_eq!(list.len(), 1, "the duplicate hive entry is dropped");
    }

    #[test]
    fn the_biggest_apps_come_first_and_unknown_sizes_come_last() {
        let small = to_installed_app(&RawApp { display_name: "Small".into(), estimated_size: "1024".into(), ..raw("Small", "u.exe") });
        let big = to_installed_app(&RawApp { display_name: "Big".into(), estimated_size: "1024000".into(), ..raw("Big", "u.exe") });
        let unknown = to_installed_app(&RawApp { display_name: "Unknown".into(), estimated_size: String::new(), ..raw("Unknown", "u.exe") });
        let list = dedupe_and_sort(vec![unknown.clone(), small, big]);
        assert_eq!(list[0].name, "Big");
        assert_eq!(list[1].name, "Small");
        assert_eq!(list[list.len() - 1].name, "Unknown");
        assert!(list[0].size_bytes > list[1].size_bytes);
    }
}
