# CHIP-8 模拟器

用 Rust 实现的完整 CHIP-8 模拟器，带图形窗口（minifb）、音频蜂鸣（rodio）和 27 个单元测试。

![CHIP-8 64x32 screen](https://img.shields.io/badge/screen-64%C3%9732-green) ![Rust 2024](https://img.shields.io/badge/edition-2024-blue) [![crates.io](https://img.shields.io/crates/v/chip8rs.svg)](https://crates.io/crates/chip8rs) [![license](https://img.shields.io/crates/l/chip8rs.svg)](LICENSE-MIT)

## 安装

```bash
cargo install chip8rs
```

或从源码运行：

```bash
git clone https://github.com/zredb/chip-8.git
cd chip-8
cargo run --release -- --demo
```

## 特性

- 全部 35 条 CHIP-8 指令的完整译码与执行
- 4 KB 内存 + 16 个 8 位寄存器 + 16 层调用栈
- 64×32 单色显示（精灵 XOR 绘制、碰撞检测、越界环绕/裁剪可配）
- 16 键十六进制小键盘（映射到 QWERTY）
- 60 Hz 延迟/声音定时器，声音定时器 > 0 时播放蜂鸣
- 可配置的执行速度（每秒指令数）与窗口放大倍数
- "方言"开关（Quirks）：兼容原版 COSMAC VIP 与现代模拟器的差异行为
- 指令跟踪模式（`--trace`），逐条打印执行的指令
- 内置演示 ROM（`--demo`），无需下载 ROM 即可验证模拟器工作正常
- 27 个单元测试覆盖跳转/调用/算术/显示/定时器/内存指令

## 构建与运行

需要 Rust 1.85+（edition 2024）。

```bash
# 运行内置演示（显示 CAFE，用于自检）
cargo run --release -- --demo

# 运行一个 ROM（默认 600 条指令/秒、像素放大 10 倍）
cargo run --release -- path/to/game.ch8

# 调速 + 调整窗口大小
cargo run --release -- path/to/game.ch8 900 12

# 调试：逐条打印执行的指令
cargo run --release -- path/to/game.ch8 --trace

# 测试
cargo test
```

ROM 可以在 GitHub 上搜索 "chip8 roms" 找到大量公开的游戏与测试 ROM（如 Brix、Tetris、Space Invaders、IBM Logo 等）。

## 用法

```
chip8rs <ROM 路径> [每秒指令数] [放大倍数]
chip8rs --demo [每秒指令数] [放大倍数]

选项:
  --trace    在标准错误流打印每条执行的指令（调试用）
  -h, --help 显示帮助
```

### 按键映射

```
CHIP-8 小键盘      键盘
1 2 3 C    ←    1 2 3 4
4 5 6 D    ←    Q W E R
7 8 9 E    ←    A S D F
A 0 B F    ←    Z X C V

Esc 退出
```

## 项目结构

```
src/
├── main.rs      入口：命令行解析、minifb 窗口、60 FPS 主循环
├── chip8.rs     CPU 核心：寄存器组 + 35 条指令译码执行 + 单元测试
├── memory.rs    4 KB 内存（含 12 位地址总线掩码）与 ROM 加载
├── display.rs   64×32 显存、精灵绘制（XOR + 碰撞检测）、放大渲染
├── keypad.rs    16 键十六进制小键盘状态
├── audio.rs     蜂鸣器（rodio 440 Hz 正弦波，设备不可用时无声降级）
└── font.rs      内置字体：0-F 十六进制字符的 5 字节点阵
```

主循环每帧（60 Hz）执行五步：同步按键 → 执行指令 → 递减定时器 → 渲染 → 帧率控制。

## 关于"方言"（Quirks）

CHIP-8 从来没有官方规范，少数指令在不同机器上行为不同。本实现提供 [`Quirks`] 配置（见 `src/chip8.rs`），默认取兼容性最好的组合：

| Quirk | 默认 | 说明 |
|---|---|---|
| `shift_uses_vy` | `false` | `8XY6/8XYE`：默认 Vx 自身移位（现代常见）；`true` 为原版 `Vx = Vy >> 1` |
| `load_store_increments_i` | `true` | `FX55/FX65`：默认结束后 `I += X + 1`（原版）；`false` 为 SCHIP 行为 |
| `sprite_wraps` | `true` | `DXYN`：默认精灵越界环绕到对侧（原版）；`false` 为裁剪 |

个别 ROM 若显示异常，通常是需要调整 quirk。

## 许可证

本项目采用双许可发布：

- [MIT](LICENSE-MIT)
- [Apache-2.0](LICENSE-APACHE)

你可以任选其一使用。版权 Copyright (c) 2026 zredb。

## 参考资料

- [Cowgod's CHIP-8 Technical Reference](http://devernay.free.fr/hacks/chip8/C8TECH10.HTM)
- [CHIP-8 instruction set (Wikipedia)](https://en.wikipedia.org/wiki/CHIP-8)
