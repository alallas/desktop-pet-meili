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
    sw, sh = user32.GetSystemMetrics(0), user32.GetSystemMetrics(1)
    print(f"启动时窗口: {x1-x0}x{y1-y0} @ ({x0},{y0}) | 屏幕(逻辑): {sw}x{sh}")

    # 关键修正：把窗口缩到桌宠大小，窗口移动=桌宠移动，巡逻范围=整个屏幕
    # （M0-1 的窗口是接近全屏的 1293x808，直接移窗只能动 13px，视觉上就是原地抖动）
    PET = 360
    user32.SetWindowPos(hwnd, 0, sw - PET, sh - PET, PET, PET, SWP_NOZORDER | SWP_NOACTIVATE)
    w = h = PET
    print(f"已缩放窗口到 {PET}x{PET}（模拟 M1 的桌宠专属小窗），位于右下角")

    # 巡逻路径：沿屏幕下缘走到左下角 → 爬到屏幕顶 → 走回右侧 → 落回右下角
    # （水平巡逻 + 垂直攀爬一次验证完；边缘检测 = 窗口矩形 vs 屏幕矩形）
    speed = 350.0  # 逻辑像素/秒
    stages = [
        ("沿下缘左行", sw - PET, sh - PET, 0, sh - PET),
        ("沿左缘攀爬", 0, sh - PET, 0, 0),
        ("沿上缘右行", 0, 0, sw - PET, 0),
        ("沿右缘落下", sw - PET, 0, sw - PET, sh - PET),
    ]
    stage_i = 0
    x, y = float(sw - PET), float(sh - PET)

    dt = 1.0 / args.hz
    t_end = time.perf_counter() + args.seconds
    n_frames = 0
    intervals = []
    setwindowpos_ms = []  # 每次 SetWindowPos 真实耗时
    t_prev = time.perf_counter()

    print(f"开始巡逻 {args.seconds}s @ {args.hz}Hz，速度 {speed}px/s（下缘→左爬→上缘→右落）…")
    while time.perf_counter() < t_end and stage_i < len(stages):
        name, x_from, y_from, x_to, y_to = stages[stage_i]
        dx, dy = x_to - x_from, y_to - y_from
        x += speed * dt * (1 if dx > 0 else -1 if dx < 0 else 0)
        y += speed * dt * (1 if dy > 0 else -1 if dy < 0 else 0)
        # 边缘钳制 + 到达舞台终点判定（按方向检查过冲）
        x = max(0.0, min(x, float(sw - w)))
        y = max(0.0, min(y, float(sh - h)))
        arrived = (dx <= 0 and x <= x_to) or (dx >= 0 and x >= x_to)
        arrived = arrived and ((dy <= 0 and y <= y_to) or (dy >= 0 and y >= y_to))
        if arrived:
            x, y = float(x_to), float(y_to)
            stage_i += 1
        t_call = time.perf_counter()
        move_to(hwnd, x, y)
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
    print(f"\n结果: {n_frames} 帧 / {args.seconds:.1f}s，完成舞台 {stage_i}/{len(stages)}")
    print(f"  位置更新间隔: 平均 {arr.mean():.1f}ms, p95 {np.percentile(arr, 95):.1f}ms, 最大 {arr.max():.1f}ms")
    print(f"  SetWindowPos 单次: 平均 {calls.mean():.2f}ms, 最大 {calls.max():.2f}ms")
    print(f"  边缘钳制: x∈[0,{sw-w}] y∈[0,{sh-h}]，生效")

    # 归位：回到启动时的位置
    move_to(hwnd, x0, y0)
    print(f"已归位到 ({x0},{y0})")


if __name__ == "__main__":
    main()
