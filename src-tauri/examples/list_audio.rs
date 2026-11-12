//! 音频设备枚举调试工具：列出 cpal 看到的输入设备与默认输入。
//! 用法：cargo run --example list_audio
//! 背景：Windows「设置」里切的默认输入与 cpal default_input_device 可能不一致
//! （Console/Multimedia/Communications 三套默认），本工具看引擎实际会选谁。

use cpal::traits::{DeviceTrait, HostTrait};

fn main() {
    let host = cpal::default_host();
    println!("host: {}", host.id().name());
    match host.default_input_device() {
        Some(d) => println!("默认输入: {}", d.name().unwrap_or_default()),
        None => println!("默认输入: (无)"),
    }
    println!("--- 全部输入设备 ---");
    for d in host.input_devices().expect("枚举输入设备") {
        let name = d.name().unwrap_or_default();
        let cfg = d
            .default_input_config()
            .map(|c| {
                format!(
                    "{}ch {}Hz {:?}",
                    c.channels(),
                    c.sample_rate().0,
                    c.sample_format()
                )
            })
            .unwrap_or_else(|e| format!("(无默认配置: {e})"));
        println!("  {name}  [{cfg}]");
    }
}
