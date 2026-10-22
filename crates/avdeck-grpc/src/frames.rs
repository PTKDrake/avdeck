//! Screen frames over the MMAP transport: the client owns a shared memory region and the
//! emulator writes each RGBA frame into it, announcing it on the `streamScreenshot` stream.
//! See `docs/grpc-notes.md` for the measurements behind this.

use crate::control::image_format::ImgFormat;
use crate::control::image_transport::TransportChannel;
use crate::control::{Image, ImageFormat, ImageTransport};
use std::ffi::OsStr;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicU32, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Memory::{
    CreateFileMappingW, FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile,
    PAGE_READWRITE, UnmapViewOfFile,
};

/// One RGBA frame, rows top-down.
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// `SkinRotation` of the device when the frame was taken.
    pub rotation: i32,
    pub seq: u32,
    pub pixels: Vec<u8>,
}

impl Frame {
    /// Every sampled pixel is pure black. During animations emulator 37.2.12 streams every other
    /// frame fully black (opaque), with MMAP and inline transport alike; a real screen almost
    /// never is, so callers hold such frames back.
    pub fn is_black(&self) -> bool {
        let pixels = self.pixels.len() / 4;
        // About a thousand pixels spread over the whole frame, coprime step to avoid a grid.
        let step = (pixels / 1021).max(1);
        (0..pixels)
            .step_by(step)
            .all(|index| self.pixels[index * 4..index * 4 + 3] == [0, 0, 0])
    }
}

/// A shared memory region the emulator writes frames into.
pub struct SharedFrames {
    /// What goes into `ImageTransport.handle`; the mapping itself is `SHM_<name>`.
    name: String,
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    length: usize,
}

// SAFETY: the mapping handle and view are process-wide; the region is only read through `&self`.
unsafe impl Send for SharedFrames {}
unsafe impl Sync for SharedFrames {}

impl SharedFrames {
    /// Room for RGBA frames of `width` x `height` in either orientation.
    pub fn new(width: u32, height: u32) -> io::Result<Self> {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "avdeck-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let length = width as usize * height as usize * 4;
        // On Windows the emulator opens `SHM_<handle>`, not `<handle>` itself.
        let wide: Vec<u16> = OsStr::new(&format!("SHM_{name}"))
            .encode_wide()
            .chain([0])
            .collect();
        let mapping = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                (length as u64 >> 32) as u32,
                length as u32,
                wide.as_ptr(),
            )
        };
        if mapping.is_null() {
            return Err(io::Error::last_os_error());
        }
        let view = unsafe { MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, length) };
        if view.Value.is_null() {
            let error = io::Error::last_os_error();
            unsafe { CloseHandle(mapping) };
            return Err(error);
        }
        Ok(Self {
            name,
            mapping,
            view,
            length,
        })
    }

    /// `streamScreenshot` request for RGBA frames scaled to fit `width` x `height` (0 = native).
    pub fn request(&self, width: u32, height: u32) -> ImageFormat {
        ImageFormat {
            format: ImgFormat::Rgba8888 as i32,
            width,
            height,
            transport: Some(ImageTransport {
                channel: TransportChannel::Mmap as i32,
                handle: self.name.clone(),
            }),
            ..Default::default()
        }
    }

    /// Copies the frame `image` announces. `None` for the empty images the emulator sends while
    /// the display is off, or if the frame does not fit the region.
    pub fn copy(&self, image: &Image) -> Option<Frame> {
        let format = image.format.as_ref()?;
        let length = format.width as usize * format.height as usize * 4;
        if length == 0 {
            return None;
        }
        let pixels = if image.image.is_empty() {
            if length > self.length {
                return None;
            }
            // SAFETY: the view is `self.length` bytes long and stays mapped while `self` lives.
            unsafe { std::slice::from_raw_parts(self.view.Value as *const u8, length) }.to_vec()
        } else {
            // The emulator fell back to sending the pixels inline.
            image.image.get(..length)?.to_vec()
        };
        Some(Frame {
            width: format.width,
            height: format.height,
            rotation: format
                .rotation
                .as_ref()
                .map_or(0, |rotation| rotation.rotation),
            seq: image.seq,
            pixels,
        })
    }
}

impl Drop for SharedFrames {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.view);
            CloseHandle(self.mapping);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(pixels: Vec<u8>) -> Frame {
        Frame {
            width: 40,
            height: 50,
            rotation: 0,
            seq: 0,
            pixels,
        }
    }

    #[test]
    fn opaque_black_is_black_whatever_the_alpha() {
        let black = [0, 0, 0, 255].repeat(2000);
        assert!(frame(black).is_black());
    }

    #[test]
    fn a_dark_screen_with_a_status_bar_is_not_black() {
        let mut pixels = [0, 0, 0, 255].repeat(2000);
        // One lit row of 40 pixels at the top, like status bar icons.
        for pixel in pixels.chunks_mut(4).take(40) {
            pixel.copy_from_slice(&[200, 200, 200, 255]);
        }
        assert!(!frame(pixels).is_black());
    }
}
