//! 线性插值重采样器（任意输入率 → 任意输出率）。
//!
//! M1-3.2 为采集（设备原生率 → 16k）引入；M1-3.5b 泛化出输出通路
//! （Kokoro 24k → 设备输出率），两处共用同一相位连续实现。
//!
//! 相位连续：全局输出序号 `next_out` + 基址 `base`（carry[0] 的全局输入序号），
//! 分块喂与整段喂结果逐采样一致（跨回调连续性有测试钉死）。
//! 输入率 = 输出率时退化为直通（尾部留 1 采样延迟，可忽略）。

/// 线性插值重采样器
pub struct Resampler {
    ratio: f64,      // 输入率 / 输出率
    carry: Vec<f32>, // 未消费的尾部采样
    base: u64,       // carry[0] 对应的全局输入采样序号
    next_out: u64,   // 下一个输出序号（全局相位，跨回调连续）
}

impl Resampler {
    pub fn new(in_rate: f64, out_rate: f64) -> Self {
        debug_assert!(in_rate > 0.0 && out_rate > 0.0, "采样率必须为正");
        Self {
            ratio: in_rate / out_rate,
            carry: Vec::new(),
            base: 0,
            next_out: 0,
        }
    }

    /// 喂入一段单声道采样，返回本段可产出的重采样采样。
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        self.carry.extend_from_slice(input);
        let n = self.carry.len();
        let mut out = Vec::new();
        if n < 2 || self.ratio <= 0.0 {
            return out;
        }
        loop {
            // 第 next_out 个输出的全局输入位置；插值需要 pos 与 pos+1 两个采样都在 carry 内
            let pos = self.next_out as f64 * self.ratio;
            let idx_global = pos.floor() as u64;
            let idx = idx_global.saturating_sub(self.base) as usize;
            if idx + 1 >= n {
                break;
            }
            let frac = (pos - idx_global as f64) as f32;
            let a = self.carry[idx];
            let b = self.carry[idx + 1];
            out.push(a + (b - a) * frac);
            self.next_out += 1;
        }
        // 消费到最后一个已用输入采样，之前的不再参与后续插值
        if self.next_out > 0 {
            let last_used = ((self.next_out - 1) as f64 * self.ratio).floor() as u64;
            let drop = (last_used + 1 - self.base) as usize;
            self.carry.drain(..drop);
            self.base = last_used + 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 16k→16k 直通：正弦一个周期（16 采样）原样通过，无增益无移位
    #[test]
    fn resample_passthrough_same_rate() {
        let mut r = Resampler::new(16000.0, 16000.0);
        let input: Vec<f32> = (0..16)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 16.0).sin() as f32)
            .collect();
        let out = r.push(&input);
        assert_eq!(out.len(), 15); // 留 1 采样给下一次插值
        for (a, b) in out.iter().zip(input.iter()) {
            assert!((a - b).abs() < 1e-6, "同率直通不应改变采样");
        }
    }

    /// 48k → 16k 三分降采样：输出长度 = 输入/3，能量保持（正弦幅度不塌）
    #[test]
    fn resample_48k_to_16k() {
        let mut r = Resampler::new(48000.0, 16000.0);
        let input: Vec<f32> = (0..4800)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 4800.0 * 8.0).sin() as f32)
            .collect();
        let out = r.push(&input);
        assert_eq!(
            out.len(),
            1600,
            "48k 4800 采样（1s）应产出 1600（0.25s 音频）"
        );
        let peak = out.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
        assert!(
            peak > 0.9,
            "8 周期正弦峰值应保留（线性插值不破幅度）: {peak}"
        );
    }

    /// 24k → 48k 二倍升采样：输出长度 = 输入×2（M1-3.5b TTS 输出通路）
    #[test]
    fn resample_24k_to_48k() {
        let mut r = Resampler::new(24000.0, 48000.0);
        let input: Vec<f32> = (0..2400)
            .map(|i| (2.0 * std::f64::consts::PI * i as f64 / 2400.0 * 6.0).sin() as f32)
            .collect();
        let out = r.push(&input);
        // 输出 k 需要输入 pos=k*0.5 与 pos+1 都在 carry 内：末尾 2 个采样无法插值
        assert_eq!(
            out.len(),
            4798,
            "24k 2400 采样应产出 4798（末尾采样留待后续输入）"
        );
        let peak = out.iter().map(|v| v.abs()).fold(0.0_f32, f32::max);
        assert!(peak > 0.9, "升采样峰值应保留: {peak}");
    }

    /// 跨回调相位连续：两块分别喂与合并喂，输出逐采样一致（base/next_out 工作正常）
    #[test]
    fn resample_across_callbacks() {
        let full: Vec<f32> = (0..960).map(|i| i as f32 / 960.0).collect();
        let (a, b) = full.split_at(480);
        let mut r_split = Resampler::new(48000.0, 16000.0);
        let out_split = {
            let mut o = r_split.push(a);
            o.extend(r_split.push(b));
            o
        };
        let mut r_full = Resampler::new(48000.0, 16000.0);
        let out_full = r_full.push(&full);
        assert_eq!(out_full.len(), out_split.len(), "分块喂不应改变总输出长度");
        for (i, (f, s)) in out_full.iter().zip(out_split.iter()).enumerate() {
            assert!(
                (f - s).abs() < 1e-4,
                "第 {i} 个采样不一致: full={f} split={s}"
            );
        }
    }
}
