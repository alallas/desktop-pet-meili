# 潮间带 M0-3 TTS：sherpa-onnx vits-melo-tts-zh_en（int8）合成 + 播放
# 用法: python scripts/tts_demo.py "要合成的文本"
# 模型: models/tts-melo/（下载来源见 docs/踩坑与环境档案.md）
import sys
import time
from pathlib import Path

import numpy as np
import sherpa_onnx

ROOT = Path(__file__).resolve().parent.parent
TTS_DIR = ROOT / "models" / "tts-melo"


def load_tts():
    config = sherpa_onnx.OfflineTtsConfig(
        model=sherpa_onnx.OfflineTtsModelConfig(
            vits=sherpa_onnx.OfflineTtsVitsModelConfig(
                model=str(TTS_DIR / "model.int8.onnx"),
                lexicon=str(TTS_DIR / "lexicon.txt"),
                tokens=str(TTS_DIR / "tokens.txt"),
                dict_dir=str(TTS_DIR / "dict"),
            ),
            provider="cpu",
            num_threads=4,
        ),
        max_num_sentences=1,
    )
    assert config.validate(), "TTS 配置校验失败"
    return sherpa_onnx.OfflineTts(config)


def main():
    text = sys.argv[1] if len(sys.argv) > 1 else "你好呀，我是美美。然后呢，今天过得怎么样？"
    tts = load_tts()

    t0 = time.perf_counter()
    audio = tts.generate(text, sid=0, speed=1.0)
    gen_ms = (time.perf_counter() - t0) * 1000

    samples = np.asarray(audio.samples, dtype=np.float32)
    duration = len(samples) / audio.sample_rate
    print(f"文本: 「{text}」")
    print(f"合成耗时: {gen_ms:.0f}ms | 音频时长: {duration:.2f}s | 采样率: {audio.sample_rate} | RTF: {gen_ms/1000/duration:.2f}")

    out = ROOT / "models" / "tts_out.wav"
    import wave

    with wave.open(str(out), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(audio.sample_rate)
        w.writeframes((np.clip(samples, -1, 1) * 32767).astype(np.int16).tobytes())
    print(f"已保存: {out}")

    import sounddevice as sd

    print("播放中…")
    sd.play(samples, audio.sample_rate)
    sd.wait()


if __name__ == "__main__":
    main()
