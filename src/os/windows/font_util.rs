use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Instant;

use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWriteCreateFactory, IDWriteFactory3, IDWriteLocalFontFileLoader,
};
use windows::core::*;

use anyhow::Result;

pub fn load_system_font_paths() -> Result<Vec<PathBuf>> {
    let timer = Instant::now();
    let factory: IDWriteFactory3 = unsafe { DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)? };

    let mut paths: HashSet<PathBuf> = HashSet::new();

    // preallocate variables and buffers
    let mut reference_key_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut reference_key_size: u32 = 0;

    let mut font_file_path_length: u32;

    // (for future me: yes, allocating and reusing one buffer with capacity 32767
    // is faster than allocating ~200 buffers with capacity ~20)
    let mut font_file_path_buffer: Vec<u16> = Vec::with_capacity(32767);

    let font_set = unsafe { factory.GetSystemFontSet()? };
    let count = unsafe { font_set.GetFontCount() };

    for i in 0..count {
        unsafe {
            let font_face = font_set.GetFontFaceReference(i)?;
            let font_file = font_face.GetFontFile()?;
            let loader = font_file.GetLoader()?;
            let local_loader = loader.cast::<IDWriteLocalFontFileLoader>();
            if local_loader.is_err() {
                continue;
            }
            let local_loader = local_loader.unwrap();

            font_file.GetReferenceKey(&mut reference_key_ptr, &mut reference_key_size)?;

            font_file_path_length =
                local_loader.GetFilePathLengthFromKey(reference_key_ptr, reference_key_size)?;

            // returned length does not include NULL-terminator
            // (https://learn.microsoft.com/en-us/windows/win32/api/dwrite/nf-dwrite-idwritelocalfontfileloader-getfilepathlengthfromkey)
            font_file_path_length += 1;

            font_file_path_buffer.set_len(font_file_path_length as usize);
            local_loader.GetFilePathFromKey(
                reference_key_ptr,
                reference_key_size,
                &mut font_file_path_buffer,
            )?;

            font_file_path_buffer.set_len(font_file_path_length as usize);

            // path may be NULL-terminated
            if let Some(0) = font_file_path_buffer.last() {
                font_file_path_buffer.set_len(font_file_path_length as usize - 1);
            }
        }

        let file_path = String::from_utf16_lossy(&font_file_path_buffer);

        paths.insert(file_path.into());
    }
    eprintln!(
        "found {} system font paths in {} micros",
        paths.len(),
        timer.elapsed().as_micros()
    );
    Ok(paths.into_iter().collect())
}
