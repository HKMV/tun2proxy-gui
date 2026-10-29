//! 上游代理连通性检测。
//!
//! 「填了地址但根本没通」是最常见的配置错误：端口写错、代理没开、防火墙拦了。
//! 这些情况 tun2proxy 只会表现为「启动了但上不了网」，很难判断是谁的问题。
//! 这里在保存/启动之前先自己握一次手，把结论直接说清楚。
//!
//! 检测分两步，能过哪步说哪步：
//!
//! 1. **TCP 连接**：解析 `scheme://host:port`，3 秒内能否建立连接。
//! 2. **协议握手**：按 scheme 走一次真实握手——
//!    - `socks5` / `socks5h`：发 `VER=5, NMETHODS=1, METHOD=0x00`，
//!      期望回 `0x05 0x00`；回 `0xFF` 说明代理要求认证。
//!    - `http` / `https`：发 `CONNECT` 请求，期望状态行里是 `200`。
//!    - 其它 scheme（`ss`、`vmess` 之类）只做 TCP 连接，不做握手。
//!
//! 全程同步阻塞（3 秒超时），所以调用方必须把它放到后台线程里，
//! 见 [`crate::core::engine::Engine::check_proxy`]。

use crate::core::i18n::t;
use crate::tr_format;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::{Duration, Instant};

/// 一次检测的结论。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    pub ok: bool,
    /// 成功时是「握手成功 + 耗时」，失败时是具体原因。
    pub detail: String,
    pub millis: u128,
}

impl ProbeReport {
    fn ok(millis: u128, note: &str) -> Self {
        Self {
            ok: true,
            detail: format!("{note}，{millis} ms"),
            millis,
        }
    }

    fn fail(detail: impl Into<String>) -> Self {
        Self {
            ok: false,
            detail: detail.into(),
            millis: 0,
        }
    }
}

/// 检测一个代理地址是否可用。
pub fn check(url: &str) -> ProbeReport {
    let Some(target) = Target::parse(url) else {
        return ProbeReport::fail(t(
            "地址格式不对，应形如 socks5://127.0.0.1:7890",
            "Malformed address; expected something like socks5://127.0.0.1:7890",
        ));
    };
    let addr = format!("{}:{}", target.host, target.port);
    let Ok(mut addrs) = addr.to_socket_addrs() else {
        return ProbeReport::fail(tr_format!("解析不出 {addr}", "Cannot resolve {addr}"));
    };
    let Some(socket) = addrs.next() else {
        return ProbeReport::fail(tr_format!(
            "{addr} 没有可用地址",
            "{addr} has no usable address"
        ));
    };

    let started = Instant::now();
    let timeout = Duration::from_secs(3);
    let stream = match TcpStream::connect_timeout(&socket, timeout) {
        Ok(stream) => stream,
        Err(err) => {
            return ProbeReport::fail(tr_format!(
                "连不上 {addr}：{}",
                "Cannot connect to {addr}: {}",
                describe(&err)
            ));
        }
    };
    let millis = started.elapsed().as_millis();
    let _ = stream.set_read_timeout(Some(timeout));
    let _ = stream.set_write_timeout(Some(timeout));

    match target.scheme {
        Scheme::Socks5 => socks5_handshake(stream, millis),
        Scheme::Http => http_handshake(stream, millis),
        // 其它协议（ss / vmess / trojan…）没有通用的探测方式：
        // 端口通了就说明「地址是对的、进程在跑」，这已经能排掉大部分错误。
        Scheme::Other(name) => ProbeReport::ok(
            millis,
            &tr_format!(
                "端口可达（{name} 不做协议握手）",
                "Port reachable ({name} does no protocol handshake)"
            ),
        ),
    }
}

/// SOCKS5 无认证握手。
fn socks5_handshake(mut stream: TcpStream, millis: u128) -> ProbeReport {
    if stream.write_all(&[0x05, 0x01, 0x00]).is_err() {
        return ProbeReport::fail(t(
            "SOCKS5：发送握手包失败",
            "SOCKS5: failed to send the handshake",
        ));
    }
    let mut reply = [0u8; 2];
    if stream.read_exact(&mut reply).is_err() {
        return ProbeReport::fail(t(
            "SOCKS5：代理没有回应握手（不是 SOCKS5，或被防火墙拦了）",
            "SOCKS5: the proxy did not answer the handshake (not SOCKS5, or blocked by a firewall)",
        ));
    }
    match reply {
        [0x05, 0x00] => ProbeReport::ok(millis, t("SOCKS5 握手成功", "SOCKS5 handshake succeeded")),
        [0x05, 0xFF] => ProbeReport::fail(t(
            "SOCKS5：代理要求用户名密码认证",
            "SOCKS5: the proxy requires username/password authentication",
        )),
        [0x05, method] => ProbeReport::fail(tr_format!(
            "SOCKS5：代理只接受认证方式 0x{method:02X}",
            "SOCKS5: the proxy only accepts auth method 0x{method:02X}"
        )),
        other => ProbeReport::fail(tr_format!(
            "SOCKS5：回应不是 SOCKS5 协议（0x{:02X} 0x{:02X}）",
            "SOCKS5: the reply is not the SOCKS5 protocol (0x{:02X} 0x{:02X})",
            other[0],
            other[1]
        )),
    }
}

/// HTTP 代理：发一个 `CONNECT` 看有没有 `200`。
///
/// 用 `CONNECT` 而不是 `GET`：隧道代理（tun2proxy 需要的那种）支持 CONNECT，
/// 而 `GET` 在部分代理上会被改写成明文请求，测不出隧道能力。
fn http_handshake(mut stream: TcpStream, millis: u128) -> ProbeReport {
    let request = "CONNECT www.example.com:443 HTTP/1.1\r\nHost: www.example.com:443\r\n\r\n";
    if stream.write_all(request.as_bytes()).is_err() {
        return ProbeReport::fail(t("HTTP：发送 CONNECT 失败", "HTTP: failed to send CONNECT"));
    }
    let mut buffer = [0u8; 64];
    let read = match stream.read(&mut buffer) {
        Ok(0) => {
            return ProbeReport::fail(t(
                "HTTP：代理直接关掉了连接",
                "HTTP: the proxy closed the connection",
            ))
        }
        Ok(read) => read,
        Err(_) => {
            return ProbeReport::fail(t(
                "HTTP：代理没有回应 CONNECT（不是 HTTP 代理？）",
                "HTTP: the proxy did not answer CONNECT (not an HTTP proxy?)",
            ))
        }
    };
    let head = String::from_utf8_lossy(&buffer[..read]);
    let status = head.lines().next().unwrap_or_default().to_string();
    if status.contains(" 200") {
        ProbeReport::ok(
            millis,
            t("HTTP 隧道握手成功", "HTTP tunnel handshake succeeded"),
        )
    } else if status.contains(" 407") {
        ProbeReport::fail(t(
            "HTTP：代理要求认证（407）",
            "HTTP: the proxy requires authentication (407)",
        ))
    } else if status.is_empty() {
        ProbeReport::fail(t(
            "HTTP：回应里没有状态行",
            "HTTP: the reply has no status line",
        ))
    } else {
        ProbeReport::fail(tr_format!(
            "HTTP：代理拒绝 CONNECT（{status}）",
            "HTTP: the proxy rejected CONNECT ({status})"
        ))
    }
}

/// 把 `io::Error` 翻译成人能读的一句话。
fn describe(err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::ConnectionRefused => {
            t("端口没有在监听", "Nothing is listening on that port").to_string()
        }
        std::io::ErrorKind::TimedOut => t("连接超时", "Connection timed out").to_string(),
        std::io::ErrorKind::NetworkUnreachable => {
            t("网络不可达", "Network unreachable").to_string()
        }
        _ => err.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Socks5,
    Http,
    Other(&'static str),
}

struct Target {
    scheme: Scheme,
    host: String,
    port: u16,
}

impl Target {
    /// 解析 `scheme://[user:pass@]host:port[/path]`。
    ///
    /// 用户名密码直接丢掉：检测用的是「能不能连上」，不是「凭据对不对」，
    /// 带认证的代理会在握手阶段明确报出来。
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let (scheme_text, rest) = match raw.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
            // 不带 scheme 时按 SOCKS5 处理：tun2proxy 的默认协议。
            None => ("socks5".to_string(), raw),
        };
        let scheme = match scheme_text.as_str() {
            "socks5" | "socks5h" => Scheme::Socks5,
            "http" | "https" => Scheme::Http,
            "ss" => Scheme::Other("shadowsocks"),
            "vmess" => Scheme::Other("vmess"),
            "trojan" => Scheme::Other("trojan"),
            _ => Scheme::Other(t("未知协议", "Unknown protocol")),
        };
        // 去掉路径、查询串、凭据。
        let rest = rest.split('/').next().unwrap_or(rest);
        let rest = rest.rsplit('@').next().unwrap_or(rest);
        // IPv6 字面量：[::1]:1080
        let (host, port) = if let Some(stripped) = rest.strip_prefix('[') {
            let (host, tail) = stripped.split_once(']')?;
            let port = tail.strip_prefix(':')?;
            (host.to_string(), port)
        } else {
            let (host, port) = rest.rsplit_once(':')?;
            (host.to_string(), port)
        };
        let port = port.parse::<u16>().ok()?;
        if host.is_empty() {
            return None;
        }
        Some(Self { scheme, host, port })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shapes_people_actually_type() {
        let target = Target::parse("socks5://127.0.0.1:7890").expect("socks5");
        assert_eq!(target.scheme, Scheme::Socks5);
        assert_eq!((target.host.as_str(), target.port), ("127.0.0.1", 7890));

        let target = Target::parse("http://user:pass@10.0.0.2:8080/").expect("http with creds");
        assert_eq!(target.scheme, Scheme::Http);
        assert_eq!((target.host.as_str(), target.port), ("10.0.0.2", 8080));

        let target = Target::parse("127.0.0.1:1080").expect("scheme defaults to socks5");
        assert_eq!(target.scheme, Scheme::Socks5);

        let target = Target::parse("[::1]:1080").expect("ipv6");
        assert_eq!(target.host, "::1");

        assert!(Target::parse("").is_none());
        assert!(Target::parse("127.0.0.1").is_none());
        assert!(Target::parse("socks5://host:notaport").is_none());
    }

    #[test]
    fn reports_a_refused_port_as_a_failure() {
        // 端口 1 在这个测试环境里不会有人监听。
        let report = check("socks5://127.0.0.1:1");
        assert!(!report.ok, "{}", report.detail);
        assert!(report.detail.contains("127.0.0.1:1"), "{}", report.detail);
    }

    #[test]
    fn reports_a_malformed_address_instead_of_panicking() {
        let report = check("nonsense");
        assert!(!report.ok);
        assert!(
            report.detail.contains(t("格式", "Format")),
            "{}",
            report.detail
        );
    }
}
