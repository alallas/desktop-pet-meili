# 潮间带 M1-3.5b TTS sidecar：Kokoro-82M 中文常驻合成服务
# 协议（行式，单行）：
#   入：一行文本（不含换行；脚本内会清洗）→ 合成 24kHz 16-bit PCM WAV
#   出：OK<TAB>wav 绝对路径  或  ERR<TAB>错误信息
# 常驻原因：Kokoro 模型加载 ~2s（坑 16 懒加载补丁），逐句 spawn 不可接受。
# 用法: python scripts/tts_sidecar.py
import sys
import tempfile
import time
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
OUT_DIR = Path(tempfile.gettempdir()) / "chaojiandai-tts"
KEEP_FILES = 20  # 滚动清理，防 temp 堆积


def load_pipeline():
    from huggingface_hub import hf_hub_download
    from kokoro import KModel, KPipeline

    config = hf_hub_download("hexgrad/Kokoro-82M", "config.json")
    model = KModel(config=config, model=str(MODELS / "kokoro-v1_0.pth")).to("cpu").eval()
    return KPipeline(lang_code="z", model=model)


def synth_to_wav(pipeline, text, path):
    """合成整句写 16-bit PCM WAV（24kHz 单声道）。返回样本数。"""
    import numpy as np

    audio = None
    for _, _, out in pipeline(text, voice="zf_xiaoxiao", speed=1.0):
        if out is not None:
            audio = out
    if audio is None or len(audio) == 0:
        raise RuntimeError("Kokoro 未产出音频")
    samples = audio.numpy().astype(np.float32)
    pcm = (samples * 32767.0).clip(-32768, 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(24000)
        w.writeframes(pcm.tobytes())
    return len(samples)


def cleanup():
    files = sorted(OUT_DIR.glob("*.wav"), key=lambda p: p.stat().st_mtime)
    for old in files[:-KEEP_FILES]:
        try:
            old.unlink()
        except OSError:
            pass


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    pipeline = load_pipeline()
    synth_to_wav(pipeline, "预热。", OUT_DIR / "_warmup.wav")  # 首次推理含缓存冷启动
    print("READY", flush=True)

    seq = 0
    for line in sys.stdin:
        text = line.strip().replace("\r", "")
        if not text:
            continue
        # 防御：协议单行，清掉所有控制字符（合成器对换行敏感）
        text = "".join(ch for ch in text if ch >= " " or ch in "\t")
        text = text.replace("\t", "，")
        seq += 1
        path = OUT_DIR / f"tts_{int(time.time())}_{seq}.wav"
        try:
            n = synth_to_wav(pipeline, text, path)
            print(f"OK\t{path}\t{n}", flush=True)
        except Exception as e:  # noqa: BLE001 —— 协议要求任何失败都回 ERR，不能断流
            print(f"ERR\t{type(e).__name__}: {e}", flush=True)
        cleanup()


if __name__ == "__main__":
    main()
