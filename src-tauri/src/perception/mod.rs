//! 感知：系统状态与截屏（截屏五段：触发器/脱敏/分析/写入/透明，M1-5 触发器 +
//! 元数据级，VLM 走待办 #4）。
//!
//! 这里放"系统级状态感知"的无状态工具函数：本地时刻（闸④深夜判定）、
//! 全局 CPU%（闸④高负载判定）等。有状态的采样器归消费方（如行为引擎的
//! CpuMeter 滑窗），本层只提供单次读取。

use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::GetSystemTimes;

/// 本地时刻小时数（0–23）。读取失败返回 None
pub fn local_hour() -> Option<u32> {
    // SAFETY：SYSTEMTIME 是 plain-old-data，GetLocalTime 无失败路径
    let st = unsafe {
        let mut st = std::mem::MaybeUninit::uninit();
        GetLocalTime(st.as_mut_ptr());
        st.assume_init()
    };
    Some(st.wHour as u32)
}

/// 全局 CPU 时间（idle, total），100ns 单位（GetSystemTimes 双取差值前的
/// 原始读数；差值换算归调用方）。读取失败返回 None
pub fn cpu_times_raw() -> Option<(u64, u64)> {
    let mut idle = std::mem::MaybeUninit::uninit();
    let mut kernel = std::mem::MaybeUninit::uninit();
    let mut user = std::mem::MaybeUninit::uninit();
    // SAFETY：三个指针均指向合法 FILETIME 内存
    let ok =
        unsafe { GetSystemTimes(idle.as_mut_ptr(), kernel.as_mut_ptr(), user.as_mut_ptr()) != 0 };
    if !ok {
        return None;
    }
    let (idle, kernel, user) =
        unsafe { (idle.assume_init(), kernel.assume_init(), user.assume_init()) };
    let to_u64 = |ft: windows_sys::Win32::Foundation::FILETIME| {
        ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
    };
    // kernel 时间含 idle：total = kernel + user
    Some((to_u64(idle), to_u64(kernel) + to_u64(user)))
}
