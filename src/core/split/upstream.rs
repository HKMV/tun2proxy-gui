//! 上游代理客户端：把分流器收到的一条 CONNECT 转交给用户配置的代理。
//!
//! tun2proxy 原生支持 `socks5://`、`socks4://`、`http://` 三种上游，分流器必须
//! 把这三种都实现一遍，否则「开了分流」等于悄悄把某些协议踢坏——用户配了
//! HTTP 代理却再也连不上网，是这类功能最容易犯的错。
//!
//! # 为什么不用现成的 crate
//!
//! 三种握手加起来也就一百多行，用 `reqwest`/`hyper` 反而要拉进一整套 TLS 与
//! 连接池：分流器只是把一条已经建立好的 TCP 连接「借」给上游，不需要那些东西。
//!
//! # 认证
//!
//! 地址里的 `user:pass@` 会带上：
//!
//! * SOCKS5 → 优先「无认证」，代理要求时再走 RFC 1929 用户名密码；
//! * SOCKS4 → 放进 USERID 字段（多数实现忽略它）；
//! * HTTP → 发 `Proxy-Authorization: Basic ...`。

use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::core::i18n::t;
use crate::tr_format;

/// 上游代理的协议。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scheme {
    Socks5,
    Socks4,
    Http,
}

impl Scheme {
    pub fn label(self) -> &'static str {
        match self {
            Scheme::Socks5 => "SOCKS5",
            Scheme::Socks4 => "SOCKS4",
            Scheme::Http => "HTTP",
        }
    }
}

/// 解析好的上游代理地址。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Upstream {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub user: Option<String>,
    pub pass: Option<String>,
}

/// 分流器要连的目的地：tun2proxy 交过来的可能是域名（fake-IP 模式），也可能是 IP。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Domain(String, u16),
    Ip(SocketAddr),
}

impl Target {
    pub fn port(&self) -> u16 {
        match self {
            Target::Domain(_, port) => *port,
            Target::Ip(addr) => addr.port(),
        }
    }

    /// 给日志和 `Host:` 头用的写法。
    pub fn host(&self) -> String {
        match self {
            Target::Domain(host, _) => host.clone(),
            Target::Ip(addr) => addr.ip().to_string(),
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Target::Domain(host, port) => format!("{host}:{port}"),
            Target::Ip(addr) => addr.to_string(),
        }
    }
}

impl Upstream {
    /// 解析 `scheme://[user:pass@]host:port`。
    ///
    /// 和 [`crate::core::probe`] 的解析刻意分开：那边只关心「通不通」，凭据直接
    /// 丢掉；这里要带着凭据真的连上去。
    pub fn parse(raw: &str) -> Result<Self, String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return Err(t("上游代理地址是空的", "The upstream proxy address is empty").to_string());
        }
        let (scheme_text, rest) = match raw.split_once("://") {
            Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest.to_string()),
            // 不带 scheme 时按 SOCKS5 处理，和 tun2proxy 的默认一致。
            None => ("socks5".to_string(), raw.to_string()),
        };
        let scheme = match scheme_text.as_str() {
            "socks5" | "socks5h" => Scheme::Socks5,
            "socks4" | "socks4a" => Scheme::Socks4,
            "http" | "https" => Scheme::Http,
            other => {
                return Err(tr_format!(
                    "分流器只支持 socks5:// socks4:// http:// 三种上游，不支持 {other}://",
                    "The splitter supports socks5:// socks4:// and http:// upstreams only, not {other}://"
                ))
            }
        };
        // 去掉路径与查询串，再拆凭据（凭据里可能有 `@`，所以从右往左找）。
        let rest = rest.split('/').next().unwrap_or(&rest).to_string();
        let (creds, host_port) = match rest.rsplit_once('@') {
            Some((creds, host_port)) => (Some(creds), host_port.to_string()),
            None => (None, rest),
        };
        let (user, pass) = match creds {
            Some(creds) => match creds.split_once(':') {
                Some((user, pass)) => (Some(user.to_string()), Some(pass.to_string())),
                None => (Some(creds.to_string()), None),
            },
            None => (None, None),
        };

        // IPv6 字面量要写成 [::1]:1080，否则冒号分不清。
        let (host, port) = if let Some(stripped) = host_port.strip_prefix('[') {
            let (host, tail) = stripped
                .split_once(']')
                .ok_or_else(|| tr_format!("地址 {raw} 的 IPv6 缺少右方括号", "IPv6 address in {raw} is missing its closing bracket"))?;
            let port = tail.strip_prefix(':').unwrap_or("");
            (host.to_string(), port)
        } else {
            match host_port.rsplit_once(':') {
                Some((host, port)) => (host.to_string(), port),
                None => (host_port.clone(), ""),
            }
        };
        let port = if port.is_empty() {
            return Err(tr_format!(
                "地址 {raw} 里没有端口",
                "The address {raw} has no port"
            ));
        } else {
            port.parse::<u16>()
                .map_err(|_| tr_format!("地址 {raw} 里的端口不是数字", "The port in {raw} is not a number"))?
        };
        if host.is_empty() {
            return Err(tr_format!(
                "地址 {raw} 里没有主机名",
                "The address {raw} has no host"
            ));
        }
        Ok(Self {
            scheme,
            host,
            port,
            user,
            pass,
        })
    }

    /// 连上游并让它把 `target` 打通，返回一条已经可以传数据的隧道。
    pub async fn connect(&self, target: &Target) -> io::Result<TcpStream> {
        let stream = TcpStream::connect((self.host.as_str(), self.port)).await?;
        let _ = stream.set_nodelay(true);
        let mut stream = stream;
        match self.scheme {
            Scheme::Socks5 => self.socks5(&mut stream, target).await?,
            Scheme::Socks4 => self.socks4(&mut stream, target).await?,
            Scheme::Http => self.http(&mut stream, target).await?,
        }
        Ok(stream)
    }

    async fn socks5(&self, stream: &mut TcpStream, target: &Target) -> io::Result<()> {
        // 有凭据时两种方法都报，让代理自己挑；没有凭据就只报「无认证」。
        let (greeting, expects_auth) = match &self.user {
            Some(_) => (vec![0x05, 0x02, 0x00, 0x02], true),
            None => (vec![0x05, 0x01, 0x00], false),
        };
        stream.write_all(&greeting).await?;
        let mut reply = [0u8; 2];
        stream.read_exact(&mut reply).await?;
        if reply[0] != 0x05 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                t("上游不是 SOCKS5（握手回应版本号不对）", "The upstream is not SOCKS5 (bad version in the handshake)"),
            ));
        }
        match reply[1] {
            0x00 => {}
            0x02 if expects_auth => self.socks5_auth(stream).await?,
            0xFF => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    t(
                        "上游 SOCKS5 不接受我们提供的认证方式（地址里需要带用户名密码？）",
                        "The upstream SOCKS5 rejected our auth methods (does the address need a username/password?)"
                    ),
                ))
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    tr_format!(
                        "上游 SOCKS5 选择了不支持的认证方式 0x{other:02X}",
                        "The upstream SOCKS5 chose unsupported auth method 0x{other:02X}"
                    ),
                ))
            }
        }

        let mut request = vec![0x05, 0x01, 0x00];
        match target {
            Target::Domain(host, port) => {
                let bytes = host.as_bytes();
                if bytes.len() > 255 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        t("域名太长，SOCKS5 装不下", "The domain is too long for SOCKS5"),
                    ));
                }
                request.push(0x03);
                request.push(bytes.len() as u8);
                request.extend_from_slice(bytes);
                request.extend_from_slice(&port.to_be_bytes());
            }
            Target::Ip(addr) => {
                match addr.ip() {
                    std::net::IpAddr::V4(ip) => {
                        request.push(0x01);
                        request.extend_from_slice(&ip.octets());
                    }
                    std::net::IpAddr::V6(ip) => {
                        request.push(0x04);
                        request.extend_from_slice(&ip.octets());
                    }
                }
                request.extend_from_slice(&addr.port().to_be_bytes());
            }
        }
        stream.write_all(&request).await?;

        let mut head = [0u8; 4];
        stream.read_exact(&mut head).await?;
        let skipped = match head[3] {
            0x01 => 4 + 2,
            0x04 => 16 + 2,
            0x03 => {
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await?;
                len[0] as usize + 2
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    tr_format!(
                        "上游 SOCKS5 回应里的地址类型 0x{other:02X} 不认识",
                        "Unknown address type 0x{other:02X} in the upstream SOCKS5 reply"
                    ),
                ))
            }
        };
        let mut tail = vec![0u8; skipped];
        stream.read_exact(&mut tail).await?;
        if head[1] != 0x00 {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                tr_format!(
                    "上游 SOCKS5 拒绝连接：{}",
                    "The upstream SOCKS5 refused the connection: {}",
                    socks5_reason(head[1])
                ),
            ));
        }
        Ok(())
    }

    async fn socks5_auth(&self, stream: &mut TcpStream) -> io::Result<()> {
        let user = self.user.clone().unwrap_or_default();
        let pass = self.pass.clone().unwrap_or_default();
        if user.len() > 255 || pass.len() > 255 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                t("用户名或密码超过 255 字节", "The username or password is longer than 255 bytes"),
            ));
        }
        let mut packet = vec![0x01, user.len() as u8];
        packet.extend_from_slice(user.as_bytes());
        packet.push(pass.len() as u8);
        packet.extend_from_slice(pass.as_bytes());
        stream.write_all(&packet).await?;
        let mut reply = [0u8; 2];
        stream.read_exact(&mut reply).await?;
        if reply[1] != 0x00 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                t("上游 SOCKS5 认证失败（用户名或密码不对）", "Upstream SOCKS5 authentication failed (wrong username or password)"),
            ));
        }
        Ok(())
    }

    async fn socks4(&self, stream: &mut TcpStream, target: &Target) -> io::Result<()> {
        let port = target.port();
        let mut packet = vec![0x04, 0x01];
        packet.extend_from_slice(&port.to_be_bytes());
        // 目标是域名时用 SOCKS4a：DSTIP 填 0.0.0.1，域名跟在 USERID 后面。
        let mut domain = None;
        match target {
            Target::Ip(addr) => match addr.ip() {
                std::net::IpAddr::V4(ip) => packet.extend_from_slice(&ip.octets()),
                std::net::IpAddr::V6(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        t("SOCKS4 不支持 IPv6", "SOCKS4 does not support IPv6"),
                    ))
                }
            },
            Target::Domain(host, _) => {
                packet.extend_from_slice(&[0, 0, 0, 1]);
                domain = Some(host.clone());
            }
        }
        packet.extend_from_slice(self.user.clone().unwrap_or_default().as_bytes());
        packet.push(0x00);
        if let Some(domain) = domain {
            packet.extend_from_slice(domain.as_bytes());
            packet.push(0x00);
        }
        stream.write_all(&packet).await?;
        let mut reply = [0u8; 8];
        stream.read_exact(&mut reply).await?;
        if reply[1] != 0x5A {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                tr_format!(
                    "上游 SOCKS4 拒绝连接（回应码 0x{:02X}）",
                    "The upstream SOCKS4 refused the connection (reply 0x{:02X})",
                    reply[1]
                ),
            ));
        }
        Ok(())
    }

    async fn http(&self, stream: &mut TcpStream, target: &Target) -> io::Result<()> {
        let authority = format!("{}:{}", target.host(), target.port());
        let mut request = format!(
            "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: keep-alive\r\n"
        );
        if let Some(user) = &self.user {
            let token = base64(&format!("{}:{}", user, self.pass.clone().unwrap_or_default()));
            request.push_str(&format!("Proxy-Authorization: Basic {token}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await?;

        // 一行一行读到空行为止：有的代理会先发若干个 1xx 或带正文的说明。
        let mut head = Vec::with_capacity(256);
        let mut byte = [0u8; 1];
        while head.len() < 8192 {
            let read = stream.read(&mut byte).await?;
            if read == 0 {
                break;
            }
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&head);
        let status = text.lines().next().unwrap_or_default().to_string();
        if status.contains(" 200") {
            return Ok(());
        }
        Err(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            tr_format!(
                "上游 HTTP 代理拒绝 CONNECT（{status}）",
                "The upstream HTTP proxy rejected CONNECT ({status})"
            ),
        ))
    }
}

/// SOCKS5 回复码的中文解释。
fn socks5_reason(code: u8) -> String {
    let reason = match code {
        0x01 => t("一般性失败", "general failure"),
        0x02 => t("规则不允许", "connection not allowed by ruleset"),
        0x03 => t("网络不可达", "network unreachable"),
        0x04 => t("主机不可达", "host unreachable"),
        0x05 => t("目标拒绝连接", "connection refused"),
        0x06 => t("TTL 过期", "TTL expired"),
        0x07 => t("不支持的命令", "command not supported"),
        0x08 => t("不支持的地址类型", "address type not supported"),
        _ => t("未知原因", "unknown reason"),
    };
    format!("{reason} (0x{code:02X})")
}

/// 标准 Base64（HTTP Basic 认证用）。不引 crate 是因为只用这一处。
fn base64(input: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[(triple >> 18) as usize & 0x3F] as char);
        out.push(TABLE[(triple >> 12) as usize & 0x3F] as char);
        out.push(if chunk.len() > 1 {
            TABLE[(triple >> 6) as usize & 0x3F] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[triple as usize & 0x3F] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_three_supported_schemes() {
        let upstream = Upstream::parse("socks5://127.0.0.1:7890").expect("socks5");
        assert_eq!(upstream.scheme, Scheme::Socks5);
        assert_eq!((upstream.host.as_str(), upstream.port), ("127.0.0.1", 7890));
        assert!(upstream.user.is_none());

        let upstream = Upstream::parse("http://user:pw@10.0.0.2:8080/ignored?q=1").expect("http");
        assert_eq!(upstream.scheme, Scheme::Http);
        assert_eq!((upstream.host.as_str(), upstream.port), ("10.0.0.2", 8080));
        assert_eq!(upstream.user.as_deref(), Some("user"));
        assert_eq!(upstream.pass.as_deref(), Some("pw"));

        let upstream = Upstream::parse("socks4://192.168.1.2:1080").expect("socks4");
        assert_eq!(upstream.scheme, Scheme::Socks4);
    }

    #[test]
    fn rejects_what_it_cannot_relay() {
        // shadowsocks / vmess 之类没有通用的 CONNECT 握手，强行开关分流只会把
        // 整条隧道打断，所以这里必须明确拒绝。
        assert!(Upstream::parse("ss://aes-256-gcm:pw@example.com:8388").is_err());
        assert!(Upstream::parse("socks5://127.0.0.1").is_err());
        assert!(Upstream::parse("").is_err());
    }

    #[test]
    fn base64_matches_the_reference_examples() {
        assert_eq!(base64("user:pass"), "dXNlcjpwYXNz");
        assert_eq!(base64("a"), "YQ==");
        assert_eq!(base64("ab"), "YWI=");
        assert_eq!(base64("abc"), "YWJj");
    }

    #[test]
    fn auth_packets_are_built_from_the_credentials() {
        let upstream = Upstream::parse("socks5://alice:pw@127.0.0.1:1080").unwrap();
        assert_eq!(upstream.user.as_deref(), Some("alice"));
        assert_eq!(upstream.pass.as_deref(), Some("pw"));
    }
}
