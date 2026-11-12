//! 网络采集器：NotifyAddrChange 事件驱动（地址表变化即唤醒），变化时重新探测连通性。
//!
//! 连通性定义：存在"已启用 + 有默认网关 + 非回环"的网卡（LAN 级连通，不做互联网探测——
//! 不引入主动外联，隐私与性能都干净；互联网级检测留给云端 LLM 调用失败时的降级信号）。

use windows_sys::Win32::NetworkManagement::IpHelper::{
    GetAdaptersAddresses, NotifyAddrChange, IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::NetworkManagement::Ndis::IfOperStatusUp;
use windows_sys::Win32::System::Threading::WaitForSingleObject;

use super::Collector;
use crate::event_bus::EventBus;
use crate::ipc::{EventKind, EventSource};

/// GAA_FLAG_INCLUDE_GATEWAYS：要求填充 FirstGatewayAddress（否则网关指针恒空）
const GAA_FLAG_INCLUDE_GATEWAYS: u32 = 0x0080;
const AF_UNSPEC: u32 = 0;

const IF_TYPE_SOFTWARE_LOOPBACK: u32 = 24;
const INFINITE: u32 = 0xFFFF_FFFF;

pub struct NetworkCollector;

impl Collector for NetworkCollector {
    fn start(&self, bus: EventBus) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        std::thread::spawn(move || {
            // 启动先报基线（没网启动 vs 有网启动对行为引擎意义不同）
            let mut prev: Option<bool> = None;
            publish_if_changed(&bus, &mut prev);
            loop {
                let mut handle = std::ptr::null_mut();
                let rc = unsafe { NotifyAddrChange(&mut handle, std::ptr::null_mut()) };
                if rc != 0 {
                    // ERROR_IO_PENDING(997) 以外是真失败（如无网卡环境），退 30s 重试
                    tracing::warn!("NotifyAddrChange failed rc={rc}, retry in 30s");
                    std::thread::sleep(RETRY_INTERVAL);
                    continue;
                }
                unsafe { WaitForSingleObject(handle, INFINITE) };
                publish_if_changed(&bus, &mut prev);
            }
        });
        Ok(())
    }

    fn name(&self) -> &'static str {
        "network"
    }
}

const RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

fn publish_if_changed(bus: &EventBus, prev: &mut Option<bool>) {
    let connected = has_connected_adapter();
    if Some(connected) == *prev {
        return;
    }
    *prev = Some(connected);
    bus.publish(bus.event(
        EventSource::Network,
        EventKind::NetworkChanged,
        serde_json::json!({ "connected": connected }),
    ));
}

/// 是否存在"启用 + 有网关 + 非回环"的网卡。
/// 两次缓冲增长法：先 16KB 探测，不够按返回的必需尺寸重试。
fn has_connected_adapter() -> bool {
    unsafe {
        let mut size = 16 * 1024u32;
        let mut buf = vec![0u8; size as usize];
        let rc = GetAdaptersAddresses(
            AF_UNSPEC,
            GAA_FLAG_INCLUDE_GATEWAYS,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
            &mut size,
        );
        if rc != 0 {
            // 111 = ERROR_BUFFER_OVERFLOW：按真实尺寸重试一次
            buf.resize(size as usize, 0);
            let rc2 = GetAdaptersAddresses(
                AF_UNSPEC,
                GAA_FLAG_INCLUDE_GATEWAYS,
                std::ptr::null_mut(),
                buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH,
                &mut size,
            );
            if rc2 != 0 {
                return false;
            }
        }
        let mut p = buf.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
        while !p.is_null() {
            let a = &*p;
            if a.OperStatus == IfOperStatusUp
                && a.IfType != IF_TYPE_SOFTWARE_LOOPBACK
                && !a.FirstGatewayAddress.is_null()
            {
                return true;
            }
            p = a.Next;
        }
        false
    }
}
