//! 直连出口：让「直连」真的从物理网卡出去，而不是又掉回 TUN。
//!
//! # 为什么这是个必须做对的问题
//!
//! tun2proxy 起来之后会把 `0.0.0.0/1` 和 `128.0.0.0/1` 两条路由指向 TUN
//! 网卡，等于接管了整机默认路由。分流器如果直接 `TcpStream::connect`，这个包
//! 会立刻被 TUN 抓回 tun2proxy、再送回分流器——**同一条连接被自己吃掉**，表现
//! 就是「一开分流网页就白屏」。
//!
//! 出路有三条，按可靠性排序：
//!
//! 1. `IP_UNICAST_IF`（Windows）/ `SO_BINDTODEVICE`（Linux）：把 socket 绑到物理
//!    网卡索引上。这是**按出接口选路**，不受目的地址影响，最可靠；
//! 2. 绑定物理网卡的源 IP：路由仍按目的地址查，只在部分网络环境下有效；
//! 3. 都不成 → 这条连接**改走代理**并记一条日志，绝不裸连。
//!
//! # DNS 也得走同一条出口
//!
//! 「直连」拿到域名后要自己解析。用系统解析器的话，查询会被 TUN 抓走、还可能
//! 拿回 fake-IP（198.18.x.x），于是「直连 baidu.com」最后连到了一个假地址。
//! 所以这里自带一个极小的 DNS 客户端：UDP 报文直接发到配置的直连 DNS，
//! socket 同样绑在物理出口上，A / AAAA 都取。

use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

use crate::core::i18n::t;
use crate::tr_format;

/// 物理出口：接口索引 + 名字 + 该接口上的 IPv4 地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outlet {
    /// 界面上显示的名字（Windows 是接口别名，Linux 是 `eth0` 这类设备名）。
    pub label: String,
    /// 系统里给这个接口的序号：`IP_UNICAST_IF` 要的就是它。
    pub index: u32,
    /// 网卡上的 IPv4 地址，绑定失败时的兜底。
    pub local_ip: Option<IpAddr>,
}

impl Outlet {
    /// 同步拨号：socket 先绑到物理出口，再连目标。
    ///
    /// 刻意做成阻塞调用：调用方（分流器）把它丢进 `spawn_blocking`，
    /// 这样「绑接口 + 连接」这一整段不会被 async 调度切成几截。
    pub fn dial(&self, target: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        let first = self.dial_bound(target, timeout);
        match first {
            Ok(stream) => Ok(stream),
            Err(bound_err) => {
                // 兜底：绑接口失败时退回「绑源 IP」。有些系统上
                // IP_UNICAST_IF 对没有该接口的环境会返回 WSAEINVAL。
                match self.dial_with_source(target, timeout) {
                    Ok(stream) => Ok(stream),
                    Err(_) => Err(bound_err),
                }
            }
        }
    }

    fn dial_bound(&self, target: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        let domain = if target.is_ipv4() {
            Domain::IPV4
        } else {
            Domain::IPV6
        };
        let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
        bind_to_interface(&socket, self, target.is_ipv4())?;
        // 必须用 connect_timeout：`set_read_timeout` / `set_write_timeout` 管不到
        // 建连，普通的 `connect()` 会一直等到系统 SYN 重传结束（Windows 约 21 秒、
        // Linux 约 130 秒），直连一个不通的国内地址就会把这条连接卡死。
        socket.connect_timeout(&target.into(), timeout)?;
        socket.set_nonblocking(true)?;
        Ok(socket.into())
    }

    fn dial_with_source(&self, target: SocketAddr, timeout: Duration) -> io::Result<TcpStream> {
        let Some(local) = self.local_ip else {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                t("出口网卡没有 IPv4 地址", "The outlet adapter has no IPv4 address"),
            ));
        };
        if local.is_ipv4() != target.is_ipv4() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                t("出口地址族与目标不一致", "The outlet and the target use different address families"),
            ));
        }
        let socket = Socket::new(
            if target.is_ipv4() {
                Domain::IPV4
            } else {
                Domain::IPV6
            },
            Type::STREAM,
            Some(Protocol::TCP),
        )?;
        let bind_addr = SocketAddr::new(local, 0);
        socket.bind(&bind_addr.into())?;
        // 同上：建连的超时只能靠 connect_timeout。
        socket.connect_timeout(&target.into(), timeout)?;
        socket.set_nonblocking(true)?;
        Ok(socket.into())
    }

    /// 用这个出口向 `server` 查 `host` 的地址（A 与 AAAA）。
    pub fn resolve(
        &self,
        server: SocketAddr,
        host: &str,
        timeout: Duration,
    ) -> io::Result<Vec<IpAddr>> {
        let id = next_query_id();
        let packet = build_query(id, host, true);
        let first = self.query(server, &packet, id, timeout)?;
        if !first.is_empty() {
            return Ok(first);
        }
        // A 查不到再试 AAAA：只有 IPv6 的站点在纯 A 查询下是「查不到」。
        let packet = build_query(id, host, false);
        self.query(server, &packet, id, timeout)
    }

    fn query(
        &self,
        server: SocketAddr,
        packet: &[u8],
        id: u16,
        timeout: Duration,
    ) -> io::Result<Vec<IpAddr>> {
        let socket = Socket::new(
            if server.is_ipv4() {
                Domain::IPV4
            } else {
                Domain::IPV6
            },
            Type::DGRAM,
            Some(Protocol::UDP),
        )?;
        // UDP 也要绑出口：否则查询本身会被 TUN 抓走。
        let _ = bind_to_interface(&socket, self, server.is_ipv4());
        let bind_addr: SocketAddr = if server.is_ipv4() {
            "0.0.0.0:0".parse().expect("常量地址")
        } else {
            "[::]:0".parse().expect("常量地址")
        };
        socket.bind(&bind_addr.into())?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        socket.connect(&server.into())?;
        // socket2 0.6 的 `recv` 要 `MaybeUninit` 缓冲区；转成标准库的
        // `UdpSocket` 之后就能直接用 `recv`，读写语义完全一致。
        let socket: std::net::UdpSocket = socket.into();
        socket.send(packet)?;

        let mut buffer = vec![0u8; 2048];
        let started = Instant::now();
        loop {
            match socket.recv(&mut buffer) {
                Ok(read) => {
                    if read < 12 {
                        continue;
                    }
                    let reply_id = u16::from_be_bytes([buffer[0], buffer[1]]);
                    if reply_id != id {
                        continue;
                    }
                    return parse_response(&buffer[..read]);
                }
                Err(err)
                    if err.kind() == io::ErrorKind::WouldBlock
                        || err.kind() == io::ErrorKind::TimedOut =>
                {
                    if started.elapsed() >= timeout {
                        return Err(err);
                    }
                    continue;
                }
                Err(err) => return Err(err),
            }
        }
    }
}

/// 把 socket 绑到一个接口上。
#[cfg(windows)]
fn bind_to_interface(socket: &Socket, outlet: &Outlet, ipv4: bool) -> io::Result<()> {
    use std::os::windows::io::AsRawSocket;

    // Windows 上没有 SO_BINDTODEVICE，等价物是 IP_UNICAST_IF（IPv4）/
    // IPV6_UNICAST_IF（IPv6）：值是**网络字节序**的接口索引。
    const IPPROTO_IP: i32 = 0;
    const IPPROTO_IPV6: i32 = 41;
    const IP_UNICAST_IF: i32 = 31;
    const IPV6_UNICAST_IF: i32 = 31;

    let value = outlet.index.to_be();
    let (level, optname) = if ipv4 {
        (IPPROTO_IP, IP_UNICAST_IF)
    } else {
        (IPPROTO_IPV6, IPV6_UNICAST_IF)
    };
    let rc = unsafe {
        setsockopt(
            socket.as_raw_socket() as usize,
            level,
            optname,
            &value as *const u32 as *const u8,
            std::mem::size_of::<u32>() as i32,
        )
    };
    if rc != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(windows)]
#[link(name = "ws2_32")]
extern "system" {
    fn setsockopt(
        socket: usize,
        level: i32,
        optname: i32,
        optval: *const u8,
        optlen: i32,
    ) -> i32;
}

/// Linux / macOS 直接用 `SO_BINDTODEVICE`。
#[cfg(unix)]
fn bind_to_interface(socket: &Socket, outlet: &Outlet, _ipv4: bool) -> io::Result<()> {
    socket.bind_device(Some(outlet.label.as_bytes()))
}

#[cfg(not(any(windows, unix)))]
fn bind_to_interface(_socket: &Socket, _outlet: &Outlet, _ipv4: bool) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        t("这个系统不支持绑定出口网卡", "This system cannot bind an outlet adapter"),
    ))
}

/// 找到当前默认路由走的物理网卡。
///
/// `hint` 是用户手填的网卡名或索引，留空则自动判断。自动判断必须在 tun2proxy
/// 起来**之前**做（那时默认路由还是干净的一条），所以调用方要按这个顺序来。
pub fn detect(hint: &str) -> Result<Outlet, String> {
    let hint = hint.trim();
    if !hint.is_empty() {
        return manual(hint);
    }
    platform_detect()
}

fn manual(hint: &str) -> Result<Outlet, String> {
    // 数字当接口索引用，其它当网卡名。
    if let Ok(index) = hint.parse::<u32>() {
        return Ok(Outlet {
            label: hint.to_string(),
            index,
            local_ip: None,
        });
    }
    let outlets = platform_list();
    outlets
        .into_iter()
        .find(|outlet| outlet.label.eq_ignore_ascii_case(hint))
        .ok_or_else(|| {
            tr_format!(
                "找不到网卡「{hint}」，请填系统里显示的网卡名或索引号",
                "No adapter named “{hint}”; enter the adapter name or index shown by the system"
            )
        })
}

#[cfg(windows)]
fn platform_detect() -> Result<Outlet, String> {
    // 只问默认路由：tun2proxy 的 TUN 路由是 0.0.0.0/1 这种更具体的条目，
    // 不会出现在 0.0.0.0/0 的列表里，正好避开。
    let script = "$r = Get-NetRoute -DestinationPrefix '0.0.0.0/0' -ErrorAction SilentlyContinue | \
         Where-Object { $_.InterfaceAlias -notmatch 'tun|wintun' } | \
         Sort-Object -Property @{Expression={$_.RouteMetric + (Get-NetIPInterface -InterfaceIndex $_.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue).InterfaceMetric}} | \
         Select-Object -First 1; \
         if ($r) { $ip = (Get-NetIPAddress -InterfaceIndex $r.InterfaceIndex -AddressFamily IPv4 -ErrorAction SilentlyContinue | Select-Object -First 1).IPAddress; \
         Write-Output ($r.InterfaceIndex.ToString() + '|' + $r.InterfaceAlias + '|' + $ip) }";
    let output = powershell(script)
        .map_err(|err| tr_format!("无法查询网卡：{err}", "Cannot query adapters: {err}"))?;
    let line = output.trim();
    if line.is_empty() {
        return Err(t(
            "找不到默认路由，请手动填写网卡名",
            "No default route found; enter the adapter name manually",
        )
        .to_string());
    }
    let mut parts = line.split('|');
    let index = parts
        .next()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .ok_or_else(|| {
            tr_format!("网卡信息解析失败：{line}", "Cannot parse adapter info: {line}")
        })?;
    let label = parts.next().unwrap_or("").trim().to_string();
    let local_ip = parts
        .next()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .and_then(|text| text.parse::<IpAddr>().ok());
    Ok(Outlet {
        label: if label.is_empty() {
            format!("接口 {index}")
        } else {
            label
        },
        index,
        local_ip,
    })
}

#[cfg(target_os = "linux")]
fn platform_detect() -> Result<Outlet, String> {
    let table = std::fs::read_to_string("/proc/net/route").unwrap_or_default();
    let mut best: Option<(String, u32)> = None;
    for line in table.lines().skip(1) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            continue;
        }
        // 目的地址 00000000 + 掩码 00000000 = 默认路由；Metric 越小越优先。
        if fields[1] != "00000000" || fields[7] != "00000000" {
            continue;
        }
        let name = fields[0].to_string();
        if name.starts_with("tun") {
            continue;
        }
        let metric = fields[6].parse::<u32>().unwrap_or(u32::MAX);
        if best.as_ref().map(|(_, m)| metric < *m).unwrap_or(true) {
            best = Some((name, metric));
        }
    }
    let Some((name, _)) = best else {
        return Err(t(
            "找不到默认路由，请手动填写网卡名",
            "No default route found; enter the adapter name manually",
        )
        .to_string());
    };
    let index = std::fs::read_to_string(format!("/sys/class/net/{name}/ifindex"))
        .ok()
        .and_then(|text| text.trim().parse::<u32>().ok())
        .ok_or_else(|| tr_format!("读不到 {name} 的接口索引", "Cannot read the index of {name}"))?;
    let local_ip = local_ipv4_of(&name);
    Ok(Outlet {
        label: name,
        index,
        local_ip,
    })
}

#[cfg(target_os = "linux")]
fn local_ipv4_of(name: &str) -> Option<IpAddr> {
    let output = Command::new("ip")
        .args(["-4", "-o", "addr", "show", "dev", name])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for token in text.split_whitespace() {
        if let Some((ip, _)) = token.split_once('/') {
            if let Ok(ip) = ip.parse::<IpAddr>() {
                return Some(ip);
            }
        }
    }
    None
}

#[cfg(windows)]
fn platform_list() -> Vec<Outlet> {
    let script = "Get-NetIPAddress -AddressFamily IPv4 -ErrorAction SilentlyContinue | \
         ForEach-Object { Write-Output ($_.InterfaceIndex.ToString() + '|' + $_.InterfaceAlias + '|' + $_.IPAddress) }";
    let Ok(output) = powershell(script) else {
        return Vec::new();
    };
    let mut outlets = Vec::new();
    for line in output.lines() {
        let mut parts = line.trim().split('|');
        let Some(index) = parts.next().and_then(|text| text.trim().parse::<u32>().ok()) else {
            continue;
        };
        let label = parts.next().unwrap_or("").trim().to_string();
        let local_ip = parts
            .next()
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .and_then(|text| text.parse::<IpAddr>().ok());
        if label.is_empty() {
            continue;
        }
        outlets.push(Outlet {
            label,
            index,
            local_ip,
        });
    }
    outlets
}

#[cfg(target_os = "linux")]
fn platform_list() -> Vec<Outlet> {
    let mut outlets = Vec::new();
    let Ok(entries) = std::fs::read_dir("/sys/class/net") else {
        return outlets;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let Ok(text) = std::fs::read_to_string(entry.path().join("ifindex")) else {
            continue;
        };
        let Ok(index) = text.trim().parse::<u32>() else {
            continue;
        };
        let local_ip = local_ipv4_of(&name);
        outlets.push(Outlet {
            label: name,
            index,
            local_ip,
        });
    }
    outlets
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_detect() -> Result<Outlet, String> {
    Err(t(
        "分流器的直连出口目前只在 Windows 与 Linux 上实现",
        "The splitter's direct outlet is only implemented on Windows and Linux",
    )
    .to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn platform_list() -> Vec<Outlet> {
    Vec::new()
}

#[cfg(windows)]
fn powershell(script: &str) -> Result<String, io::Error> {
    use std::os::windows::process::CommandExt;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        // CREATE_NO_WINDOW：否则每次检测都会闪一个黑框。
        .creation_flags(0x0800_0000)
        .output()?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn next_query_id() -> u16 {
    // 不引随机数库：时间戳低位足够区分同时进行的几条查询。
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.subsec_nanos())
        .unwrap_or(0);
    (nanos & 0xFFFF) as u16
}

/// 拼一个最小的 DNS 查询报文。
fn build_query(id: u16, host: &str, ipv4: bool) -> Vec<u8> {
    let mut packet = Vec::with_capacity(64);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00]); // 标准查询 + 递归请求
    packet.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    packet.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT
    for label in host.trim_end_matches('.').split('.') {
        let bytes = label.as_bytes();
        let len = bytes.len().min(63);
        packet.push(len as u8);
        packet.extend_from_slice(&bytes[..len]);
    }
    packet.push(0);
    packet.extend_from_slice(&(if ipv4 { 1u16 } else { 28u16 }).to_be_bytes()); // A / AAAA
    packet.extend_from_slice(&1u16.to_be_bytes()); // IN
    packet
}

/// 从响应里取出 A / AAAA 记录。
fn parse_response(packet: &[u8]) -> io::Result<Vec<IpAddr>> {
    if packet.len() < 12 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            t("DNS 回应太短", "The DNS reply is too short"),
        ));
    }
    let rcode = packet[3] & 0x0F;
    if rcode == 3 {
        // NXDOMAIN 不算错误，只是没有结果：调用方会退回走代理。
        return Ok(Vec::new());
    }
    if rcode != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            tr_format!(
                "DNS 查询失败（rcode {rcode}）",
                "The DNS query failed (rcode {rcode})"
            ),
        ));
    }
    let questions = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answers = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12usize;
    for _ in 0..questions {
        offset = skip_name(packet, offset)?;
        offset += 4; // QTYPE + QCLASS
        if offset > packet.len() {
            return Ok(Vec::new());
        }
    }
    let mut found = Vec::new();
    for _ in 0..answers {
        offset = skip_name(packet, offset)?;
        if offset + 10 > packet.len() {
            break;
        }
        let record_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        if offset + length > packet.len() {
            break;
        }
        match (record_type, length) {
            (1, 4) => {
                let ip = Ipv4Addr::new(
                    packet[offset],
                    packet[offset + 1],
                    packet[offset + 2],
                    packet[offset + 3],
                );
                found.push(IpAddr::V4(ip));
            }
            (28, 16) => {
                let mut bytes = [0u8; 16];
                bytes.copy_from_slice(&packet[offset..offset + 16]);
                found.push(IpAddr::V6(Ipv6Addr::from(bytes)));
            }
            _ => {}
        }
        offset += length;
    }
    Ok(found)
}

/// 跳过域名（可能是压缩指针）。
fn skip_name(packet: &[u8], mut offset: usize) -> io::Result<usize> {
    loop {
        let Some(&byte) = packet.get(offset) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                t("DNS 报文被截断", "The DNS packet is truncated"),
            ));
        };
        if byte == 0 {
            return Ok(offset + 1);
        }
        if byte & 0xC0 == 0xC0 {
            // 压缩指针：两个字节，之后就是记录正文。
            return Ok(offset + 2);
        }
        offset += 1 + byte as usize;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_carry_the_expected_qtype() {
        let query = build_query(0x1234, "www.example.com", true);
        assert_eq!(&query[0..2], &[0x12, 0x34]);
        assert_eq!(&query[12..16], b"\x03www");
        assert_eq!(&query[query.len() - 4..query.len() - 2], &[0x00, 0x01]);

        let query = build_query(1, "example.com", false);
        assert_eq!(&query[query.len() - 4..query.len() - 2], &[0x00, 0x1C]);
    }

    #[test]
    fn parses_an_a_record() {
        // 手工拼一个回答：1 个问题 + 1 条 A 记录。
        let mut packet = vec![0x00, 0x01, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00];
        packet.extend_from_slice(b"\x03www\x07example\x03com\x00");
        packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        packet.extend_from_slice(&[0xC0, 0x0C]); // 指向问题里的名字
        packet.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        packet.extend_from_slice(&[0x00, 0x00, 0x00, 0x3C]);
        packet.extend_from_slice(&[0x00, 0x04]);
        packet.extend_from_slice(&[10, 1, 2, 3]);

        let found = parse_response(&packet).expect("应当解析成功");
        assert_eq!(found, vec![IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))]);
    }

    #[test]
    fn nxdomain_is_not_an_error() {
        let packet = vec![0x00, 0x01, 0x81, 0x83, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
        assert!(parse_response(&packet).unwrap().is_empty());
    }

    #[test]
    fn manual_hints_accept_an_index_or_a_name() {
        let outlet = manual("17").expect("数字当索引");
        assert_eq!(outlet.index, 17);
        assert_eq!(outlet.label, "17");

        // 名字必须是本机真实存在的网卡；找不到时要给可读的错误，而不是 panic。
        let missing = manual("绝对不会存在的网卡名");
        assert!(missing.is_err());
    }

    #[test]
    fn dial_honours_the_timeout() {
        // 203.0.113.0/24 是文档保留地址：能路由出去，但永远没人应答。
        // 它要么立刻报错，要么就一直等到我们给的超时——重点是**不能**等到系统
        // 自己的 SYN 重传结束（Windows 约 21 秒），那样一条不通的直连就会把
        // 整条连接卡住，界面上的「8 秒超时」也就成了空话。
        let Ok(outlet) = detect("") else {
            // 没有默认路由的机器（CI 容器）不用跑这条。
            return;
        };
        let started = std::time::Instant::now();
        let _ = outlet.dial("203.0.113.9:81".parse().expect("常量地址"), Duration::from_millis(200));
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "拨号没有遵守超时：等了 {elapsed:?}"
        );
    }
}
