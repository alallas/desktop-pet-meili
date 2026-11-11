# 潮间带 M0-7：llama.cpp sidecar 延迟实测（Qwen3-4B Q4_K_M，CPU）
# 流程: 启动 llama-server -> 流式请求美美口吻短回复 -> 测首 token 延迟与生成速度
# 用法: python scripts/llm_latency_demo.py [--port 8123] [--keep-server]
import argparse
import json
import subprocess
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LLAMA_SERVER = ROOT / "tools" / "llama.cpp" / "llama-server.exe"
MODEL = ROOT / "models" / "qwen3-4b" / "Qwen3-4B-Q4_K_M.gguf"

SYSTEM_PROMPT = (
    "你是「美丽」（meili），潮间带海洋收容所的一只水獭，刚刚被收容、还不太会说话。"
    "你的口癖是“然后呢”。性格好奇、温暖、话不多，每次回复不超过两句话。"
    "你通过观察主人的屏幕和日常陪伴他。"
)

QUESTIONS = [
    "美丽，我今天工作好累啊",
    "美丽，你在干嘛呢",
    "我刚才看视频笑了好久，你觉得我在看什么？",
]


def wait_ready(port: int, timeout: int = 300) -> float:
    t0 = time.perf_counter()
    while time.perf_counter() - t0 < timeout:
        try:
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=2) as r:
                if r.status == 200:
                    return time.perf_counter() - t0
        except Exception:  # noqa: BLE001
            time.sleep(1)
    raise RuntimeError("llama-server 启动超时")


def chat_once(port: int, question: str, max_tokens: int = 90):
    """流式请求，返回 (首token秒, 总秒, 回复文本, token数)"""
    body = json.dumps(
        {
            "model": "qwen3",
            "stream": True,
            "max_tokens": max_tokens,
            "chat_template_kwargs": {"enable_thinking": False},  # Qwen3 关思考，抢首 token
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": question},
            ],
        }
    ).encode()
    req = urllib.request.Request(
        f"http://127.0.0.1:{port}/v1/chat/completions",
        data=body,
        headers={"Content-Type": "application/json"},
    )
    t0 = time.perf_counter()
    ttft = None
    text = ""
    n_tokens = 0
    with urllib.request.urlopen(req, timeout=300) as resp:
        for line in resp:
            line = line.decode("utf-8", "ignore").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                break
            delta = json.loads(payload)["choices"][0]["delta"]
            piece = delta.get("content") or delta.get("reasoning_content")
            if piece:
                if ttft is None:
                    ttft = time.perf_counter() - t0
                text += piece
                n_tokens += 1
    total = time.perf_counter() - t0
    return ttft, total, text, n_tokens


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--port", type=int, default=8123)
    parser.add_argument("--threads", type=int, default=16)
    parser.add_argument("--keep-server", action="store_true")
    args = parser.parse_args()

    server = subprocess.Popen(
        [
            str(LLAMA_SERVER),
            "-m", str(MODEL),
            "--port", str(args.port),
            "-t", str(args.threads),
            "-c", "4096",
            "-np", "1",
        ],
        cwd=str(LLAMA_SERVER.parent),
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        load_s = wait_ready(args.port)
        print(f"llama-server 就绪（模型加载 {load_s:.1f}s，Qwen3-4B Q4_K_M，{args.threads} 线程 CPU）\n")

        # 预热：首次推理包含图编译/缓存冷启动，不计入
        chat_once(args.port, "预热", max_tokens=8)

        for q in QUESTIONS:
            ttft, total, text, n = chat_once(args.port, q)
            speed = n / total if total else 0
            print(f"问: {q}")
            print(f"  首token: {ttft:.2f}s | 总耗时: {total:.2f}s | {n} tokens | {speed:.1f} tok/s")
            print(f"  美美: {text.strip()[:120]}\n")
    finally:
        if not args.keep_server:
            server.terminate()
            try:
                server.wait(timeout=10)
            except subprocess.TimeoutExpired:
                server.kill()


if __name__ == "__main__":
    main()
