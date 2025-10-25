use std::path::{Path, PathBuf};

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_ISOLATED, DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory,
    IDWriteFontCollection,
};
use windows::Win32::System::Registry;
use windows::Win32::System::SystemInformation;
use windows::core::*;

use anyhow::Result;

pub fn load_system_font_paths() -> Result<Vec<PathBuf>> {
    let mut results = load_system_font_paths_for_registry_root(Registry::HKEY_LOCAL_MACHINE)?;
    results.extend(load_system_font_paths_for_registry_root(
        Registry::HKEY_CURRENT_USER,
    )?);
    Ok(results)
}

fn load_system_font_paths_for_registry_root(root_key: Registry::HKEY) -> Result<Vec<PathBuf>> {
    unsafe {
        let font_registry_path = w!("Software\\Microsoft\\Windows NT\\CurrentVersion\\Fonts");

        let mut hkey: Registry::HKEY = Default::default();

        let mut result = Registry::RegOpenKeyExW(
            root_key,
            font_registry_path,
            None,
            Registry::KEY_READ,
            &mut hkey,
        );

        if result != ERROR_SUCCESS {
            eprintln!("could not open HKEY_LOCAL_MACHINE: {}", result.0);
            return Ok(Vec::new());
        }

        let paths = load_system_font_paths_for_registry_key(hkey, None);

        let result = unsafe { Registry::RegCloseKey(hkey) };

        if result != ERROR_SUCCESS {
            panic!(
                "COULD NOT CLOSE REGISTRY KEY HANDLE. IS THIS BAD? I DON'T KNOW BUT I DON'T WANT TO FIND OUT. SEEYA!"
            );
        }

        paths
    }
}

fn load_system_font_paths_for_registry_key(
    key: Registry::HKEY,
    windows_font_path: Option<&Path>,
) -> Result<Vec<PathBuf>> {
    let mut nr_of_subkeys: u32 = 0;
    let mut max_subkey_name_size: u32 = 0;
    let mut nr_of_values: u32 = 0;
    let mut max_value_name_size: u32 = 0;
    let mut max_value_data_size: u32 = 0;

    let mut result: WIN32_ERROR = WIN32_ERROR::default();

    let mut paths: Vec<PathBuf> = Vec::new();

    let windows_font_path = if let Some(p) = windows_font_path {
        p
    } else {
        &get_windows_directory().join("Fonts")
    };

    result = unsafe {
        Registry::RegQueryInfoKeyW(
            key,
            None,
            None,
            None,
            Some(&mut nr_of_subkeys),
            Some(&mut max_subkey_name_size),
            None,
            Some(&mut nr_of_values),
            Some(&mut max_value_name_size),
            Some(&mut max_value_data_size),
            None,
            None,
        )
    };

    if result != ERROR_SUCCESS {
        eprintln!("Could not query reg info: {}", result.0);
        return Ok(Vec::new());
    }

    // returned max lengths of strings do not include NULL-terminator
    max_subkey_name_size += 1;
    max_value_name_size += 1;

    if nr_of_subkeys > 0 {
        let mut subkey_name_buffer: Vec<u16> = Vec::with_capacity(max_subkey_name_size as usize);

        for i in 0..nr_of_subkeys {
            let mut subkey_name_size = max_subkey_name_size;
            unsafe {
                subkey_name_buffer.set_len(max_subkey_name_size as usize);
                result = Registry::RegEnumKeyExW(
                    key,
                    i,
                    Some(PWSTR(subkey_name_buffer.as_mut_ptr())),
                    &mut subkey_name_size,
                    None,
                    None,
                    None,
                    None,
                );
                subkey_name_buffer.set_len(subkey_name_size as usize);
            }

            if result == ERROR_NO_MORE_ITEMS {
                break;
            }

            if result != ERROR_SUCCESS {
                eprintln!("Could not enum key: {}", result.0);
                return Ok(Vec::new());
            }

            let mut subkey: Registry::HKEY = Registry::HKEY::default();

            result = unsafe {
                Registry::RegOpenKeyExW(
                    key,
                    PCWSTR(subkey_name_buffer.as_mut_ptr()),
                    None,
                    Registry::KEY_READ,
                    &mut subkey,
                )
            };

            if result != ERROR_SUCCESS {
                eprintln!("Could not open enum key: {}", result.0);
                return Ok(Vec::new());
            }

            if let Ok(recursive_result) =
                load_system_font_paths_for_registry_key(subkey, Some(windows_font_path))
            {
                paths.extend(recursive_result);
            }

            result = unsafe { Registry::RegCloseKey(subkey) };

            if result != ERROR_SUCCESS {
                panic!(
                    "COULD NOT CLOSE REGISTRY KEY HANDLE. IS THIS BAD? I DON'T KNOW BUT I DON'T WANT TO FIND OUT. SEEYA!"
                );
            }
        }
    }

    if !windows_font_path.exists() || !windows_font_path.is_dir() {
        eprintln!(
            "\"Windows\" directory found at {} either does not exist or is not an actual directory.",
            windows_font_path.to_string_lossy()
        );
        return Ok(Vec::new());
    }

    if nr_of_values > 0 {
        let mut value_name_buffer: Vec<u16> = Vec::with_capacity(max_value_name_size as usize);
        let mut value_data_buffer: Vec<u16> = Vec::with_capacity(max_value_data_size as usize);
        let data_buffer_pointer: *mut u8 = value_data_buffer.as_mut_ptr() as *mut u8;
        let mut value_type: Registry::REG_VALUE_TYPE = Registry::REG_VALUE_TYPE::default();

        for i in 0..nr_of_values {
            unsafe {
                value_name_buffer.set_len(max_value_name_size as usize);
                value_data_buffer.set_len(max_value_data_size as usize);
                let mut value_name_size = max_value_name_size;
                let mut value_data_size = max_value_data_size;

                result = Registry::RegEnumValueW(
                    key,
                    i,
                    Some(PWSTR(value_name_buffer.as_mut_ptr())),
                    &mut value_name_size,
                    None,
                    Some(&mut value_type.0),
                    Some(data_buffer_pointer),
                    Some(&mut value_data_size),
                );

                if result != ERROR_SUCCESS {
                    eprintln!("Not success: {}", result.0);
                    break;
                }
                if value_type != Registry::REG_SZ {
                    continue;
                }

                value_name_buffer.set_len(value_name_size as usize);
                // value_data_buffer contains u16 values, but data size is calculated for u8 data
                value_data_buffer.set_len((value_data_size / 2) as usize);

                // data *may* be NULL-terminated
                if value_data_buffer.len() > 0
                    && value_data_buffer[value_data_buffer.len() - 1] == 0
                {
                    value_data_buffer.set_len(value_data_buffer.len() - 1);
                }
            }

            let value = String::from_utf16_lossy(&value_data_buffer);

            let mut path = PathBuf::from(value);

            if path.is_relative() {
                path = windows_font_path.join(path);
            }

            if path.exists()
                && path.is_file()
                && let Some(ext) = path.extension()
            {
                let extension = ext.to_string_lossy().to_ascii_lowercase();
                match extension.as_str() {
                    "ttf" | "ttc" => paths.push(path),
                    _ => {}
                }
            }
        }
    }

    Ok(paths)
}

fn get_windows_directory() -> PathBuf {
    let mut path_buffer: Vec<u16> = Vec::with_capacity(MAX_PATH as usize);

    unsafe {
        path_buffer.set_len(MAX_PATH as usize);

        let length = SystemInformation::GetWindowsDirectoryW(Some(&mut path_buffer));
        path_buffer.set_len(length as usize);
    }

    PathBuf::from(String::from_utf16_lossy(&path_buffer))
}
