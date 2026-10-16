//! 蜂鸣器：声音定时器 > 0 时播放 440 Hz 提示音。
//!
//! 连接音频设备失败时返回 `None`，模拟器以"无声模式"继续运行，
//! 不会因为机器没有音频设备而崩溃。

use rodio::source::SineWave;

/// 蜂鸣频率（CHIP-8 只有一个单调蜂鸣器，频率随意，取标准音 A4）
const BEEP_FREQ: f32 = 440.0;
/// 音量（0.0-1.0，取低一些避免刺耳）
const VOLUME: f32 = 0.25;

pub struct Beeper {
    /// 设备句柄必须存活整个播放期，否则音频流会被关闭
    _device: rodio::MixerDeviceSink,
    player: rodio::Player,
    beeping: bool,
}

impl Beeper {
    /// 连接系统默认音频输出。失败返回 `None`（无声运行）。
    #[must_use]
    pub fn new() -> Option<Self> {
        let device = rodio::DeviceSinkBuilder::open_default_sink().ok()?;
        let player = rodio::Player::connect_new(device.mixer());
        // 挂上一条无限播放的音源，之后靠暂停/恢复实现"开关"
        player.append(SineWave::new(BEEP_FREQ));
        player.set_volume(VOLUME);
        player.pause();
        Some(Self {
            _device: device,
            player,
            beeping: false,
        })
    }

    /// 依据声音定时器是否 > 0 开/关蜂鸣。
    ///
    /// CHIP-8 只有"响/不响"两种状态，因此用暂停一个持续音源
    /// 的方式实现，避免反复创建音频流。
    pub fn set_beeping(&mut self, on: bool) {
        if self.beeping == on {
            return;
        }
        self.beeping = on;
        if on {
            self.player.play();
        } else {
            self.player.pause();
        }
    }
}
