//! CHIP-8 模拟器入口：解析命令行、创建窗口、驱动 60 FPS 主循环。
//!
//! 主循环每帧（1/60 秒）做五件事：
//! 1. 同步宿主按键到模拟器的十六进制小键盘
//! 2. 按设定速度执行若干条指令
//! 3. 递减 60Hz 定时器，驱动蜂鸣器
//! 4. 把显存渲染到窗口
//! 5. 控制帧率，保持 60 FPS

// 模拟器代码中存在大量"有意为之"的整数窄化转换与模块同名类型，
// 关闭相关 pedantic 检查
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::similar_names,
    clippy::module_name_repetitions
)]

mod audio;
mod chip8;
mod display;
mod font;
mod keypad;
mod memory;

use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;
use std::thread::sleep;
use std::time::{Duration, Instant};

use minifb::{Key, Window, WindowOptions};

use crate::audio::Beeper;
use crate::chip8::{Chip8, Quirks};
use crate::display::{HEIGHT, WIDTH};
use crate::memory::PROG_START;

/// 默认执行速度（条指令/秒），接近原机 500-1000 条/秒的水平
const DEFAULT_IPS: u32 = 600;
/// 默认放大倍数：64×32 → 640×320 窗口
const DEFAULT_SCALE: usize = 10;
/// 点亮像素（白色）
const COLOR_ON: u32 = 0xFFFF_FFFF;
/// 熄灭像素（黑色）
const COLOR_OFF: u32 = 0xFF00_0000;

/// 宿主按键 → CHIP-8 十六进制小键盘（经典 QWERTY 映射）：
/// ```text
/// 1234 → 123C    QWER → 456D    ASDF → 789E    ZXCV → A0BF
/// ```
const KEY_MAP: [(Key, u8); 16] = [
    (Key::Key1, 0x1),
    (Key::Key2, 0x2),
    (Key::Key3, 0x3),
    (Key::Key4, 0xC),
    (Key::Q, 0x4),
    (Key::W, 0x5),
    (Key::E, 0x6),
    (Key::R, 0xD),
    (Key::A, 0x7),
    (Key::S, 0x8),
    (Key::D, 0x9),
    (Key::F, 0xE),
    (Key::Z, 0xA),
    (Key::X, 0x0),
    (Key::C, 0xB),
    (Key::V, 0xF),
];

fn main() {
    let raw_args: Vec<String> = env::args().skip(1).collect();
    if raw_args.iter().any(|a| a == "-h" || a == "--help") {
        print_usage();
        return;
    }
    let args = match parse_args(&raw_args) {
        Ok(a) => a,
        Err(msg) => {
            eprintln!("参数错误：{msg}\n");
            print_usage();
            std::process::exit(2);
        }
    };
    if let Err(e) = run(&args) {
        eprintln!("运行错误：{e}");
        std::process::exit(1);
    }
}

/// 解析后的启动配置
struct Args {
    rom: Vec<u8>,
    title: String,
    /// 每秒执行的指令条数
    ips: u32,
    /// 窗口像素放大倍数
    scale: usize,
    trace: bool,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut trace = false;
    let mut demo = false;
    let mut pos: Vec<&str> = Vec::new();
    for arg in raw {
        match arg.as_str() {
            "--trace" => trace = true,
            "--demo" => demo = true,
            _ => pos.push(arg.as_str()),
        }
    }

    let (rom, title) = if demo {
        (demo_rom(), "内置演示 CAFE".to_string())
    } else {
        let path = pos
            .first()
            .ok_or("缺少 ROM 路径（也可用 --demo 运行内置演示）")?;
        let rom = fs::read(path).map_err(|e| format!("无法读取 ROM \"{path}\"：{e}"))?;
        let title = Path::new(path)
            .file_name()
            .map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
        (rom, title)
    };

    // --demo 模式下没有 ROM 路径参数，因此速度/倍数参数整体前移一位
    let rest: &[&str] = if demo {
        pos.as_slice()
    } else {
        pos.get(1..).unwrap_or(&[])
    };

    let ips = match rest.first() {
        Some(s) => s
            .parse::<u32>()
            .map_err(|_| format!("\"{s}\" 不是有效的指令速度（整数）"))?,
        None => DEFAULT_IPS,
    };
    if ips == 0 || ips > 100_000 {
        return Err(format!("指令速度须在 1-100000 之间，当前为 {ips}"));
    }

    let scale = match rest.get(1) {
        Some(s) => s
            .parse::<usize>()
            .map_err(|_| format!("\"{s}\" 不是有效的放大倍数（整数）"))?,
        None => DEFAULT_SCALE,
    };
    if scale == 0 || scale > 30 {
        return Err(format!("放大倍数须在 1-30 之间，当前为 {scale}"));
    }

    Ok(Args {
        rom,
        title,
        ips,
        scale,
        trace,
    })
}

fn run(args: &Args) -> Result<(), Box<dyn Error>> {
    let mut chip8 = Chip8::new(Quirks::default());
    chip8.load_rom(&args.rom)?;
    chip8.set_trace(args.trace);

    // 音频设备打不开不影响模拟器运行，只是没有声音
    let mut beeper = Beeper::new();
    if beeper.is_none() {
        eprintln!("提示：无法打开音频设备，将无声运行");
    }

    let (win_w, win_h) = (WIDTH * args.scale, HEIGHT * args.scale);
    let mut window = Window::new(
        &format!("CHIP-8 · {}", args.title),
        win_w,
        win_h,
        WindowOptions::default(),
    )
    .map_err(|e| format!("无法创建窗口：{e}"))?;
    let mut buffer = vec![COLOR_OFF; win_w * win_h];

    // 一帧 1/60 秒：60Hz 定时器与渲染共用这个节拍
    let frame_time = Duration::from_secs_f64(1.0 / 60.0);
    // 每帧指令数可能是小数（ips 不一定是 60 的倍数），用累加器消化
    let steps_per_frame = f64::from(args.ips) / 60.0;
    let mut pending_steps = 0.0;

    loop {
        let frame_start = Instant::now();

        if !window.is_open() || window.is_key_down(Key::Escape) {
            break;
        }

        // 1. 同步宿主按键 → 模拟器小键盘
        for (host_key, chip_key) in KEY_MAP {
            let down = window.is_key_down(host_key);
            chip8.keypad_mut().set(chip_key, down);
        }

        // 2. 执行指令
        pending_steps += steps_per_frame;
        while pending_steps >= 1.0 {
            chip8
                .step()
                .map_err(|e| format!("{e}（PC=0x{:04X}）", chip8.pc()))?;
            pending_steps -= 1.0;
        }

        // 3. 60Hz 定时器与蜂鸣
        chip8.tick_timers();
        if let Some(beeper) = beeper.as_mut() {
            beeper.set_beeping(chip8.sound_timer() > 0);
        }

        // 4. 渲染
        chip8
            .display()
            .render(&mut buffer, args.scale, COLOR_ON, COLOR_OFF);
        window
            .update_with_buffer(&buffer, win_w, win_h)
            .map_err(|e| format!("渲染失败：{e}"))?;

        // 5. 帧率控制（60 FPS）
        if let Some(remaining) = frame_time.checked_sub(frame_start.elapsed()) {
            sleep(remaining);
        }
    }
    Ok(())
}

/// 生成内置演示 ROM：用内置字体在屏幕中央显示 "CAFE"。
/// 让用户无需下载任何 ROM 就能快速验证模拟器是否正常工作。
fn demo_rom() -> Vec<u8> {
    const TEXT: &[u8] = b"CAFE";
    const X0: u8 = 15; // 4 个字形各占 10 像素，整体大致居中
    const Y: u8 = 13;

    let mut prog = Vec::new();
    for (n, &ch) in TEXT.iter().enumerate() {
        let digit = if ch.is_ascii_digit() {
            ch - b'0'
        } else {
            ch - b'A' + 10
        };
        let x = X0 + n as u8 * 10;
        push_op(&mut prog, 0xA000 | font::char_addr(digit)); // LD  I, 字形地址
        push_op(&mut prog, 0x6000 | u16::from(x)); // LD  V0, x
        push_op(&mut prog, 0x6100 | u16::from(Y)); // LD  V1, y
        push_op(&mut prog, 0xD015); // DRW V0, V1, 5 行
    }
    // 末尾跳转到自身：原地停住（相当于 halt）
    let halt_addr = PROG_START + prog.len() as u16;
    push_op(&mut prog, 0x1000 | halt_addr);
    prog
}

/// 把一条指令按大端拆成两个字节写入
fn push_op(prog: &mut Vec<u8>, op: u16) {
    prog.push((op >> 8) as u8);
    prog.push((op & 0xFF) as u8);
}

fn print_usage() {
    println!(
        "\
CHIP-8 模拟器

用法:
  chip8rs <ROM 路径> [每秒指令数] [放大倍数]
  chip8rs --demo [每秒指令数] [放大倍数]

示例:
  chip8rs roms/TETRIS            # 默认 600 条/秒、放大 10 倍
  chip8rs roms/TETRIS 900 8      # 调速 + 更小的窗口
  chip8rs --demo                 # 无需 ROM 文件，运行内置演示（显示 CAFE）

选项:
  --trace    在标准错误流打印每条执行的指令（调试用）
  -h, --help 显示本帮助

按键映射（CHIP-8 小键盘 ← 键盘）:
  1 2 3 C ← 1 2 3 4
  4 5 6 D ← Q W E R
  7 8 9 E ← A S D F
  A 0 B F ← Z X C V
  Esc 退出"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chip8::Quirks;

    #[test]
    fn demo_rom_draws_text() {
        let mut chip8 = Chip8::new(Quirks::default());
        chip8.load_rom(&demo_rom()).unwrap();
        for _ in 0..20 {
            chip8.step().unwrap(); // 16 条绘制指令 + 若干次原地跳转
        }
        // "C" 的首行字形 0xF0 → (15..=18, 13) 四个像素应点亮
        assert!(chip8.display().get(15, 13));
        assert!(chip8.display().get(18, 13));
        assert!(!chip8.display().get(14, 13)); // 字形左侧应为空
    }
}
