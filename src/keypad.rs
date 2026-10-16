//! 16 键十六进制小键盘（0x0-0xF）的实时状态。
//!
//! 模拟器核心只关心"哪个键按下/抬起"，与宿主机具体按键无关；
//! 宿主键映射（QWERTY → 十六进制小键盘）在 main.rs 中完成。

/// 按键数量
pub const KEY_COUNT: usize = 16;

#[derive(Clone)]
pub struct Keypad {
    down: [bool; KEY_COUNT],
}

impl Keypad {
    #[must_use]
    pub fn new() -> Self {
        Self {
            down: [false; KEY_COUNT],
        }
    }

    /// 查询某个键是否按下。键号来自寄存器，可能 > 0xF，
    /// 与原机一致按低 4 位处理。
    #[must_use]
    pub fn is_down(&self, key: u8) -> bool {
        self.down[usize::from(key & 0x0F)]
    }

    /// 设置某键状态（由宿主输入循环调用）
    pub fn set(&mut self, key: u8, down: bool) {
        self.down[usize::from(key & 0x0F)] = down;
    }

    /// 返回当前按下的键中编号最小的一个；全部抬起时返回 `None`。
    /// 供 FX0A（等待按键）使用。
    #[must_use]
    pub fn first_down(&self) -> Option<u8> {
        self.down.iter().position(|&d| d).map(|i| i as u8)
    }
}

impl Default for Keypad {
    fn default() -> Self {
        Self::new()
    }
}
