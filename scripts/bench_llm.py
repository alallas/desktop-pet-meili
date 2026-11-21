# -*- coding: utf-8 -*-
"""LLM A/B 基准：同 prompt 同参数打两个 llama-server 端口，测流式延迟。
system prompt 从生物档案 TOML 装配（与 Rust persona.rs 的装配模板保持一致）。
用法: python scripts/bench_llm.py <port> <label> [轮数]
"""
import json, sys, time, urllib.request

PORT = int(sys.argv[1]); LABEL = sys.argv[2]; ROUNDS = int(sys.argv[3]) if len(sys.argv) > 3 else 1

def load_system_prompt(creature_id="meili"):
    try:
        import tomllib
        with open(f"src-tauri/resources/creatures/{creature_id}.toml", "rb") as f:
            spec = tomllib.load(f)
    except Exception:
        spec = {"id": "meili", "name": "美丽", "species": "小水獭",
                "role": "收容所的引导员老员工", "quirk": "美丽认为",
                "quirk_desc": "以第三人称自称的口头垫词", "quirk_comma_habit": True,
                "personality": "元气、好奇、话多、爱收藏，口袋里总有几块最喜欢的石头",
                "setting": "你在主人的电脑桌面上陪伴他、观察他的生活", "max_sentences": 2}
    s = f"你是「{spec['name']}」（{spec['id']}），「潮间带」海洋收容所里的一只{spec['species']}，{spec['role']}。"
    if spec.get("quirk") and spec.get("quirk_desc"):
        comma = "，口癖后习惯带逗号停顿" if spec.get("quirk_comma_habit") else ""
        s += (f"口癖是\"{spec['quirk']}\"——{spec['quirk_desc']}，句首自然带出{comma}，"
              "每两三句里最多出现一次，不刻意刷屏。")
    s += (f"性格{spec['personality']}。{spec['setting']}。"
          f"每次回复不超过{spec.get('max_sentences', 2)}句话，语气温暖，像朋友而不是助理。")
    return s

SYSTEM = load_system_prompt()

PROMPTS = [
    "今天过得好吗",
    "听到吗？",
    "你美吗",
    "我的翅膀画太小了，还是没位置放了怎么办啊？",
    "你知道朱元璋怎么死的吗？",
]
# 生产口径 = Rust SentenceSplitter::DELIMS（逗号不切，见 llm/mod.rs）
SENT_END = set("。！？；!?;\n")
# 参考口径：逗号也切（#32 已落地为生产行为：前缀>=4字即在逗号处软切）
COMMA_END = set("。！？；!?;\n，,")

def chat(user_text):
    body = json.dumps({
        "messages": [{"role": "system", "content": SYSTEM},
                     {"role": "user", "content": user_text}],
        "max_tokens": 90, "temperature": 0.7,
        "chat_template_kwargs": {"enable_thinking": False},
        "stream": True,
    }).encode()
    req = urllib.request.Request(f"http://127.0.0.1:{PORT}/v1/chat/completions",
                                 data=body, headers={"Content-Type": "application/json"})
    t0 = time.perf_counter()
    ttft = first_sent = first_comma = None
    text = ""; ntok = 0
    with urllib.request.urlopen(req, timeout=120) as resp:
        for raw in resp:
            line = raw.decode("utf-8", "replace").strip()
            if not line.startswith("data:"):
                continue
            payload = line[5:].strip()
            if payload == "[DONE]":
                break
            try:
                j = json.loads(payload)
            except json.JSONDecodeError:
                continue
            delta = j["choices"][0]["delta"].get("content") or ""
            if delta:
                now = time.perf_counter()
                if ttft is None:
                    ttft = now - t0
                text += delta; ntok += 1
                if first_sent is None and any(c in SENT_END for c in delta):
                    first_sent = now - t0
                if first_comma is None and any(c in COMMA_END for c in delta):
                    first_comma = now - t0
    total = time.perf_counter() - t0
    return ttft, first_sent, first_comma, total, ntok, text

print(f"=== {LABEL} (port {PORT}) ===")
for r in range(ROUNDS):
    for p in PROMPTS:
        ttft, fs, fc, total, ntok, text = chat(p)
        fs_s = f"{fs*1000:5.0f}ms" if fs else "  n/a "
        fc_s = f"{fc*1000:5.0f}ms" if fc else "  n/a "
        print(f"[{p[:14]:　<14}] TTFT={ttft*1000:4.0f} 生产首句={fs_s} 逗号首句={fc_s} "
              f"总长={total*1000:5.0f}ms ({ntok}tok {ntok/total:4.1f}tok/s) | {text}")
    print()
