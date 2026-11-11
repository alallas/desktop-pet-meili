# 潮间带 M0-5：DXGI 截屏 + 本地 VLM（SmolVLM-256M）屏幕理解 demo
# 用法:
#   python scripts/vlm_demo.py                    # 截当前屏幕 → 本地 VLM 描述 → 打印延迟
#   python scripts/vlm_demo.py --image out.png    # 分析指定图片
# 目的: 测本地小模型对屏幕内容的理解质量与延迟，为"本地 vs 云端"截图分析决策给数据
import argparse
import time
from pathlib import Path

import torch

ROOT = Path(__file__).resolve().parent.parent
MODELS = ROOT / "models"
VLM_DIR = MODELS / "smolvlm-256m"

DEFAULT_PROMPT = "请用一句简短的中文描述这张屏幕截图里用户在做什么。"


def grab_screen():
    """DXGI 截屏（物理分辨率），返回 PIL RGB 图像"""
    import dxcam
    from PIL import Image

    cam = dxcam.create()
    frame = cam.grab()  # BGRA numpy
    cam.release()
    if frame is None:
        raise RuntimeError("截屏失败")
    img = Image.fromarray(frame[:, :, :3][:, :, ::-1])  # BGR→RGB
    return img


def load_model():
    from transformers import AutoModelForImageTextToText, AutoProcessor

    processor = AutoProcessor.from_pretrained(str(VLM_DIR))
    model = AutoModelForImageTextToText.from_pretrained(
        str(VLM_DIR), torch_dtype=torch.float32, attn_implementation="eager"
    ).eval()
    return processor, model


def ask(processor, model, image, prompt: str, max_new_tokens: int = 48):
    messages = [
        {
            "role": "user",
            "content": [
                {"type": "image"},
                {"type": "text", "text": prompt},
            ],
        }
    ]
    text = processor.apply_chat_template(messages, add_generation_prompt=True)
    inputs = processor(text=text, images=[image], return_tensors="pt")
    with torch.no_grad():
        generated = model.generate(**inputs, max_new_tokens=max_new_tokens, do_sample=False)
    out = processor.batch_decode(generated[:, inputs["input_ids"].shape[1] :], skip_special_tokens=True)
    return out[0].strip()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--image", type=str, default=None, help="分析已有图片而非截屏")
    parser.add_argument("--prompt", type=str, default=DEFAULT_PROMPT)
    parser.add_argument("--rounds", type=int, default=1, help="连续测几轮（测稳态延迟）")
    args = parser.parse_args()

    from PIL import Image

    if args.image:
        image = Image.open(args.image).convert("RGB")
        print(f"图片: {args.image} {image.size}")
    else:
        image = grab_screen()
        print(f"截屏: {image.size}（物理分辨率）")

    t0 = time.perf_counter()
    processor, model = load_model()
    print(f"模型加载: {time.perf_counter()-t0:.1f}s（513MB fp32 CPU）")

    for i in range(args.rounds):
        if i > 0 and not args.image:
            image = grab_screen()
        t0 = time.perf_counter()
        answer = ask(processor, model, image, args.prompt)
        elapsed = time.perf_counter() - t0
        print(f"[第{i+1}轮] 延迟: {elapsed:.1f}s")
        print(f"  回答: {answer}")


if __name__ == "__main__":
    main()
