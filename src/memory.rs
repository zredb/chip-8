//! CHIP-8 主内存：4 KB 地址空间（0x000-0x0FFF）。
//!
//! - 0x000-0x1FF：保留区（解释器 + 字体，程序不应使用）
//! - 0x200-0xFFF：程序区，ROM 加载到 0x200 处

use crate::font::{FONT, FONT_ADDR};

/// 地址空间大小（字节）
pub const MEM_SIZE: usize = 4096;

/// ROM 加载与执行起始地址
pub const PROG_START: u16 = 0x200;

pub struct Memory {
    data: [u8; MEM_SIZE],
}

impl Memory {
    /// 全零内存 + 载入内置字体
    #[must_use]
    pub fn new() -> Self {
        let mut mem = Self {
            data: [0; MEM_SIZE],
        };
        mem.load_font();
        mem
    }

    /// 把内置字体复制到 0x050 处
    fn load_font(&mut self) {
        let base = usize::from(FONT_ADDR);
        self.data[base..base + FONT.len()].copy_from_slice(&FONT);
    }

    /// 读取一个字节。
    ///
    /// 地址自动掩码为 12 位：CHIP-8 地址总线只有 12 根，
    /// 超出 0xFFF 的地址在原机上会回绕，这里保持一致。
    #[must_use]
    pub fn read(&self, addr: u16) -> u8 {
        self.data[usize::from(addr & 0x0FFF)]
    }

    /// 从 `addr` 读取一条大端指令（2 字节）
    #[must_use]
    pub fn read16(&self, addr: u16) -> u16 {
        let hi = u16::from(self.read(addr));
        let lo = u16::from(self.read(addr.wrapping_add(1)));
        hi << 8 | lo
    }

    /// 写入一个字节（同样做 12 位掩码）
    pub fn write(&mut self, addr: u16, value: u8) {
        self.data[usize::from(addr & 0x0FFF)] = value;
    }

    /// 把 ROM 复制到 0x200 起始的程序区
    ///
    /// # Errors
    /// ROM 超过程序区最大容量（3584 字节）时返回错误
    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), String> {
        let start = usize::from(PROG_START);
        let max = MEM_SIZE - start;
        if rom.len() > max {
            return Err(format!("ROM 过大：{} 字节（上限 {max} 字节）", rom.len()));
        }
        self.data[start..start + rom.len()].copy_from_slice(rom);
        Ok(())
    }
}

impl Default for Memory {
    fn default() -> Self {
        Self::new()
    }
}
