# 潮间带 M0-10：WASAPI 回环采集 + 节拍检测 demo
# 验证: 拿到系统输出音频(不影响播放) → 实时检测节拍脉冲 → 估 BPM
# 用途: 节拍脉冲将来直接驱动美美"听到音乐跳舞"的动画节奏
# 用法: 先让系统播放音乐, 再 python scripts/beat_detect_demo.py [--seconds 20]
import argparse
import time
from collections import deque

import numpy as np

CHUNK = 2048          # 48kHz 下 ~42ms/chunk
HOP = 1024            # 谱通量帧移 ~21ms
WIN = 2048            # FFT 窗
REFRactory_MS = 200   # 节拍不应期
BPM_RANGE = (60, 180)

import pyaudiowpatch


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=20.0)
    parser.add_argument("--device", type=int, default=None, help="回环设备号，默认系统默认输出")
    args = parser.parse_args()

    pa = pyaudiowpatch.PyAudio()
    info = pa.get_default_wasapi_loopback() if args.device is None else pa.get_device_info_by_index(args.device)
    sr = int(info["defaultSampleRate"])
    ch = max(1, min(2, int(info["maxInputChannels"])))
    print(f"回环采集: {info['name']} @ {sr}Hz {ch}ch")

    stream = pa.open(
        format=pyaudiowpatch.paFloat32, channels=ch, rate=sr, input=True,
        frames_per_buffer=CHUNK, input_device_index=info["index"],
    )

    prev_mag = None
    flux_hist = deque(maxlen=64)
    beat_times = []
    proc_ms = []
    rms_peaks = []
    audio_buf = np.zeros(WIN, dtype=np.float32)
    t0 = time.perf_counter()
    last_beat = 0.0
    peak_rms = 0.0

    print(f"检测 {args.seconds}s，请确保系统正在播放音乐…")
    while time.perf_counter() - t0 < args.seconds:
        t_read = time.perf_counter()
        raw = stream.read(CHUNK, exception_on_overflow=False)
        data = np.frombuffer(raw, dtype=np.float32).reshape(-1, ch).mean(axis=1)
        peak_rms = max(peak_rms, float(np.sqrt((data**2).mean())))

        # 滑窗谱通量（onset 强度）；只统计 FFT 计算本身的耗时
        t_proc = time.perf_counter()
        audio_buf = np.roll(audio_buf, -CHUNK)
        audio_buf[-CHUNK:] = data
        mag = np.abs(np.fft.rfft(audio_buf * np.hanning(WIN)))
        proc_ms.append((time.perf_counter() - t_proc) * 1000)
        if prev_mag is not None:
            flux = float(np.maximum(0, mag - prev_mag).sum())
            flux_hist.append(flux)
            # 自适应阈值: 近期均值 + 1.3 倍标准差
            if len(flux_hist) > 10:
                th = np.mean(flux_hist) + 1.3 * np.std(flux_hist)
                now = time.perf_counter() - t0
                if flux > th and (now - last_beat) * 1000 > REFRactory_MS:
                    beat_times.append(now)
                    last_beat = now
                    print(f"  ♪ 节拍 t={now:.2f}s flux={flux:.0f}")
        prev_mag = mag

    stream.stop_stream(); stream.close(); pa.terminate()

    proc = np.array(proc_ms)
    print(f"\n结果:")
    print(f"  音频电平峰值 RMS: {peak_rms:.4f}（接近 0 说明没抓到声音，检查是否在播放音乐）")
    print(f"  单块处理耗时: 平均 {proc.mean():.1f}ms / 最大 {proc.max():.1f}ms（预算 {CHUNK/sr*1000:.0f}ms/块）")
    print(f"  检出节拍数: {len(beat_times)}")
    if len(beat_times) >= 4:
        iois = np.diff(beat_times)
        bpms = 60.0 / iois[(iois > 60.0 / BPM_RANGE[1]) & (iois < 60.0 / BPM_RANGE[0])]
        if len(bpms):
            print(f"  节拍间隔: 中位 {np.median(iois)*1000:.0f}ms → BPM 估计 ≈ {np.median(bpms):.0f}")


if __name__ == "__main__":
    main()
