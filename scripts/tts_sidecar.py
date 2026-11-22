# 潮间带 M1-3.5b TTS sidecar：Kokoro-82M 中文常驻合成服务（#35 预热缓存版）
# 协议（行式，单行）：
#   入：<seq>\t<文本>（seq 为非负整数，按请求回显；并发安全靠 seq 关联）
#   出：OK\t<seq>\t<wav 绝对路径>\t<样本数>  或  ERR\t<seq>\t<错误信息>
# 常驻原因：Kokoro 模型加载 ~2s（坑 16 懒加载补丁），逐句 spawn 不可接受。
# #35：文本→wav 缓存（dict）——行为评论的 map.toml 变体是有限的，
# 后台预热后重复/预合成台词零合成延迟；cleanup 只删不在缓存里的文件。
# 用法: python scripts/tts_sidecar.py
import sys
import tempfile
import time
import wave
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
OUT_DIR = Path(tempfile.gettempdir()) / "chaojiandai-tts"
CACHE_CAP = 150  # 缓存上限：超出后按插入序驱逐（模板台词 ~30 条 + 会话台词，余量充足）

# 文本 → wav 路径。缓存是"零合成延迟"的核心，cleanup 以它为准。
CACHE: dict[str, Path] = {}


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
    """只删不在缓存里的 wav（缓存文件可能被正在播放的台词引用，绝不能动）。"""
    keep = set(CACHE.values())
    for old in OUT_DIR.glob("*.wav"):
        if old in keep:
            continue
        try:
            old.unlink()
        except OSError:
            pass


def evict_if_needed():
    """缓存超上限：按插入序驱逐最老的，删其文件。"""
    while len(CACHE) > CACHE_CAP:
        oldest_text, oldest_path = next(iter(CACHE.items()))
        CACHE.pop(oldest_text)
        try:
            oldest_path.unlink()
        except OSError:
            pass


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    pipeline = load_pipeline()
    synth_to_wav(pipeline, "预热。", OUT_DIR / "_warmup.wav")  # 首次推理含缓存冷启动
    print("READY", flush=True)

    seq = 0
    for line in sys.stdin:
        line = line.strip().replace("\r", "")
        if not line:
            continue
        # 防御：协议单行，清掉所有控制字符（合成器对换行敏感）
        line = "".join(ch for ch in line if ch >= " " or ch in "\t")
        # 协议：行首是 seq，\t 后为合成文本——必须拆开，seq 绝不能进合成文本
        # （#35 回归教训：曾把 \t 替换成逗号，导致每句被念出序号"8，美丽认为"）
        seq_str, sep, text = line.partition("\t")
        text = text.replace("\t", "，")  # 文本内部的制表符降级为逗号
        if not sep or not text.strip():
            print(f"ERR\t{seq}\t协议错误：缺少 seq 或文本", flush=True)
            continue
        try:
            seq = int(seq_str)
        except ValueError:
            print(f"ERR\t{seq}\t协议错误：seq 非数字 {seq_str!r}", flush=True)
            continue
        path = OUT_DIR / f"tts_{int(time.time())}_{seq}.wav"
        try:
            cached = CACHE.get(text)
            if cached is not None and cached.exists():
                n = cached.stat().st_size // 2  # 样本数≈字节数/2（16-bit 单声道）
                print(f"OK\t{seq}\t{cached}\t{n}", flush=True)
                continue
            n = synth_to_wav(pipeline, text, path)
            CACHE[text] = path
            evict_if_needed()
            print(f"OK\t{seq}\t{path}\t{n}", flush=True)
        except Exception as e:  # noqa: BLE001 —— 协议要求任何失败都回 ERR，不能断流
            print(f"ERR\t{seq}\t{type(e).__name__}: {e}", flush=True)
        cleanup()


if __name__ == "__main__":
    main()
