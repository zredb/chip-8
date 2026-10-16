//! 64×32 单色显示屏。
//!
//! 显存是一个简单的布尔位图；渲染时按倍率放大写入 RGBA 缓冲。

/// 屏幕宽度（像素）
pub const WIDTH: usize = 64;
/// 屏幕高度（像素）
pub const HEIGHT: usize = 32;

#[derive(Clone)]
pub struct Display {
    pixels: [[bool; WIDTH]; HEIGHT],
}

impl Display {
    #[must_use]
    pub fn new() -> Self {
        Self {
            pixels: [[false; WIDTH]; HEIGHT],
        }
    }

    /// 清屏（指令 00E0 CLS）
    pub fn clear(&mut self) {
        self.pixels = [[false; WIDTH]; HEIGHT];
    }

    /// 读取像素。坐标必须合法，否则 panic。
    ///
    /// 模拟器主程序渲染走 [`Display::render`]，本方法主要供测试
    /// 与调试使用，因此主程序未使用时不算死代码。
    #[cfg_attr(not(test), allow(dead_code))]
    #[must_use]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.pixels[y][x]
    }

    /// 绘制精灵（指令 DXYN）：8 像素宽、`sprite.len()` 像素高。
    ///
    /// 每个置位位对应一次 XOR 翻转；只要有一个原本点亮的像素被翻成
    /// 熄灭，就视为"碰撞"，返回 true（CPU 会据此把 VF 置 1）。
    ///
    /// `wrap`：`true`  = 越出屏幕的部分环绕到对侧（原版行为），
    ///         `false` = 越界部分直接裁掉。
    #[must_use]
    pub fn draw_sprite(&mut self, sprite: &[u8], x: usize, y: usize, wrap: bool) -> bool {
        let mut collided = false;
        for (row, &bits) in sprite.iter().enumerate() {
            let mut py = y + row;
            if py >= HEIGHT {
                if !wrap {
                    break;
                }
                py %= HEIGHT;
            }
            for col in 0..8 {
                if bits & (0x80 >> col) == 0 {
                    continue;
                }
                let mut px = x + col;
                if px >= WIDTH {
                    if !wrap {
                        continue;
                    }
                    px %= WIDTH;
                }
                let pixel = &mut self.pixels[py][px];
                collided |= *pixel;
                *pixel = !*pixel;
            }
        }
        collided
    }

    /// 把显存放大 `scale` 倍渲染到 RGBA 像素缓冲（供 minifb 显示）。
    ///
    /// # Panics
    /// `out.len()` 与屏幕尺寸（放大后）不符时 panic。
    pub fn render(&self, out: &mut [u32], scale: usize, on: u32, off: u32) {
        let stride = WIDTH * scale;
        assert_eq!(
            out.len(),
            stride * HEIGHT * scale,
            "渲染缓冲尺寸与屏幕不匹配"
        );
        for (y, row) in self.pixels.iter().enumerate() {
            for (x, &lit) in row.iter().enumerate() {
                let color = if lit { on } else { off };
                for dy in 0..scale {
                    let start = (y * scale + dy) * stride + x * scale;
                    out[start..start + scale].fill(color);
                }
            }
        }
    }
}

impl Default for Display {
    fn default() -> Self {
        Self::new()
    }
}
