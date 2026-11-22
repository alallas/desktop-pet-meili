# 潮间带 M1-6：模拟"按住宠物拎起拖动"的验收脚本（SendInput 注入真实输入）
# 用法: python scripts/drag_pet_sim.py [--dx -7 --dy -5 --steps 20]
# 预期: 小窗被拖离原位（默认每次移动 -7,-5，20 次 ≈ 位移 -140,-100），
#       日志出现 "pet drag ended moved=true"
import argparse
import ctypes
import ctypes.wintypes as wt
import time

# DPI 感知必须先做：不感知时 GetWindowRect 返回 96dpi 缩放坐标，
# SendInput 绝对坐标是物理像素，两个坐标空间混用会点偏（踩坑档案）
ctypes.windll.shcore.SetProcessDpiAwareness(2)

user32 = ctypes.windll.user32

INPUT_MOUSE = 0
MOUSEEVENTF_MOVE = 0x0001
MOUSEEVENTF_LEFTDOWN = 0x0002
MOUSEEVENTF_LEFTUP = 0x0004
MOUSEEVENTF_ABSOLUTE = 0x8000


class MOUSEINPUT(ctypes.Structure):
    _fields_ = [
        ("dx", wt.LONG),
        ("dy", wt.LONG),
        ("mouseData", wt.DWORD),
        ("dwFlags", wt.DWORD),
        ("time", wt.DWORD),
        ("dwExtraInfo", ctypes.POINTER(ctypes.c_ulong)),
    ]


class INPUT_UNION(ctypes.Union):
    _fields_ = [("mi", MOUSEINPUT), ("pad", ctypes.c_byte * 24)]


class INPUT(ctypes.Structure):
    _fields_ = [("type", wt.DWORD), ("union", INPUT_UNION)]


def send(flags, dx=0, dy=0):
    inp = INPUT()
    inp.type = INPUT_MOUSE
    inp.union.mi = MOUSEINPUT(dx, dy, 0, flags, 0, None)
    sent = user32.SendInput(1, ctypes.byref(inp), ctypes.sizeof(INPUT))
    if sent != 1:
        raise RuntimeError(f"SendInput 失败 flags={flags:#x}")


def find_pet_hwnd():
    """按进程名 chaojiandai 找主窗（窗口标题随配置变，进程名稳定）"""
    import subprocess

    out = subprocess.run(
        ["powershell", "-NoProfile", "-Command",
         "(Get-Process chaojiandai -ErrorAction SilentlyContinue).MainWindowHandle"],
        capture_output=True, text=True,
    ).stdout.strip()
    handles = [int(x) for x in out.split() if x.strip().isdigit() and int(x) != 0]
    if not handles:
        raise RuntimeError("找不到 chaojiandai 窗口进程")
    return handles[0]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--dx", type=int, default=-7)
    parser.add_argument("--dy", type=int, default=-5)
    parser.add_argument("--steps", type=int, default=20)
    args = parser.parse_args()

    hwnd = find_pet_hwnd()
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    cx = (r.left + r.right) // 2
    cy = (r.top + r.bottom) // 2
    print(f"窗口 rect=({r.left},{r.top}) {r.right - r.left}x{r.bottom - r.top}，从中心 ({cx},{cy}) 抓起")

    sw = user32.GetSystemMetrics(0)
    sh = user32.GetSystemMetrics(1)

    def abs_coords(x, y):
        # 绝对坐标是 0..65535 映射整屏
        return int(x * 65535 / (sw - 1)), int(y * 65535 / (sh - 1))

    # 1) 移到窗口中心
    ax, ay = abs_coords(cx, cy)
    send(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay)
    time.sleep(0.1)
    # 2) 按下（WH_MOUSE_LL 应摘掉穿透并启动拖线程）
    send(MOUSEEVENTF_LEFTDOWN)
    print("LEFTDOWN 已注入")
    # 3) 20 次绝对坐标移动，左上各 -7/-5 px，16ms 间隔（模拟人手拖动）。
    #    必须用绝对坐标：相对移动会过鼠标加速曲线，16ms 一步被放大 ~2.8 倍，
    #    窗口瞬间被甩到屏幕角落（2026-11-22 实测踩过）
    for i in range(1, args.steps + 1):
        ax, ay = abs_coords(cx + args.dx * i, cy + args.dy * i)
        send(MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE, ax, ay)
        time.sleep(0.016)
    # 4) 松手
    send(MOUSEEVENTF_LEFTUP)
    print("LEFTUP 已注入")
    time.sleep(0.5)

    r2 = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r2))
    dx, dy = r2.left - r.left, r2.top - r.top
    exp_dx, exp_dy = args.dx * args.steps, args.dy * args.steps
    print(f"位移: ({dx},{dy})（预期 ≈ ({exp_dx},{exp_dy})，右下拖拽预期被工作区夹取截断）")
    if args.dx > 0 or args.dy > 0:
        ok = r2.bottom <= 1504 + 1 and r2.right <= 2560 + 1  # 夹取上限（本机工作区）
        print("✅ 夹取生效（没扎进任务栏）" if ok else f"❌ 越界 bottom={r2.bottom} right={r2.right}")
    else:
        print("✅ 拖拽生效" if abs(dx - exp_dx) < 30 and abs(dy - exp_dy) < 30 else "❌ 拖拽未生效")


if __name__ == "__main__":
    main()
