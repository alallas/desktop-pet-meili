# 潮间带 M0-3：语音链路 demo（唤醒 → VAD 断句 → ASR → 延迟测量）
# 用法:
#   python scripts/voice_chain_demo.py --file            # 用测试 wav 离线验证 KWS+ASR（自动化）
#   python scripts/voice_chain_demo.py --replay 3.wav    # 把 wav 分块喂给实时状态机（自动化，模拟麦克风）
#   python scripts/voice_chain_demo.py                   # 麦克风实时：喊唤醒词（默认"林美丽"）→ 说话 → 转写
# 模型目录: models/（gitignored，见 docs/踩坑与环境档案.md 下载来源）
import argparse
import time
from pathlib import Path

import numpy as np
import sherpa_onnx

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
SAMPLE_RATE = 16000


def load_kws(keywords_file: str = "kws/keywords.txt"):
    kws_dir = MODELS / "kws"
    kf = Path(keywords_file)
    if not kf.is_absolute():
        kf = MODELS / kf  # 相对路径一律按 models/ 解析，别依赖 CWD
    return sherpa_onnx.KeywordSpotter(
        tokens=str(kws_dir / "tokens.txt"),
        encoder=str(kws_dir / "encoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
        decoder=str(kws_dir / "decoder-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
        joiner=str(kws_dir / "joiner-epoch-12-avg-2-chunk-16-left-64.int8.onnx"),
        keywords_file=str(kf),
        num_threads=2,
        keywords_score=1.0,
        keywords_threshold=0.25,
    )


def load_asr():
    return sherpa_onnx.OfflineRecognizer.from_sense_voice(
        model=str(MODELS / "sense-voice.int8.onnx"),
        tokens=str(MODELS / "sense-voice-tokens.txt"),
        num_threads=4,
        language="zh",
        use_itn=True,
    )


def read_wav(path: Path):
    """读 16kHz 单声道 wav（测试音频均为该格式）"""
    import wave

    with wave.open(str(path), "rb") as w:
        assert w.getframerate() == SAMPLE_RATE and w.getnchannels() == 1
        return np.frombuffer(w.readframes(w.getnframes()), dtype=np.int16).astype(np.float32) / 32768.0


def run_file_mode():
    # 测试 wav 对应的专用关键词表（与 keywords.txt 不同，别混用）
    spotter = load_kws(MODELS / "test_wavs" / "test_keywords.txt")
    recognizer = load_asr()
    kw = (MODELS / "test_wavs" / "test_keywords.txt").read_text(encoding="utf-8").strip().splitlines()
    print(f"测试关键词: {[k.split('@')[-1] for k in kw]}\n")

    total_kw_latency = []
    for wav in sorted((MODELS / "test_wavs").glob("*.wav")):
        samples = read_wav(wav)
        stream = spotter.create_stream()
        stream.accept_waveform(SAMPLE_RATE, samples)
        stream.input_finished()

        t0 = time.perf_counter()
        detected = None
        while spotter.is_ready(stream):
            spotter.decode_stream(stream)
            r = spotter.get_result(stream)
            if r:
                detected = r
                spotter.reset_stream(stream)
        kw_ms = (time.perf_counter() - t0) * 1000

        s = recognizer.create_stream()
        s.accept_waveform(SAMPLE_RATE, samples)
        t0 = time.perf_counter()
        recognizer.decode_stream(s)
        asr_ms = (time.perf_counter() - t0) * 1000
        text = s.result.text.strip()

        total_kw_latency.append(kw_ms)
        print(f"{wav.name}: 唤醒={detected or '（未检出）'} KWS耗时={kw_ms:.0f}ms | ASR: 「{text}」 {asr_ms:.0f}ms")

    print(f"\nKWS 平均耗时: {np.mean(total_kw_latency):.0f}ms（{len(total_kw_latency)} 条）")


class VoiceChain:
    """实时状态机：IDLE(KWS 唤醒) → LISTENING(VAD 断句) → RECOGNIZING(ASR)。麦克风与回放共用。"""

    def __init__(self, keywords_file: str = "kws/keywords.txt", use_tts: bool = False):
        self.spotter = load_kws(keywords_file)
        self.recognizer = load_asr()
        vad = sherpa_onnx.VadModelConfig(
            silero_vad=sherpa_onnx.SileroVadModelConfig(model=str(MODELS / "silero_vad.onnx"))
        )
        self.detector = sherpa_onnx.VoiceActivityDetector(vad, buffer_size_in_seconds=30)

        self.state = "IDLE"
        self.kws_stream = self.spotter.create_stream()  # KWS 是流式解码器，必须复用同一个 stream
        self.utterance = np.array([], dtype=np.float32)
        self.silence_samples = 0  # 用采样数而非墙钟计时，回放模式才正确
        self.wake_time = None

        self.tts = None
        self._sd = None
        if use_tts:
            import os
            import threading

            os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")
            print("后台加载 Kokoro TTS（不阻塞监听）…", flush=True)

            def _load_tts():
                from tts_demo_kokoro import load_pipeline, synth

                p = load_pipeline()
                synth(p, "预热。")  # 冷启动耗时不能算进对话延迟
                self.tts = p
                print("TTS 就绪", flush=True)

            threading.Thread(target=_load_tts, daemon=True).start()
            import sounddevice as sd

            self._sd = sd

    def reply_and_play(self, asr_text: str):
        """M0 占位回复：复读 + 口癖，验证 ASR→TTS→出声全链路"""
        core = asr_text.strip("。，！？、… ")
        if len(core) < 2:
            print("[TTS] 转写太短，跳过回复", flush=True)
            return
        while self.tts is None:
            time.sleep(0.2)  # 等后台 TTS 加载完成
        t0 = time.perf_counter()
        from tts_demo_kokoro import synth

        samples, tts_ms = synth(self.tts, f"我听到你说：{core}。然后呢，我还不太会聊天，请多包涵。")
        print(f"[TTS] 回复合成={tts_ms:.0f}ms，播放中…", flush=True)
        self._sd.play(samples, 24000)
        self._sd.wait()
        print(f"[TTS] 播放完毕，ASR出字→播放完={(time.perf_counter()-t0)*1000:.0f}ms", flush=True)

    def process_chunk(self, chunk: np.ndarray):
        """喂入一帧 16kHz 单声道 float32 音频（建议 1280 帧 = 80ms）"""
        if self.state == "IDLE":
            stream = self.kws_stream
            stream.accept_waveform(SAMPLE_RATE, chunk)
            while self.spotter.is_ready(stream):
                self.spotter.decode_stream(stream)
                r = self.spotter.get_result(stream)  # 先取结果再 reset，否则拿不到
                if r:
                    self.wake_time = time.perf_counter()
                    print(f"\n[唤醒] {r}")
                    self.state = "LISTENING"
                    self.utterance = np.array([], dtype=np.float32)
                    self.silence_samples = 0
                    self.detector.reset()  # 清掉 IDLE 期间积累的音频，避免误断句
                    self.spotter.reset_stream(stream)
                    break
        elif self.state == "LISTENING":
            self.utterance = np.concatenate([self.utterance, chunk])
            self.detector.accept_waveform(chunk)
            while not self.detector.empty():  # drain 已检出的语音段，防缓冲膨胀
                self.detector.pop()
            if self.detector.is_speech_detected():  # 实时说话状态；empty() 在说话中也是空，不能用
                self.silence_samples = 0
            else:
                self.silence_samples += len(chunk)
                if self.silence_samples > SAMPLE_RATE * 0.8 and len(self.utterance) > SAMPLE_RATE * 0.5:
                    self.state = "RECOGNIZING"

        if self.state == "RECOGNIZING":
            self.state = "IDLE"
            s = self.recognizer.create_stream()
            s.accept_waveform(SAMPLE_RATE, self.utterance)
            t0 = time.perf_counter()
            self.recognizer.decode_stream(s)
            asr_ms = (time.perf_counter() - t0) * 1000
            total_ms = (time.perf_counter() - self.wake_time) * 1000
            text = s.result.text.strip()
            print(f"[ASR] 「{text}」 转写={asr_ms:.0f}ms 唤醒到出字总时长={total_ms:.0f}ms\n")
            if self.tts:
                self.reply_and_play(text)


def run_replay_mode(wav_path: Path, keywords_file: str, use_tts: bool = False):
    """自动化验证实时状态机：把 wav 按 1280 帧分块喂进去，KWS 命中词需在关键词表里"""
    chain = VoiceChain(keywords_file, use_tts=use_tts)
    samples = read_wav(wav_path)
    print(f"回放 {wav_path.name}（{len(samples)/SAMPLE_RATE:.1f}s）通过实时状态机…")
    for i in range(0, len(samples), 1280):
        chain.process_chunk(samples[i : i + 1280])
    # 结尾补 1.2s 静音，触发 VAD 断句收尾
    for _ in range(15):
        chain.process_chunk(np.zeros(1280, dtype=np.float32))
    print(f"回放结束，最终状态: {chain.state}（IDLE 表示已出结果或未唤醒）")


def run_mic_mode(device: int | None, use_tts: bool = False):
    import sounddevice as sd

    chain = VoiceChain(use_tts=use_tts)
    print("麦克风监听中，喊「美丽」开始对话，Ctrl+C 退出")

    def on_audio(indata, frames, time_info, status):
        chain.process_chunk(indata[:, 0].copy())

    with sd.InputStream(
        samplerate=SAMPLE_RATE, channels=1, dtype="float32", blocksize=1280, device=device, callback=on_audio
    ):
        while True:
            time.sleep(0.1)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--file", action="store_true", help="用测试 wav 离线验证 KWS+ASR")
    parser.add_argument("--replay", type=str, default=None, help="把指定 wav 喂给实时状态机（自动化）")
    parser.add_argument("--keywords", type=str, default="kws/keywords.txt", help="关键词表文件名（models/ 下，相对路径以 models/ 为根）")
    parser.add_argument("--device", type=int, default=None, help="麦克风设备号（sounddevice 列表）")
    parser.add_argument("--tts", action="store_true", help="接入 Kokoro TTS，ASR 后出声回复")
    args = parser.parse_args()
    if args.file:
        run_file_mode()
    elif args.replay:
        run_replay_mode(MODELS / "test_wavs" / args.replay, MODELS / args.keywords, use_tts=args.tts)
    else:
        run_mic_mode(args.device, use_tts=args.tts)
