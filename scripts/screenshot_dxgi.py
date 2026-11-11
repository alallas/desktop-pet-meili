# DXGI 全屏截图（GDI CopyFromScreen 会漏拍分层透明窗口，本脚本为可信截图方式）
# 用法: python scripts/screenshot_dxgi.py [out.png] [--cursor X Y]
# 坐标为物理像素（本机 2560x1600 @200% 缩放）
import sys
import time

import dxcam
from PIL import Image

out = sys.argv[1] if len(sys.argv) > 1 and not sys.argv[1].startswith("--") else r"D:\Repos\desktop-pet-meili\shot.png"
cursor = None
if "--cursor" in sys.argv:
    i = sys.argv.index("--cursor")
    cursor = (int(sys.argv[i + 1]), int(sys.argv[i + 2]))

if cursor:
    import ctypes
    ctypes.windll.user32.SetCursorPos(cursor[0], cursor[1])
    time.sleep(1.0)  # 等桌宠 100ms 轮询 + 转头插值

cam = dxcam.create(output_idx=0)
img = cam.grab()
if img is None:
    print("grab failed")
    sys.exit(1)
Image.fromarray(img).save(out)
print(f"saved {out} {img.shape[1]}x{img.shape[0]}")
