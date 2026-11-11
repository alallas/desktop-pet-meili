# 潮间带 M0-3 TTS：Kokoro-82M 中文（本机权重 models/kokoro-v1_0.pth）
# 用法: python scripts/tts_demo_kokoro.py "要合成的文本"
import sys
import time
from pathlib import Path

import numpy as np
import torch

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"


def load_pipeline():
    from huggingface_hub import hf_hub_download
    from kokoro import KModel, KPipeline

    config = hf_hub_download("hexgrad/Kokoro-82M", "config.json")
    model = KModel(config=config, model=str(MODELS / "kokoro-v1_0.pth")).to("cpu").eval()
    return KPipeline(lang_code="z", model=model)


def synth(pipeline, text, voice="zf_xiaoxiao"):
    """返回 (float32 24kHz 样本, 耗时ms)"""
    t0 = time.perf_counter()
    audio = None
    for _, _, out in pipeline(text, voice, speed=1.0):
        if out is not None:
            audio = out
    ms = (time.perf_counter() - t0) * 1000
    return audio.numpy() if audio is not None else None, ms


def main():
    text = sys.argv[1] if len(sys.argv) > 1 else "你好呀，我是美美。然后呢，今天过得怎么样？"
    pipeline = load_pipeline()

    # 预热
    synth(pipeline, "预热。")

    for text_i, t in enumerate([text, "然后呢，我在听你说呀。", "今天的潮水很平静，适合一起发呆。"]):
        samples, ms = synth(pipeline, t)
        duration = len(samples) / 24000
        print(f"[{text_i+1}] 「{t}」 合成={ms:.0f}ms 音频={duration:.2f}s RTF={ms/1000/duration:.2f}")

    # 播放最后一句给用户听
    import sounddevice as sd

    print("播放中…")
    sd.play(samples, 24000)
    sd.wait()


if __name__ == "__main__":
    main()
