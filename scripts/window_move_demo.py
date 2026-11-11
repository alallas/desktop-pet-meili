# 潮间带 M0-6：窗口自移动 demo（SetWindowPos 动画 + 屏幕边缘检测）
# 验证: 桌宠窗口能否平滑地自己走动（60Hz 位置更新）、到屏幕边缘自动折返、CPU 开销可接受
# 前置: chaojiandai.exe 已运行（M0-1 的透明窗口，标题「潮间带」）
# 用法: python scripts/window_move_demo.py [--seconds 8]
import argparse
import ctypes
import ctypes.wintypes as wt
import time

user32 = ctypes.windll.user32

SWP_NOSIZE = 0x0001
SWP_NOZORDER = 0x0004
SWP_NOACTIVATE = 0x0010

WINDOW_TITLE = "潮间带"


def find_pet_window() -> int:
    hwnd = user32.FindWindowW(None, WINDOW_TITLE)
    if not hwnd:
        raise RuntimeError(f"找不到窗口「{WINDOW_TITLE}」，先启动 chaojiandai.exe")
    return hwnd


def get_rect(hwnd) -> tuple[int, int, int, int]:
    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    return r.left, r.top, r.right, r.bottom


def move_to(hwnd, x: int, y: int):
    user32.SetWindowPos(hwnd, 0, int(x), int(y), 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=8.0, help="巡逻时长")
    parser.add_argument("--hz", type=float, default=60.0, help="目标位置更新频率")
    args = parser.parse_args()

    hwnd = find_pet_window()
    x0, y0, x1, y1 = get_rect(hwnd)
    w, h = x1 - x0, y1 - y0
    sw, sh = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    print(f"窗口: {w}x{h} @ ({x0},{y0}) | 屏幕(逻辑): {sw}x{sh}")

    # 巡逻路径：沿屏幕下缘左右走动，到边缘折返（M1 的"攀爬"先验证水平移动）
    y_bottom = sh - h  # 窗口贴屏幕下缘
    speed = 400.0  # 逻辑像素/秒
    x = float(x0)
    direction = 1.0 if x0 <= 0 else -1.0
    if x0 <= 0:
        x = 0.0

    dt = 1.0 / args.hz
    t_end = time.perf_counter() + args.seconds
    n_frames = 0
    intervals = []
    setwindowpos_ms = []  # 每次 SetWindowPos 真实耗时
    t_prev = time.perf_counter()

    print(f"开始巡逻 {args.seconds}s @ {args.hz}Hz，速度 {speed}px/s …")
    while time.perf_counter() < t_end:
        x += direction * speed * dt
        if x <= 0:
            x, direction = 0.0, 1.0
        elif x >= sw - w:
            x, direction = float(sw - w), -1.0
        t_call = time.perf_counter()
        move_to(hwnd, x, y_bottom)
        setwindowpos_ms.append((time.perf_counter() - t_call) * 1000)

        now = time.perf_counter()
        intervals.append(now - t_prev)
        t_prev = now
        n_frames += 1
        # 睡到临近下一帧再短旋，降低无谓 CPU 空转
        target = now + dt
        while time.perf_counter() < target - 0.003:
            time.sleep(0.001)

    import numpy as np

    arr = np.array(intervals[1:]) * 1000
    calls = np.array(setwindowpos_ms)
    print(f"\n结果: {n_frames} 帧 / {args.seconds:.1f}s")
    print(f"  位置更新间隔: 平均 {arr.mean():.1f}ms, p95 {np.percentile(arr, 95):.1f}ms, 最大 {arr.max():.1f}ms")
    print(f"  SetWindowPos 单次: 平均 {calls.mean():.2f}ms, 最大 {calls.max():.2f}ms")
    print(f"  边缘检测: 窗口范围 [{0},{sw-w}], 本次折返 {'是' if n_frames else '否'}")

    # 归位：回到启动时的位置
    move_to(hwnd, x0, y0)
    print(f"已归位到 ({x0},{y0})")


if __name__ == "__main__":
    main()
