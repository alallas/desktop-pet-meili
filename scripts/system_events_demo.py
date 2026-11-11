# 潮间带 M0-4：系统事件采集 demo
# 能力：前台窗口标题/进程名（ctypes user32）+ SMTC 全局媒体信息（winsdk）+ 电池状态（psutil）
# 用法: python scripts/system_events_demo.py [--interval 2] [--once]
# 验证方法：运行后切换前台窗口、播放/暂停音乐、拔插电源，看日志是否准确上报变更
import argparse
import asyncio
import ctypes
import ctypes.wintypes as wt
from dataclasses import dataclass, field

import psutil

user32 = ctypes.windll.user32
kernel32 = ctypes.windll.kernel32

STATUS_NAMES = {0: "已关闭", 1: "已打开", 2: "切换中", 3: "已停止", 4: "播放中", 5: "已暂停"}


@dataclass
class SystemState:
    foreground: str = ""          # "进程名 | 窗口标题"
    music: str = ""               # "歌手 - 歌名 (状态)" 或 "（无音乐播放）"
    battery: str = ""             # "78% 放电中/充电中/已充满"
    changes: list = field(default_factory=list)

    def snapshot(self):
        fg = get_foreground()
        music = asyncio.run(get_media())
        bat = get_battery()
        changes = []
        if fg != self.foreground:
            changes.append(f"前台切换 -> {fg}")
        if music != self.music:
            changes.append(f"音乐变更 -> {music}")
        if bat != self.battery:
            changes.append(f"电池变更 -> {bat}")
        self.foreground, self.music, self.battery = fg, music, bat
        return changes


def get_foreground() -> str:
    """前台窗口：进程名 + 标题；拿不到进程名就只给标题"""
    hwnd = user32.GetForegroundWindow()
    if not hwnd:
        return "（无前台窗口）"
    length = user32.GetWindowTextLengthW(hwnd)
    buf = ctypes.create_unicode_buffer(length + 1)
    user32.GetWindowTextW(hwnd, buf, length + 1)
    title = buf.value or "（无标题）"

    pid = wt.DWORD()
    user32.GetWindowThreadProcessId(hwnd, ctypes.byref(pid))
    try:
        name = psutil.Process(pid.value).name()
    except psutil.NoSuchProcess:
        name = f"pid{pid.value}"
    return f"{name} | {title}"


async def get_media() -> str:
    """SMTC：当前全局媒体会话的歌名/歌手/播放状态。任何播放器走系统通道都能拿到"""
    try:
        from winsdk.windows.media.control import (
            GlobalSystemMediaTransportControlsSessionManager,
        )

        manager = await GlobalSystemMediaTransportControlsSessionManager.request_async()
        session = manager.get_current_session()
        if session is None:
            return "（无媒体会话）"
        props = await session.try_get_media_properties_async()
        status = session.get_playback_info().playback_status
        status_str = STATUS_NAMES.get(status.value, str(status.value))
        artist = props.artist or "未知歌手"
        title = props.title or "未知标题"
        return f"{artist} - {title} ({status_str})"
    except Exception as e:  # noqa: BLE001 - demo 里想知道真实失败原因
        return f"（SMTC 获取失败: {type(e).__name__}: {e}）"


def get_battery() -> str:
    b = psutil.sensors_battery()
    if b is None:
        return "（无电池，台式机）"
    pct = round(b.percent)
    if b.power_plugged:
        state = "已充满" if pct >= 99 else "充电中"
    else:
        state = "放电中"
    secs = b.secsleft
    if isinstance(secs, int) and secs > 0 and not b.power_plugged:
        state += f" 预计{secs//60}分钟"
    return f"{pct}% {state}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--interval", type=float, default=2.0, help="轮询间隔秒")
    parser.add_argument("--once", action="store_true", help="采一次就退出")
    args = parser.parse_args()

    state = SystemState()
    print(f"初始状态：{state.snapshot() and ''}")
    print(f"  前台: {state.foreground}")
    print(f"  音乐: {state.music}")
    print(f"  电池: {state.battery}")
    print(f"轮询中（间隔 {args.interval}s），切换窗口/播停音乐/拔插电源看变更上报，Ctrl+C 退出\n")

    try:
        while True:
            for c in state.snapshot():
                print(c, flush=True)
            if args.once:
                break
            import time

            time.sleep(args.interval)
    except KeyboardInterrupt:
        print("\n退出")


if __name__ == "__main__":
    main()
