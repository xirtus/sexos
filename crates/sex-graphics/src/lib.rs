#![no_std]

pub mod font;
pub mod qoi;

pub use font::draw_str;
pub use font::draw_char;

use sex_pdx::Rect;

// Defines a simple WindowBuffer for drawing operations.
#[derive(Clone, Copy)]
pub struct WindowBuffer {
    pub virtual_address: *mut u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32, // Stride in pixels
}

impl WindowBuffer {
    pub unsafe fn new(virtual_address: u64, width: u32, height: u32, stride: u32) -> Self {
        Self {
            virtual_address: virtual_address as *mut u32,
            width,
            height,
            stride,
        }
    }

    /// Draws a single pixel at (x, y) with the given color.
    pub unsafe fn draw_pixel(&mut self, x: u32, y: u32, color: u32) {
        if x < self.width && y < self.height {
            *self.virtual_address.add((y * self.stride + x) as usize) = color;
        }
    }

    /// Safely draws a single pixel at (x, y) with the given color, returning 0 if out of bounds.
    pub unsafe fn draw_pixel_safe(&mut self, x: u32, y: u32, color: u32) -> u32 {
        if x < self.width && y < self.height {
            *self.virtual_address.add((y * self.stride + x) as usize) = color;
            color
        } else {
            0 // Or some other error indicator
        }
    }

    /// Safely reads a single pixel at (x, y), returning 0 if out of bounds.
    pub unsafe fn read_pixel_safe(&self, x: u32, y: u32) -> u32 {
        if x < self.width && y < self.height {
            *self.virtual_address.add((y * self.stride + x) as usize)
        } else {
            0 // Or some other default/error color
        }
    }

    /// Draws a filled rectangle using a Rect struct.
    pub unsafe fn draw_rect(&mut self, rect: Rect, color: u32) {
        let start_x = rect.x.max(0) as u32;
        let start_y = rect.y.max(0) as u32;
        let end_x = (rect.x as i32 + rect.width as i32).max(0) as u32;
        let end_y = (rect.y as i32 + rect.height as i32).max(0) as u32;

        for row in start_y..end_y.min(self.height) {
            for col in start_x..end_x.min(self.width) {
                self.draw_pixel(col, row, color);
            }
        }
    }

    /// Fills the entire buffer with a single color.
    pub unsafe fn clear(&mut self, color: u32) {
        for y in 0..self.height {
            for x in 0..self.width {
                self.draw_pixel(x, y, color);
            }
        }
    }
}
