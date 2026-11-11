# 潮间带 M0-9：SetWinEventHook 窗口移动跟踪 demo
# 验证: 其他窗口被拖动/移动时，系统级钩子能否实时拿到位置流（喂给 M1 的"摇晃/跌落"动画）
# 自动化方式: 本脚本自己用 SetWindowPos 正弦摆动记事本，同时用钩子收事件，对时间戳测延迟
# 用法: python scripts/drag_track_demo.py [--seconds 6]
import argparse
import ctypes
import ctypes.wintypes as wt
import threading
import time

user32 = ctypes.windll.user32

EVENT_OBJECT_LOCATIONCHANGE = 0x800B
WINEVENT_OUTOFCONTEXT = 0x0000

SWP_NOSIZE = 0x0001
SWP_NOZORDER = 0x0004
SWP_NOACTIVATE = 0x0010

kernel32 = ctypes.windll.kernel32
GetTickCount64 = kernel32.GetTickCount64

# 全局：事件日志 [(dwmsEventTime, x, y, w, h)]；移动发送日志 [(GetTickCount64, x, y)]
events = []
moves = []
stop_flag = threading.Event()

WinEventProc = ctypes.WINFUNCTYPE(
    None, wt.HANDLE, wt.DWORD, wt.HWND, wt.LONG, wt.LONG, wt.DWORD, wt.DWORD
)


def make_callback(target_hwnd):
    def _cb(hook, event, hwnd, id_object, id_child, thread, event_time):
        if hwnd != target_hwnd or id_object != 0:  # idObject=0 是窗口本身
            return
        r = wt.RECT()
        user32.GetWindowRect(hwnd, ctypes.byref(r))
        arrive_ms = GetTickCount64()
        events.append((arrive_ms, arrive_ms - event_time, r.left, r.top, r.right - r.left, r.bottom - r.top))

    return WinEventProc(_cb)


def animator(hwnd, seconds: float, hz: float):
    """正弦摆动目标窗口，记录每次 SetWindowPos 的发送时间"""
    import math

    r = wt.RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(r))
    base_x, base_y = r.left, r.top
    t0 = time.perf_counter()
    tq0 = t0
    n = 0
    while time.perf_counter() - t0 < seconds:
        t = time.perf_counter() - t0
        x = base_x + int(300 * math.sin(2 * math.pi * 0.8 * t))
        y = base_y + int(60 * math.sin(2 * math.pi * 1.7 * t))
        send_t = GetTickCount64()
        user32.SetWindowPos(hwnd, 0, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE)
        moves.append((send_t, x, y))
        n += 1
        while (GetTickCount64() - send_t) < int(1000.0 / hz):
            time.sleep(0.001)
    print(f"动画端: {n} 次 SetWindowPos @ {hz}Hz")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=6.0)
    parser.add_argument("--hz", type=float, default=30.0)
    args = parser.parse_args()

    # 目标窗口：记事本（自动打开）
    import subprocess

    subprocess.Popen(["notepad.exe"])
    time.sleep(1.5)
    hwnd = user32.FindWindowW(None, "记事本")
    if not hwnd:
        hwnd = user32.FindWindowW("Notepad", None)
    if not hwnd:
        raise RuntimeError("找不到记事本窗口")

    cb = make_callback(hwnd)
    hook = user32.SetWinEventHook(
        EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE, 0, cb, 0, 0, WINEVENT_OUTOFCONTEXT
    )
    if not hook:
        raise RuntimeError("SetWinEventHook 失败")
    print(f"钩子已安装，目标 hwnd={hwnd}，开始摆动 {args.seconds}s…")

    anim = threading.Thread(target=animator, args=(hwnd, args.seconds, args.hz))
    anim.start()

    # 消息循环（钩子回调需要）
    msg = wt.MSG()
    t_end = time.perf_counter() + args.seconds + 2
    while time.perf_counter() < t_end and anim.is_alive():
        user32.PeekMessageW(ctypes.byref(msg), 0, 0, 0, 1)  # PM_REMOVE=1
        time.sleep(0.002)
    anim.join()
    time.sleep(0.3)  # 收尾剩余事件

    user32.UnhookWinEvent(hook)

    # 性能汇总
    import numpy as np

    perf0 = time.perf_counter() - (args.seconds + 2)  # 近似墙钟原点
    # dwmsEventTime 是系统启动以来的毫秒；moves 是 perf_counter。用第一个事件对齐估计偏移
    if not events:
        print("❌ 未收到任何窗口移动事件")
        return
    ev_arr = np.array([e[0] for e in events], dtype=np.float64)   # 到达时刻
    lat = np.array([e[1] for e in events], dtype=np.float64)      # 投递延迟
    xs = np.array([e[2] for e in events])
    print(f"\n结果:")
    print(f"  收到位置事件: {len(events)} 条（动画发送 {len(moves)} 次）")
    print(f"  事件到达间隔: 平均 {np.diff(ev_arr).mean():.1f}ms, p95 {np.percentile(np.diff(ev_arr), 95):.1f}ms")
    print(f"  投递延迟(事件生成→回调执行): 中位 {np.median(lat):.0f}ms, p95 {np.percentile(lat, 95):.0f}ms, 最大 {lat.max():.0f}ms")
    print(f"  x 范围: {xs.min()}..{xs.max()}（预期摆动 ±300）")
    # 拖动结束检测验证：事件流静默 >=100ms 即视为"停手"（M1 摇晃/跌落触发点）
    gaps = np.diff(ev_arr)
    print(f"  最长静默: {gaps.max():.0f}ms（>100ms 即满足跌落触发条件: {'是' if gaps.max() > 100 else '否'}）")


if __name__ == "__main__":
    main()
