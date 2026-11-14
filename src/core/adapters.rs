//! 启动前清理本程序留下的隧道网卡（Windows）。
//!
//! 网卡名默认是每次启动随机生成的（`t2p` + `_` + 8 位十六进制，见
//! [`crate::core::config::random_tun_name`]）。进程被强杀时，Wintun 大多会自己
//! 把适配器收走，但偶尔会留下一个**非在册**（phantom）设备节点：`Get-NetAdapter`
//! 里看不见，`pnputil /enum-devices` 才列得出来，而且会一直留在系统里。所以每次
//! 启动（以及每次启动隧道前）都把「`t2p_` 前缀 + 当前不在用」的适配器删掉。
//!
//! 只碰自己建的：**前缀**和**硬件 ID 是 wintun** 两个条件同时满足才动手——机器
//! 上别的 VPN（aTrust 之类）也用 Wintun，靠名字区分。
//!
//! 实现走 SetupAPI 而不是 `pnputil`：后者的输出是本地化的，中文系统上按列解析
//! 迟早出错。

/// 一次清理的结果，调用方拿去写日志。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cleanup {
    /// 删掉的网卡名。
    pub removed: Vec<String>,
    /// 认出来但没删掉的（通常是没权限）。
    pub failed: Vec<String>,
}

/// 删掉本程序留下、当前又不在用的隧道网卡。
///
/// 非 Windows 平台直接返回空结果：Linux 的 tun 设备随进程消失，没有残留。
#[cfg(windows)]
pub fn remove_stale() -> Cleanup {
    imp::remove_stale()
}

#[cfg(not(windows))]
pub fn remove_stale() -> Cleanup {
    Cleanup::default()
}

#[cfg(windows)]
mod imp {
    use std::ptr;
    use std::time::{Duration, Instant};

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiCallClassInstaller, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
        SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, DIF_REMOVE, GUID_DEVCLASS_NET,
        SPDRP_FRIENDLYNAME, SPDRP_HARDWAREID, SP_DEVINFO_DATA,
    };
    use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;

    use super::Cleanup;
    use crate::core::config::{TUN_NAME_PREFIX, TUN_NAME_SEPARATOR};
    use crate::core::elevate;
    use crate::core::traffic;

    /// 随机名的长度：前缀后面固定 8 位十六进制。
    const RANDOM_SUFFIX_LEN: usize = 8;

    /// 等设备真的拆完的上限。
    ///
    /// 正常情况下一两拍（50–100ms）就从枚举里消失了；1.5 秒是留给 PnP 管理器的
    /// 余量，再久就不值得把启动拖下去——真没拆干净，下一次启动还会再判一遍。
    const WAIT_LIMIT: Duration = Duration::from_millis(1500);
    const WAIT_STEP: Duration = Duration::from_millis(50);

    pub fn remove_stale() -> Cleanup {
        let mut result = Cleanup::default();
        // 没有管理员权限时 DIF_REMOVE 一定失败，没必要挨个试一遍。
        if !elevate::is_elevated() {
            return result;
        }
        let Some(set) = open_net_devices() else {
            return result;
        };

        // 先把目标收集齐，再逐个删：删掉一个设备会让后面设备的枚举下标前移，
        // 边枚举边删会漏掉一半。
        let mut targets: Vec<(SP_DEVINFO_DATA, String)> = Vec::new();
        let mut index = 0u32;
        while let Some(info) = next_device(set, index) {
            index += 1;
            let friendly = device_string(set, &info, SPDRP_FRIENDLYNAME).unwrap_or_default();
            let hardware_id = device_string(set, &info, SPDRP_HARDWAREID).unwrap_or_default();
            let Some(name) = stale_name(&friendly, &hardware_id) else {
                continue;
            };
            // 正在用的那张不许删：它可能正是当前隧道在用的适配器（比如固定了名字）。
            if traffic::adapter_up(name) == Some(true) {
                continue;
            }
            targets.push((info, name.to_string()));
        }

        for (info, name) in targets {
            if remove_device(set, &info) {
                result.removed.push(name);
            } else {
                result.failed.push(name);
            }
        }

        // SAFETY: `set` 由 `open_net_devices` 分配，只归还一次。
        unsafe { SetupDiDestroyDeviceInfoList(set) };

        // 删完别急着往下走：`DIF_REMOVE` 只是把移除请求排队，PnP 管理器拆设备栈
        // 是异步的。清理正好排在启动隧道之前，紧接着 tun2proxy 就要建新适配器，
        // 撞上还没拆完的那次移除时内核会用「设备未就绪」（0x10DF）把创建顶回来，
        // 用户看到 `WintunCreateAdapter failed`，可这时系统里一张残留网卡都没有，
        // 怎么看都不像「有东西没清干净」。
        if !result.removed.is_empty() {
            wait_until_gone(&result.removed);
        }
        result
    }

    /// 等这些名字的设备真的从设备枚举里消失，最多 [`WAIT_LIMIT`]。
    ///
    /// 等不到也不报错：宁可这一次多等一会儿，也不能把启动卡在这里——真没拆干净
    /// 的话下一次启动还会再判一遍。
    pub(super) fn wait_until_gone(names: &[String]) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while Instant::now() < deadline {
            if !still_present(names) {
                return;
            }
            std::thread::sleep(WAIT_STEP);
        }
    }

    /// 这些名字里还有没有留在设备树上的。
    pub(super) fn still_present(names: &[String]) -> bool {
        if names.is_empty() {
            return false;
        }
        let present = net_devices();
        names
            .iter()
            .any(|name| present.iter().any(|(friendly, _)| friendly == name))
    }

    /// 打开「网卡类」设备信息集。用完必须 [`SetupDiDestroyDeviceInfoList`]。
    ///
    /// 不带 `DIGCF_PRESENT`：残留的适配器往往是**非在册**（phantom）设备节点——
    /// `Get-NetAdapter` 里看不见，`pnputil /enum-devices` 才列得出来。只枚举
    /// 「当前存在」的设备正好会漏掉要清理的那些。
    fn open_net_devices() -> Option<isize> {
        // SAFETY: 返回设备信息集句柄，失败时是 `INVALID_HANDLE_VALUE`。
        let set = unsafe { SetupDiGetClassDevsW(&GUID_DEVCLASS_NET, ptr::null(), ptr::null_mut(), 0) };
        (set != INVALID_HANDLE_VALUE as isize && set != 0).then_some(set)
    }

    /// 取第 `index` 个网卡设备，越界时返回 `None`。
    fn next_device(set: isize, index: u32) -> Option<SP_DEVINFO_DATA> {
        // SAFETY: 结构体要先自己填好 `cbSize`，这是 SetupAPI 的约定。
        let mut info: SP_DEVINFO_DATA = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;
        // SAFETY: `info` 已按上面初始化，句柄来自成功的 `SetupDiGetClassDevsW`。
        (unsafe { SetupDiEnumDeviceInfo(set, index, &mut info) } != 0).then_some(info)
    }

    /// 枚举一次所有网卡设备的「友好名 + 硬件 ID」。
    ///
    /// 清理要用它确认「删掉的设备是不是真的没了」，测试也用它核对枚举没漏项。
    fn net_devices() -> Vec<(String, String)> {
        let Some(set) = open_net_devices() else {
            return Vec::new();
        };
        let mut found = Vec::new();
        let mut index = 0u32;
        while let Some(info) = next_device(set, index) {
            index += 1;
            found.push((
                device_string(set, &info, SPDRP_FRIENDLYNAME).unwrap_or_default(),
                device_string(set, &info, SPDRP_HARDWAREID).unwrap_or_default(),
            ));
        }
        // SAFETY: `set` 由 `open_net_devices` 分配，只归还一次。
        unsafe { SetupDiDestroyDeviceInfoList(set) };
        found
    }

    /// 这个设备是不是本程序留下、可以删掉的隧道网卡；是的话返回网卡名。
    ///
    /// 两个条件缺一不可：
    /// * 硬件 ID 是 `wintun`（别的 VPN 也用 Wintun，但名字不是我们的前缀）；
    /// * 友好名是「`t2p` + `_` + 8 位十六进制 + ` Tunnel`」，正好是
    ///   [`crate::core::config::random_tun_name`] 生成的样子。
    ///
    /// 判 Wintun 只能看硬件 ID，不能看驱动描述（`SPDRP_DEVICEDESC`）：Wintun 建
    /// 适配器时会把描述改写成网卡名，残留设备上读到的就是 `t2p_xxxxxxxx Tunnel`
    /// 本身，按描述匹配等于永远匹配不上。
    fn stale_name<'a>(friendly: &'a str, hardware_id: &str) -> Option<&'a str> {
        if !hardware_id.to_ascii_lowercase().contains("wintun") {
            return None;
        }
        let name = friendly.strip_suffix(" Tunnel")?;
        let suffix = name
            .strip_prefix(TUN_NAME_PREFIX)?
            .strip_prefix(TUN_NAME_SEPARATOR)?;
        let looks_random = suffix.len() == RANDOM_SUFFIX_LEN
            && suffix.chars().all(|ch| ch.is_ascii_hexdigit());
        looks_random.then_some(name)
    }

    /// 读设备的某个字符串属性（`SPDRP_*`）。
    fn device_string(set: isize, info: &SP_DEVINFO_DATA, property: u32) -> Option<String> {
        let mut buf = [0u16; 256];
        let mut kind = 0u32;
        let mut needed = 0u32;
        // SAFETY: 缓冲区按字节数传入，长度取自 `buf` 自身；失败时不会写缓冲区。
        let ok = unsafe {
            SetupDiGetDeviceRegistryPropertyW(
                set,
                info,
                property,
                &mut kind,
                buf.as_mut_ptr().cast::<u8>(),
                std::mem::size_of_val(&buf) as u32,
                &mut needed,
            )
        };
        if ok == 0 {
            return None;
        }
        let len = buf.iter().position(|&ch| ch == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    /// 卸载（删除）一个设备节点。失败通常是权限不够。
    ///
    /// 这里不调 `SetupDiSetClassInstallParams`：对残留的 Wintun 设备它一律返回
    /// `ERROR_INVALID_USER_BUFFER`（1784），参数压根没挂上，按它的返回值判断就等于
    /// 永远「删不掉」。`DIF_REMOVE` 用默认作用域就能把设备节点整台删掉——实测删完
    /// `Get-PnpDevice` 里就查不到了。
    fn remove_device(set: isize, info: &SP_DEVINFO_DATA) -> bool {
        // SAFETY: `set` 来自成功的 `SetupDiGetClassDevsW`，`info` 是本次枚举出的设备。
        unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set, info) != 0 }
    }

    /// 给单测用的转发：[`stale_name`] 本身是私有函数。
    #[cfg(test)]
    pub(super) fn stale_name_for_test<'a>(friendly: &'a str, hardware_id: &str) -> Option<&'a str> {
        stale_name(friendly, hardware_id)
    }

    /// 给单测用：枚举一次网卡设备，把「友好名 + 硬件 ID」收集出来。
    #[cfg(test)]
    pub(super) fn net_device_names_for_test() -> Vec<(String, String)> {
        net_devices()
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::imp::{
        net_device_names_for_test, stale_name_for_test as stale_name, still_present,
        wait_until_gone,
    };
    use crate::core::config::{TUN_NAME_PREFIX, TUN_NAME_SEPARATOR};

    #[test]
    fn only_our_random_adapters_are_stale() {
        let ours = format!("{TUN_NAME_PREFIX}{TUN_NAME_SEPARATOR}3f9c1a04");
        let friendly = format!("{ours} Tunnel");
        // 真实设备上的硬件 ID 首串就是 `wintun`，大小写不固定（见过 `Wintun`）。
        assert_eq!(
            stale_name(&friendly, "wintun").map(str::to_string),
            Some(ours.clone())
        );
        assert_eq!(
            stale_name(&friendly, "Wintun").map(str::to_string),
            Some(ours.clone()),
            "硬件 ID 大小写不该影响判断"
        );
        assert_eq!(
            stale_name("T2P_3F9C1A04 Tunnel", "wintun"),
            None,
            "前缀区分大小写"
        );
        assert_eq!(
            stale_name(&friendly, "Sangfor aTrust VNIC"),
            None,
            "不是 Wintun 设备"
        );
        assert_eq!(stale_name("t2p_XYZ Tunnel", "wintun"), None, "后缀不是 8 位十六进制");
        assert_eq!(
            stale_name(&format!("{TUN_NAME_PREFIX}3f9c1a04 Tunnel"), "wintun"),
            None,
            "少了分隔符就不算本程序的随机名"
        );
        assert_eq!(
            stale_name("本地连接 2 Tunnel", "wintun"),
            None,
            "别的 Wintun 适配器"
        );
        assert_eq!(stale_name(&ours, "wintun"), None, "没有 Tunnel 后缀就不是隧道适配器");
        // 残留设备的驱动描述被 Wintun 改写成了网卡名，别再拿描述当 Wintun 的判据。
        assert_eq!(
            stale_name(&friendly, &friendly),
            None,
            "描述等于网卡名时不该被当成 Wintun 设备"
        );
    }

    /// 枚举这一步不需要管理员权限，单测里能真跑一遍：能拿到网卡设备、且每台都
    /// 读得出硬件 ID（`cbSize` 或属性号写错时这里会全军覆没）。
    #[test]
    fn enumerating_network_devices_reads_their_hardware_ids() {
        let devices = net_device_names_for_test();
        assert!(
            !devices.is_empty(),
            "这台机器上至少有一张网卡，枚举结果不该是空的"
        );
        assert!(
            devices.iter().any(|(_, hardware_id)| !hardware_id.is_empty()),
            "硬件 ID 一个都没读到，说明 SetupAPI 的调用方式有问题"
        );
        // 清单里不该出现「看着像我们的、但其实不是」的名字被误判。
        for (friendly, hardware_id) in &devices {
            let name = stale_name(friendly, hardware_id);
            assert!(
                name.is_none_or(|name| name.starts_with(crate::core::config::TUN_NAME_PREFIX)),
                "只认本程序的前缀"
            );
        }
    }

    /// 没删到东西就不许等：清理一件没做时，启动不该为此多花一毫秒。
    #[test]
    fn waiting_is_a_no_op_when_nothing_was_removed() {
        let start = std::time::Instant::now();
        wait_until_gone(&[]);
        assert!(
            start.elapsed() < std::time::Duration::from_millis(50),
            "空名单却等了 {:?}",
            start.elapsed()
        );
    }

    /// 空名单永远不算「还在设备树上」，否则一次空清理也会把 1.5 秒等满。
    #[test]
    fn an_empty_name_list_is_never_present() {
        assert!(!still_present(&[]));
    }

    /// 真拿一台存在的设备去问：它必须被认成「还在」，不然等待会在第一拍就提前
    /// 收工，等于白加。
    #[test]
    fn a_device_that_exists_is_reported_as_present() {
        let devices = net_device_names_for_test();
        let Some((friendly, _)) = devices.iter().find(|(name, _)| !name.is_empty()) else {
            return;
        };
        assert!(
            still_present(std::slice::from_ref(friendly)),
            "枚举里明明有 {friendly}，却说它不在"
        );
    }
}
