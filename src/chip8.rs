//! CHIP-8 CPU 核心：寄存器组 + 全部 35 条指令的译码与执行。
//!
//! 一条指令 2 字节（大端）。指令格式：`IIXY N NNN`
//! - `X`、`Y`：寄存器号（0-15，即 V0-VF）
//! - `NN`：8 位立即数，`NNN`：12 位地址，`N`：4 位立即数
//!
//! 关于"方言"（Quirks）：CHIP-8 从没有官方标准，几条指令在不同
//! 机器上行为不同，详见 [`Quirks`]。

use std::fmt;

use crate::display::Display;
use crate::font;
use crate::keypad::Keypad;
use crate::memory::{Memory, PROG_START};

/// 调用栈深度（原机为 16 层）
const STACK_SIZE: usize = 16;

/// 指令集"方言"开关。
///
/// CHIP-8 没有官方规范，少数指令在不同实现上行为不同。
/// 这里给出兼容性最好的默认值，个别 ROM 需要调整后才能正确运行。
#[derive(Clone, Copy)]
pub struct Quirks {
    /// 8XY6/8XYE 移位：
    /// `true`  = `Vx = Vy >> 1`（原版 COSMAC VIP 行为）
    /// `false` = `Vx` 自身移位（Cowgod 参考手册 / 现代模拟器常见行为）
    pub shift_uses_vy: bool,
    /// FX55/FX65 读写寄存器组：
    /// `true`  = 结束后 `I += X + 1`（原版行为）
    /// `false` = `I` 保持不变（SCHIP 行为）
    pub load_store_increments_i: bool,
    /// DXYN 绘制精灵：
    /// `true`  = 越出屏幕环绕到对侧（原版行为）
    /// `false` = 越界部分被裁剪
    pub sprite_wraps: bool,
}

impl Default for Quirks {
    fn default() -> Self {
        Self {
            shift_uses_vy: false,
            load_store_increments_i: true,
            sprite_wraps: true,
        }
    }
}

/// CPU 执行错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chip8Error {
    /// 遇到无法识别的指令（括号内为指令编码）
    UnknownOpcode(u16),
    /// CALL 嵌套超过 16 层
    StackOverflow,
    /// 栈空时执行 RET
    StackUnderflow,
}

impl fmt::Display for Chip8Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownOpcode(op) => write!(f, "未识别的指令 0x{op:04X}"),
            Self::StackOverflow => write!(f, "调用栈溢出（超过 {STACK_SIZE} 层）"),
            Self::StackUnderflow => write!(f, "栈空时执行 RET（00EE）"),
        }
    }
}

impl std::error::Error for Chip8Error {}

/// 一台完整的 CHIP-8：CPU 寄存器 + 内存 + 显存 + 键盘。
pub struct Chip8 {
    memory: Memory,
    display: Display,
    keypad: Keypad,
    /// 数据寄存器 V0-VF（VF 兼作标志位）
    v: [u8; 16],
    /// 地址寄存器 I
    i: u16,
    /// 程序计数器（12 位有效）
    pc: u16,
    /// 调用栈
    stack: [u16; STACK_SIZE],
    /// 栈指针（0-16，既是栈内元素数也是下一个空位下标）
    sp: u8,
    /// 延迟定时器（60Hz 递减）
    delay_timer: u8,
    /// 声音定时器（60Hz 递减，>0 时蜂鸣）
    sound_timer: u8,
    quirks: Quirks,
    trace: bool,
}

impl Chip8 {
    /// 新建一台复位的机器（字体已载入内存，PC 指向 0x200）
    #[must_use]
    pub fn new(quirks: Quirks) -> Self {
        Self {
            memory: Memory::new(),
            display: Display::new(),
            keypad: Keypad::new(),
            v: [0; 16],
            i: 0,
            pc: PROG_START,
            stack: [0; STACK_SIZE],
            sp: 0,
            delay_timer: 0,
            sound_timer: 0,
            quirks,
            trace: false,
        }
    }

    /// 加载 ROM 并复位 CPU（程序从 0x200 开始执行）
    ///
    /// # Errors
    /// ROM 超过程序区容量（3584 字节）时返回错误
    pub fn load_rom(&mut self, rom: &[u8]) -> Result<(), String> {
        self.memory.load_rom(rom)?;
        self.v = [0; 16];
        self.i = 0;
        self.pc = PROG_START;
        self.stack = [0; STACK_SIZE];
        self.sp = 0;
        self.delay_timer = 0;
        self.sound_timer = 0;
        Ok(())
    }

    /// 取指并执行一条指令。
    ///
    /// # Errors
    /// 遇到未知指令或栈操作越界时返回错误；此时 PC 保持指向出错指令。
    pub fn step(&mut self) -> Result<(), Chip8Error> {
        let pc = self.pc;
        let op = self.memory.read16(pc);
        if self.trace {
            eprintln!("pc=0x{pc:04X} op=0x{op:04X}");
        }
        self.pc = (pc + 2) & 0x0FFF;
        if let Err(e) = self.execute(op) {
            // 出错时回退 PC，方便定位出错指令
            self.pc = pc;
            return Err(e);
        }
        Ok(())
    }

    /// 定时器递减。需以 60 Hz 调用（由主循环驱动）。
    pub fn tick_timers(&mut self) {
        self.delay_timer = self.delay_timer.saturating_sub(1);
        self.sound_timer = self.sound_timer.saturating_sub(1);
    }

    /// 跳过下一条指令（SE/SNE/SKP 系列使用）
    fn skip_next(&mut self) {
        self.pc = self.pc.wrapping_add(2) & 0x0FFF;
    }

    /// 指令译码与执行。35 条指令按最高 4 位分派。
    ///
    // 译码器本质上就是一张大表，强行拆分反而难读；
    // x、y、a、b 等单字符名与 CHIP-8 指令集的字段命名（V0-VF、X、Y）一致
    #[allow(clippy::too_many_lines, clippy::many_single_char_names)]
    fn execute(&mut self, op: u16) -> Result<(), Chip8Error> {
        let x = ((op >> 8) & 0x0F) as usize;
        let y = ((op >> 4) & 0x0F) as usize;
        let nn = (op & 0xFF) as u8;
        let nnn = op & 0x0FFF;

        match op >> 12 {
            // 0xxx：清屏 / 子程序返回 / 机器码调用
            0x0 => match op & 0x0FFF {
                // 00E0 CLS：清屏
                0x0E0 => self.display.clear(),
                // 00EE RET：返回子程序调用点
                0x0EE => {
                    if self.sp == 0 {
                        return Err(Chip8Error::StackUnderflow);
                    }
                    self.sp -= 1;
                    self.pc = self.stack[usize::from(self.sp)];
                }
                // 0NNN：跳转到 RCA 1802 机器码，现代 ROM 不使用，忽略
                _ => {}
            },
            // 1NNN JP：无条件跳转
            0x1 => self.pc = nnn,
            // 2NNN CALL：调用子程序（压栈）
            0x2 => {
                if usize::from(self.sp) >= STACK_SIZE {
                    return Err(Chip8Error::StackOverflow);
                }
                self.stack[usize::from(self.sp)] = self.pc;
                self.sp += 1;
                self.pc = nnn;
            }
            // 3XNN SE：Vx == NN 则跳过
            0x3 => {
                if self.v[x] == nn {
                    self.skip_next();
                }
            }
            // 4XNN SNE：Vx != NN 则跳过
            0x4 => {
                if self.v[x] != nn {
                    self.skip_next();
                }
            }
            // 5XY0 SE：Vx == Vy 则跳过
            0x5 => match op & 0x0F {
                0x0 => {
                    if self.v[x] == self.v[y] {
                        self.skip_next();
                    }
                }
                _ => return Err(Chip8Error::UnknownOpcode(op)),
            },
            // 6XNN LD：Vx = NN
            0x6 => self.v[x] = nn,
            // 7XNN ADD：Vx += NN（不影响标志位）
            0x7 => self.v[x] = self.v[x].wrapping_add(nn),
            // 8XY_ 算术逻辑运算
            0x8 => {
                let (a, b) = (self.v[x], self.v[y]);
                match op & 0x0F {
                    0x0 => self.v[x] = b,     // LD   Vx, Vy
                    0x1 => self.v[x] = a | b, // OR   Vx, Vy
                    0x2 => self.v[x] = a & b, // AND  Vx, Vy
                    0x3 => self.v[x] = a ^ b, // XOR  Vx, Vy
                    0x4 => {
                        // ADD  Vx, Vy（VF = 进位）
                        let (r, carry) = a.overflowing_add(b);
                        self.v[x] = r;
                        self.v[0xF] = u8::from(carry);
                    }
                    0x5 => {
                        // SUB  Vx, Vy（VF = 无借位）
                        let (r, borrow) = a.overflowing_sub(b);
                        self.v[x] = r;
                        self.v[0xF] = u8::from(!borrow);
                    }
                    0x6 => {
                        // SHR：VF = 最低位，Vx >>= 1
                        let src = if self.quirks.shift_uses_vy { b } else { a };
                        self.v[0xF] = src & 1;
                        self.v[x] = src >> 1;
                    }
                    0x7 => {
                        // SUBN Vx, Vy：Vx = Vy - Vx（VF = 无借位）
                        let (r, borrow) = b.overflowing_sub(a);
                        self.v[x] = r;
                        self.v[0xF] = u8::from(!borrow);
                    }
                    0xE => {
                        // SHL：VF = 最高位，Vx <<= 1
                        let src = if self.quirks.shift_uses_vy { b } else { a };
                        self.v[0xF] = src >> 7;
                        self.v[x] = src << 1;
                    }
                    _ => return Err(Chip8Error::UnknownOpcode(op)),
                }
            }
            // 9XY0 SNE：Vx != Vy 则跳过
            0x9 => match op & 0x0F {
                0x0 => {
                    if self.v[x] != self.v[y] {
                        self.skip_next();
                    }
                }
                _ => return Err(Chip8Error::UnknownOpcode(op)),
            },
            // ANNN LD：I = NNN
            0xA => self.i = nnn,
            // BNNN JP：PC = NNN + V0（原版基于 V0）
            0xB => self.pc = (nnn + u16::from(self.v[0])) & 0x0FFF,
            // CXNN RND：Vx = 随机数 & NN
            0xC => self.v[x] = rand::random::<u8>() & nn,
            // DXYN DRW：在 (Vx, Vy) 绘制 N 行精灵，VF = 碰撞标志
            0xD => {
                let rows = (op & 0x0F) as usize;
                // 精灵数据最多 15 字节，先拷出来避免同一帧内反复借用内存
                let mut sprite = [0u8; 15];
                for (k, cell) in sprite.iter_mut().enumerate().take(rows) {
                    *cell = self.memory.read(self.i + k as u16);
                }
                let collided = self.display.draw_sprite(
                    &sprite[..rows],
                    usize::from(self.v[x]),
                    usize::from(self.v[y]),
                    self.quirks.sprite_wraps,
                );
                self.v[0xF] = u8::from(collided);
            }
            // EXxx：按键跳转
            0xE => match op & 0x0FF {
                // EX9E SKP：键 Vx 按下则跳过
                0x9E => {
                    if self.keypad.is_down(self.v[x]) {
                        self.skip_next();
                    }
                }
                // EXA1 SKNP：键 Vx 未按下则跳过
                0xA1 => {
                    if !self.keypad.is_down(self.v[x]) {
                        self.skip_next();
                    }
                }
                _ => return Err(Chip8Error::UnknownOpcode(op)),
            },
            // FXxx：杂项
            0xF => match op & 0x0FF {
                // FX07：Vx = 延迟定时器
                0x07 => self.v[x] = self.delay_timer,
                // FX0A：等待按键。没有键按下时回退 PC 原地重试，
                // 从而"阻塞"程序直到有输入。
                0x0A => match self.keypad.first_down() {
                    Some(key) => self.v[x] = key,
                    None => self.pc = self.pc.wrapping_sub(2) & 0x0FFF,
                },
                // FX15：延迟定时器 = Vx
                0x15 => self.delay_timer = self.v[x],
                // FX18：声音定时器 = Vx
                0x18 => self.sound_timer = self.v[x],
                // FX1E：I += Vx（12 位地址，回绕）
                0x1E => self.i = (self.i + u16::from(self.v[x])) & 0x0FFF,
                // FX29：I = 字形 Vx 的地址
                0x29 => self.i = font::char_addr(self.v[x]),
                // FX33：把 Vx 存为 BCD（百/十/个位 → I、I+1、I+2）
                0x33 => {
                    let vx = self.v[x];
                    self.memory.write(self.i, vx / 100);
                    self.memory.write(self.i + 1, vx / 10 % 10);
                    self.memory.write(self.i + 2, vx % 10);
                }
                // FX55：把 V0..Vx 存入 I 开始的内存
                0x55 => {
                    for k in 0..=x {
                        self.memory.write(self.i + k as u16, self.v[k]);
                    }
                    if self.quirks.load_store_increments_i {
                        self.i += x as u16 + 1;
                    }
                }
                // FX65：从 I 开始的内存读入 V0..Vx
                0x65 => {
                    for k in 0..=x {
                        self.v[k] = self.memory.read(self.i + k as u16);
                    }
                    if self.quirks.load_store_increments_i {
                        self.i += x as u16 + 1;
                    }
                }
                _ => return Err(Chip8Error::UnknownOpcode(op)),
            },
            _ => unreachable!("op >> 12 的范围为 0-15，上面的 match 已全部覆盖"),
        }
        Ok(())
    }

    /// 当前程序计数器（供错误信息与调试）
    #[must_use]
    pub fn pc(&self) -> u16 {
        self.pc
    }

    /// 声音定时器是否 > 0（蜂鸣器驱动）
    #[must_use]
    pub fn sound_timer(&self) -> u8 {
        self.sound_timer
    }

    /// 借用显存（渲染用）
    #[must_use]
    pub fn display(&self) -> &Display {
        &self.display
    }

    /// 可变借用键盘（同步宿主输入用）
    pub fn keypad_mut(&mut self) -> &mut Keypad {
        &mut self.keypad
    }

    /// 开关指令跟踪（在标准错误流打印每条执行的指令）
    pub fn set_trace(&mut self, trace: bool) {
        self.trace = trace;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 载入程序并执行 `steps` 条指令
    fn run(prog: &[u8], steps: usize) -> Chip8 {
        let mut chip8 = Chip8::new(Quirks::default());
        chip8.load_rom(prog).unwrap();
        for _ in 0..steps {
            chip8.step().unwrap();
        }
        chip8
    }

    // ---- 跳转 / 调用 ----

    #[test]
    fn jump_sets_pc() {
        let c = run(&[0x12, 0x34], 1);
        assert_eq!(c.pc, 0x234);
    }

    #[test]
    fn call_and_return() {
        // 0x200: 2204 CALL 0x204 → 0x204: 0000 空操作 → 0x206: 00EE RET
        let mut c = run(&[0x22, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0xEE], 3);
        assert_eq!(c.pc, 0x202); // 返回到 CALL 的下一条
        c.step().unwrap(); // 再走一步也不该出错
        assert_eq!(c.pc, 0x204);
    }

    #[test]
    fn call_stack_overflow_is_detected() {
        // 2200: CALL 0x200（调用自身）→ 无限压栈
        let mut c = run(&[0x22, 0x00], 16);
        assert_eq!(c.step(), Err(Chip8Error::StackOverflow));
    }

    #[test]
    fn ret_on_empty_stack_is_detected() {
        let mut c = Chip8::new(Quirks::default());
        c.load_rom(&[0x00, 0xEE]).unwrap();
        assert_eq!(c.step(), Err(Chip8Error::StackUnderflow));
    }

    #[test]
    fn jump_with_v0_offset() {
        // 6005: V0 = 5 → B200: PC = 0x200 + 5
        let c = run(&[0x60, 0x05, 0xB2, 0x00], 2);
        assert_eq!(c.pc, 0x205);
    }

    // ---- 条件跳过 ----

    #[test]
    fn skip_if_equal_and_not_equal() {
        // 6005: V0=5 → 3005: V0==5，跳过 → PC 越过 0x204
        let c = run(&[0x60, 0x05, 0x30, 0x05], 2);
        assert_eq!(c.pc, 0x206);
        // 4006: V0!=6，跳过
        let c = run(&[0x60, 0x05, 0x40, 0x06], 2);
        assert_eq!(c.pc, 0x206);
        // 4005: V0==5，不跳过
        let c = run(&[0x60, 0x05, 0x40, 0x05], 2);
        assert_eq!(c.pc, 0x204);
        // 5XY0: V0==V1，跳过
        let c = run(&[0x60, 0x05, 0x61, 0x05, 0x50, 0x10], 3);
        assert_eq!(c.pc, 0x208);
        // 9XY0: V0!=V1，跳过
        let c = run(&[0x60, 0x05, 0x61, 0x06, 0x90, 0x10], 3);
        assert_eq!(c.pc, 0x208);
    }

    #[test]
    fn skip_on_keypress() {
        let mut c = run(&[0x60, 0x05, 0xE0, 0x9E], 1); // 只执行 6005，等设置好按键
        assert_eq!(c.pc, 0x202);
        c.keypad_mut().set(0x5, true);
        c.step().unwrap(); // EX9E：键 5 按下 → 跳过下一条
        assert_eq!(c.pc, 0x206); // 0x204 + 2
    }

    // ---- 算术逻辑 ----

    #[test]
    fn add_immediate_wraps_without_flag() {
        // V0 = 0xFF + 0x01 → 0x00，VF 不变
        let c = run(&[0x60, 0xFF, 0x70, 0x01], 2);
        assert_eq!(c.v[0], 0);
        assert_eq!(c.v[0xF], 0);
    }

    #[test]
    fn add_registers_sets_carry() {
        // V0 = 0xFF, V1 = 0x01 → V0 = 0, VF = 1（进位）
        let c = run(&[0x60, 0xFF, 0x61, 0x01, 0x80, 0x14], 3);
        assert_eq!(c.v[0], 0);
        assert_eq!(c.v[0xF], 1);
    }

    #[test]
    fn sub_sets_borrow_flag() {
        // V0 = 1 - 2 = 0xFF，VF = 0（有借位）
        let c = run(&[0x60, 0x01, 0x61, 0x02, 0x80, 0x15], 3);
        assert_eq!(c.v[0], 0xFF);
        assert_eq!(c.v[0xF], 0);
    }

    #[test]
    fn subn_computes_y_minus_x() {
        // SUBN：V0 = V1 - V0 = 2 - 1 = 1，VF = 1（无借位）
        let c = run(&[0x60, 0x01, 0x61, 0x02, 0x80, 0x17], 3);
        assert_eq!(c.v[0], 1);
        assert_eq!(c.v[0xF], 1);
    }

    #[test]
    fn shift_defaults_to_in_place() {
        // 默认 quirk：V0 自身右移。0x03 → 0x01，VF = 1（最低位）
        let c = run(&[0x60, 0x03, 0x80, 0x06], 2);
        assert_eq!(c.v[0], 1);
        assert_eq!(c.v[0xF], 1);
    }

    #[test]
    fn shift_uses_vy_with_quirk() {
        // 打开原版 quirk：V0 = V1 >> 1 = 3 >> 1 = 1
        // 8016 = SHR V0, V1（X=0, Y=1, 操作码 6）
        let quirks = Quirks {
            shift_uses_vy: true,
            ..Quirks::default()
        };
        let mut c = Chip8::new(quirks);
        c.load_rom(&[0x61, 0x03, 0x80, 0x16]).unwrap();
        c.step().unwrap();
        c.step().unwrap();
        assert_eq!(c.v[0], 1);
        assert_eq!(c.v[1], 3);
        assert_eq!(c.v[0xF], 1);
    }

    #[test]
    fn shift_left_sets_msb_flag() {
        // 0x80 << 1 = 0x00，VF = 1（最高位溢出）
        let c = run(&[0x60, 0x80, 0x80, 0x0E], 2);
        assert_eq!(c.v[0], 0);
        assert_eq!(c.v[0xF], 1);
    }

    // ---- 显示 ----

    #[test]
    fn draw_and_redraw_detects_collision() {
        // D012 = DRW V0, V1, 2 行（X=0, Y=1, N=2）
        let mut c = run(
            &[0xA3, 0x00, 0x60, 0x05, 0x61, 0x06, 0xD0, 0x12, 0xD0, 0x12],
            3,
        );
        // 精灵放到 0x300：两行 0xF0、0xAA
        c.memory.write(0x300, 0xF0);
        c.memory.write(0x301, 0xAA);
        // 第一次绘制：点亮 (5..=8, 6)，无碰撞
        c.step().unwrap(); // D002
        assert!(c.display.get(5, 6));
        assert!(c.display.get(8, 6));
        assert_eq!(c.v[0xF], 0);
        // 再画一次：XOR 抹掉全部像素 → 碰撞
        c.step().unwrap(); // D002
        assert!(!c.display.get(5, 6));
        assert_eq!(c.v[0xF], 1);
    }

    #[test]
    fn cls_clears_screen() {
        // D011 = DRW V0, V1, 1 行（X=0, Y=1, N=1）
        let mut c = run(&[0xA3, 0x00, 0x60, 0x00, 0x61, 0x00, 0xD0, 0x11], 3);
        c.memory.write(0x300, 0xFF);
        c.step().unwrap(); // 绘制
        assert!(c.display.get(0, 0));
        c.memory.write(0x208, 0x00);
        c.memory.write(0x209, 0xE0);
        c.step().unwrap(); // 00E0 清屏
        assert!(!c.display.get(0, 0));
    }

    // ---- 定时器 / 内存 / I-O ----

    #[test]
    fn timers_decrement_to_zero() {
        let mut c = run(&[0x60, 0x03, 0xF0, 0x15, 0xF0, 0x18], 3);
        assert_eq!(c.delay_timer, 3);
        assert_eq!(c.sound_timer(), 3);
        c.tick_timers();
        assert_eq!(c.delay_timer, 2);
        c.tick_timers();
        c.tick_timers();
        c.tick_timers(); // 多 tick 一次不应下溢
        assert_eq!(c.delay_timer, 0);
    }

    #[test]
    fn fx0a_blocks_until_key_down() {
        let mut c = run(&[0xF0, 0x0A], 1);
        assert_eq!(c.pc, 0x200); // 无按键 → 原地等待
        c.keypad_mut().set(0x5, true);
        c.step().unwrap();
        assert_eq!(c.v[0], 5);
        assert_eq!(c.pc, 0x202);
    }

    #[test]
    fn bcd_conversion() {
        // V0 = 0x8E = 142 → 百十个位 1、4、2
        let c = run(&[0x60, 0x8E, 0xA3, 0x00, 0xF0, 0x33], 3);
        assert_eq!(c.memory.read(0x300), 1);
        assert_eq!(c.memory.read(0x301), 4);
        assert_eq!(c.memory.read(0x302), 2);
    }

    #[test]
    fn fx55_stores_registers_and_increments_i() {
        // 原版 quirk：存完 V0..Vx 后 I += x+1
        // F155 = FX55 存 V0..V1；F165 = FX65 读回 V0..V1
        let c = run(&[0x60, 0x11, 0x61, 0x22, 0xA3, 0x00, 0xF1, 0x55], 4);
        assert_eq!(c.memory.read(0x300), 0x11);
        assert_eq!(c.memory.read(0x301), 0x22);
        assert_eq!(c.i, 0x302);
        // 关闭 quirk 时 I 不动（改用 FX65 测读回）
        let quirks = Quirks {
            load_store_increments_i: false,
            ..Quirks::default()
        };
        let mut c = Chip8::new(quirks);
        c.load_rom(&[0xA3, 0x00, 0xF1, 0x65]).unwrap();
        c.memory.write(0x300, 0xAA);
        c.memory.write(0x301, 0xBB);
        c.step().unwrap();
        c.step().unwrap();
        assert_eq!(c.v[0], 0xAA);
        assert_eq!(c.v[1], 0xBB);
        assert_eq!(c.i, 0x300);
    }

    #[test]
    fn fx1e_adds_to_i() {
        let c = run(&[0xA0, 0x00, 0x60, 0x10, 0xF0, 0x1E], 3);
        assert_eq!(c.i, 0x010);
    }

    #[test]
    fn fx29_points_to_font() {
        let c = run(&[0x60, 0x03, 0xF0, 0x29], 2);
        assert_eq!(c.i, font::FONT_ADDR + 15);
        assert_eq!(c.memory.read(c.i), 0xF0); // 字形 "3" 的首行
    }

    #[test]
    fn font_is_loaded_at_boot() {
        let c = Chip8::new(Quirks::default());
        assert_eq!(c.memory.read(font::FONT_ADDR), 0xF0); // "0"
        assert_eq!(c.memory.read(font::char_addr(1)), 0x20); // "1"
    }

    #[test]
    fn unknown_opcode_is_reported() {
        let mut c = Chip8::new(Quirks::default());
        c.load_rom(&[0xF0, 0x99]).unwrap();
        assert_eq!(c.step(), Err(Chip8Error::UnknownOpcode(0xF099)));
        assert_eq!(c.pc, 0x200); // PC 回退到出错指令
    }

    #[test]
    fn random_masks_with_nn() {
        // CX00：无论随机数多少，结果一定是 0
        let c = run(&[0xC0, 0x00], 1);
        assert_eq!(c.v[0], 0);
    }

    #[test]
    fn sys_call_is_ignored() {
        // 0NNN 应为空操作，不报错
        let c = run(&[0x02, 0x00], 1);
        assert_eq!(c.pc, 0x202);
    }
}
