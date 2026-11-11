//! 分流器「真跑」自检：真的起服务端、真的发 SOCKS5 请求，把判定结果和日志打出来。
//!
//! ```text
//! cargo test --offline live_split -- --ignored --nocapture
//! ```
//!
//! 它做三件事：
//!
//! 1. 起一个**假上游**（本机 SOCKS5 服务端：只记录 CONNECT 的目标，然后回显数据），
//!    代替真实代理——「上游收到了这条 CONNECT」就是「走了代理」的硬证据；
//! 2. 用 [`Library::builtin`] + [`SplitSettings`] 起**真的** [`SplitService`]，
//!    和程序启动时是同一份代码，不是复制出来的判定逻辑；
//! 3. 扮演 tun2proxy 往分流器发 SOCKS5 CONNECT，一条用例一条连接，把判定结论、
//!    分流器自己打的日志、计数器增量都打出来。
//!
//! # 为什么不是「真 TUN」
//!
//! 起完整隧道要管理员权限（TUN 网卡）和已安装的 tun2proxy，这两个条件在开发机上
//! 未必成立。这个自检什么都不需要，覆盖的正是分流器这一层——tun2proxy 收到的
//! CONNECT 就是从这里进来的，「国内直连 / 国外走代理」的结论在这一层就已经定了。
//!
//! # 顺带验证
//!
//! * **单行开关**：把 `suffix:qq.com` 放进 `disabled` 后热更新，`www.qq.com` 必须改走代理；
//! * **自定义规则**：`proxy:www.baidu.com` 必须盖住内置的「国内域名」清单；
//! * **热更新**：上面两步都走 [`SplitService::reload`]，不重启监听。
//!
//! 判定日志本身只打前 20 条（整机流量下不能每条都打），所以用例数要控制在 20 以内。

use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::ipset::Cidr;
use super::library::Library;
use super::rules::{Matcher, RuleSet};
use super::service::{CounterSnapshot, SplitService, SplitSink, StartPlan};
use super::settings::{SplitGroup, SplitSettings};
use super::upstream::Upstream;

/// 期望的判定结果。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    Direct,
    Proxy,
}

impl Expect {
    fn label(self) -> &'static str {
        match self {
            Expect::Direct => "直连",
            Expect::Proxy => "代理",
        }
    }
}

/// 一条用例。
struct Case {
    label: &'static str,
    /// `host:port`，域名和 IP 混着写。
    target: String,
    expect: Expect,
    /// 目标本身会回显数据吗（本机回声服务 / 假上游会，真实站点不会）。
    echo: bool,
}

/// 一条用例跑完之后看到的东西。
struct Outcome {
    label: String,
    target: String,
    expect: Expect,
    /// 分流器日志里能读出的判定结论（`"直连"` / `"代理"` / 判定直连但改走代理）。
    decided: String,
    /// SOCKS5 应答码，0x00 表示隧道打通。
    rep: u8,
    /// 数据能不能真的来回（`None` = 这条用例没测）。
    echo_ok: Option<bool>,
    /// 假上游这一轮有没有收到这条 CONNECT。
    hit_upstream: bool,
    /// 计数器这一轮怎么动的（`直连+1` / `代理+1` / `失败+1`）。
    counter: String,
    /// 佐证：分流器自己打的那行日志。
    evidence: String,
}

/// 日志收集器：既能被断言，也直接打到 stdout（配合 `--nocapture` 就是界面里的那份日志）。
#[derive(Default)]
struct Recorder {
    lines: Mutex<Vec<String>>,
}

impl SplitSink for Recorder {
    fn log(&self, line: String) {
        println!("    [分流器] {line}");
        if let Ok(mut lines) = self.lines.lock() {
            lines.push(line);
        }
    }
}

impl Recorder {
    /// 取走这一轮攒下的日志，并把收集窗口清空。
    fn take(&self) -> Vec<String> {
        match self.lines.lock() {
            Ok(mut lines) => std::mem::take(&mut *lines),
            Err(_) => Vec::new(),
        }
    }
}

/// 假上游：本机 SOCKS5 服务端，记录 CONNECT 目标后回显数据。
struct FakeProxy {
    addr: SocketAddr,
    seen: Arc<Mutex<Vec<String>>>,
}

impl FakeProxy {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("假上游要能监听本机端口");
        let addr = listener.local_addr().expect("假上游要能拿到监听地址");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let record = record.clone();
                std::thread::spawn(move || serve_upstream(stream, record));
            }
        });
        Self { addr, seen }
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().map(|list| list.clone()).unwrap_or_default()
    }
}

fn serve_upstream(mut stream: TcpStream, seen: Arc<Mutex<Vec<String>>>) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    // 问候：分流器的上游客户端只报「无认证」。
    let mut head = [0u8; 2];
    if stream.read_exact(&mut head).is_err() || head[0] != 0x05 {
        return;
    }
    let mut methods = [0u8; 255];
    if stream
        .read_exact(&mut methods[..head[1] as usize])
        .is_err()
    {
        return;
    }
    if stream.write_all(&[0x05, 0x00]).is_err() {
        return;
    }
    // 请求：只支持 CONNECT，其余一律拒。
    let mut request = [0u8; 4];
    if stream.read_exact(&mut request).is_err() {
        return;
    }
    if request[1] != 0x01 {
        let _ = stream.write_all(&[0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
        return;
    }
    let Some(target) = read_target(&mut stream, request[3]) else {
        return;
    };
    if let Ok(mut list) = seen.lock() {
        list.push(target);
    }
    let _ = stream.write_all(&[0x05, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0]);
    // 回显：客户端能读到自己的字节，说明整条隧道（分流器 → 上游 → 中继）是通的。
    let mut buffer = [0u8; 1024];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(size) => {
                if stream.write_all(&buffer[..size]).is_err() {
                    break;
                }
            }
        }
    }
}

/// 读出 SOCKS5 请求里的目标，写成和 [`super::upstream::Target::describe`] 一样的形状。
fn read_target(stream: &mut TcpStream, address_type: u8) -> Option<String> {
    let host = match address_type {
        0x01 => {
            let mut octets = [0u8; 4];
            stream.read_exact(&mut octets).ok()?;
            IpAddr::V4(Ipv4Addr::from(octets)).to_string()
        }
        0x04 => {
            let mut octets = [0u8; 16];
            stream.read_exact(&mut octets).ok()?;
            IpAddr::V6(std::net::Ipv6Addr::from(octets)).to_string()
        }
        0x03 => {
            let mut length = [0u8; 1];
            stream.read_exact(&mut length).ok()?;
            let mut name = vec![0u8; length[0] as usize];
            stream.read_exact(&mut name).ok()?;
            String::from_utf8_lossy(&name).to_string()
        }
        _ => return None,
    };
    let mut port = [0u8; 2];
    stream.read_exact(&mut port).ok()?;
    Some(format!("{host}:{}", u16::from_be_bytes(port)))
}

/// 本机回声服务：绑在 0.0.0.0 上，但对外用「本机出站地址」访问，
/// 这样它落在局域网网段里（命中「本地网络」清单），而不是被回环判定拦下。
struct Echo {
    addr: SocketAddr,
}

impl Echo {
    fn start() -> Option<Self> {
        let ip = local_outbound_ip()?;
        let listener = TcpListener::bind("0.0.0.0:0").ok()?;
        let port = listener.local_addr().ok()?.port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                std::thread::spawn(move || {
                    let mut buffer = [0u8; 1024];
                    loop {
                        match stream.read(&mut buffer) {
                            Ok(0) | Err(_) => break,
                            Ok(size) => {
                                if stream.write_all(&buffer[..size]).is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });
        Some(Self {
            addr: SocketAddr::new(ip, port),
        })
    }
}

/// 本机出站时用的那个地址。连一个不可路由的文档保留地址不会真的发包，
/// 只是让系统按默认路由挑出口。
fn local_outbound_ip() -> Option<IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("203.0.113.1:80").ok()?;
    Some(socket.local_addr().ok()?.ip())
}

/// 扮演 tun2proxy：一条 SOCKS5 CONNECT，返回应答码。
fn probe(proxy: SocketAddr, target: &str, echo: bool) -> Result<(u8, Option<bool>), String> {
    let (host, port) = split_target(target)?;
    let mut stream = TcpStream::connect(proxy)
        .map_err(|err| format!("连不上分流器 {proxy}：{err}"))?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));

    stream
        .write_all(&[0x05, 0x01, 0x00])
        .map_err(|err| format!("发问候失败：{err}"))?;
    let mut greeting = [0u8; 2];
    stream
        .read_exact(&mut greeting)
        .map_err(|err| format!("读问候回应失败：{err}"))?;
    if greeting[1] != 0x00 {
        return Err(format!("分流器选了不支持的认证方式 0x{:02X}", greeting[1]));
    }

    let mut request = vec![0x05, 0x01, 0x00];
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ip)) => {
            request.push(0x01);
            request.extend_from_slice(&ip.octets());
        }
        Ok(IpAddr::V6(ip)) => {
            request.push(0x04);
            request.extend_from_slice(&ip.octets());
        }
        Err(_) => {
            let bytes = host.as_bytes();
            request.push(0x03);
            request.push(bytes.len() as u8);
            request.extend_from_slice(bytes);
        }
    }
    request.extend_from_slice(&port.to_be_bytes());
    stream
        .write_all(&request)
        .map_err(|err| format!("发 CONNECT 失败：{err}"))?;

    let mut head = [0u8; 4];
    stream
        .read_exact(&mut head)
        .map_err(|err| format!("等分流器应答超时/失败：{err}"))?;
    let skip = match head[3] {
        0x01 => 4 + 2,
        0x04 => 16 + 2,
        0x03 => {
            let mut length = [0u8; 1];
            stream
                .read_exact(&mut length)
                .map_err(|err| format!("读地址类型失败：{err}"))?;
            length[0] as usize + 2
        }
        other => return Err(format!("应答里的地址类型 0x{other:02X} 不认识")),
    };
    let mut tail = vec![0u8; skip];
    stream
        .read_exact(&mut tail)
        .map_err(|err| format!("读应答地址失败：{err}"))?;

    let mut echo_ok = None;
    if echo && head[1] == 0x00 {
        let payload = b"live-split-ping";
        let mut back = vec![0u8; payload.len()];
        let ok = stream.write_all(payload).is_ok()
            && stream.read_exact(&mut back).is_ok()
            && back == payload;
        echo_ok = Some(ok);
    }
    Ok((head[1], echo_ok))
}

fn split_target(target: &str) -> Result<(String, u16), String> {
    let (host, port) = target
        .rsplit_once(':')
        .ok_or_else(|| format!("目标 {target} 少了端口"))?;
    let port = port
        .parse::<u16>()
        .map_err(|err| format!("目标 {target} 的端口看不懂：{err}"))?;
    Ok((host.to_string(), port))
}

/// 从分流器这一轮打的日志里读出判定结论。
fn classify(lines: &[String], target: &str) -> (String, String) {
    let mut fallback = None;
    for line in lines.iter().filter(|line| line.contains(target)) {
        if line.contains("判定为直连") || line.contains("is direct but no outlet") {
            fallback = Some("直连（无出口，改走代理）".to_string());
            continue;
        }
        if line.contains("无法解析") || line.contains("could not be resolved") {
            fallback = Some("直连（解析失败，改走代理）".to_string());
            continue;
        }
        if line.starts_with("直连 ") || line.starts_with("direct ") {
            return ("直连".to_string(), line.clone());
        }
        if line.starts_with("代理 ") || line.starts_with("proxy ") {
            return ("代理".to_string(), line.clone());
        }
    }
    match fallback {
        Some(decided) => (decided.clone(), decided),
        // 日志只打前 20 条，超出的用例读不到结论——这时候只能靠假上游和应答码。
        None => ("（日志未覆盖）".to_string(), String::new()),
    }
}

/// 计数器这一轮怎么动的——比日志更难伪造。
fn counter_delta(before: CounterSnapshot, after: CounterSnapshot) -> String {
    let mut parts: Vec<String> = Vec::new();
    if after.direct > before.direct {
        parts.push(format!("直连+{}", after.direct - before.direct));
    }
    if after.proxy > before.proxy {
        parts.push(format!("代理+{}", after.proxy - before.proxy));
    }
    if after.failed > before.failed {
        parts.push(format!("失败+{}", after.failed - before.failed));
    }
    if parts.is_empty() {
        "无".to_string()
    } else {
        parts.join(" ")
    }
}

fn run_cases(
    service: &SplitService,
    recorder: &Recorder,
    addr: SocketAddr,
    fake: &FakeProxy,
    cases: &[Case],
) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    for case in cases {
        let _ = recorder.take();
        let seen_before = fake.seen().len();
        let counters_before = service.counters();
        let probed = probe(addr, &case.target, case.echo);
        let lines = recorder.take();
        let seen_after = fake.seen();
        let counter = counter_delta(counters_before, service.counters());

        let (decided, evidence) = classify(&lines, &case.target);
        let (rep, echo_ok) = match probed {
            Ok((rep, echo_ok)) => (rep, echo_ok),
            Err(reason) => {
                // 客户端没走完：照样记一行，交给最后统一报错。
                outcomes.push(Outcome {
                    label: case.label.to_string(),
                    target: case.target.clone(),
                    expect: case.expect,
                    decided: format!("客户端出错：{reason}"),
                    rep: 0xFF,
                    echo_ok: None,
                    hit_upstream: false,
                    counter,
                    evidence,
                });
                continue;
            }
        };
        let hit_upstream = seen_after
            .iter()
            .skip(seen_before.min(seen_after.len()))
            .any(|item| item == &case.target);
        outcomes.push(Outcome {
            label: case.label.to_string(),
            target: case.target.clone(),
            expect: case.expect,
            decided,
            rep,
            echo_ok,
            hit_upstream,
            counter,
            evidence,
        });
    }
    outcomes
}

fn report(outcomes: &[Outcome], failures: &mut Vec<String>) {
    println!();
    println!(
        "{} {} {} {} {} {} {}",
        pad("用例", 24),
        pad("目标", 28),
        pad("期望", 4),
        pad("实测", 6),
        pad("应答", 8),
        pad("计数", 8),
        "证据"
    );
    println!("{}", "-".repeat(128));
    for outcome in outcomes {
        let reply = match outcome.rep {
            0x00 => "0x00 通".to_string(),
            0x05 => "0x05 失败".to_string(),
            0xFF => "无".to_string(),
            other => format!("0x{other:02X}"),
        };
        println!(
            "{} {} {} {} {} {} {}",
            pad(&outcome.label, 24),
            pad(&outcome.target, 28),
            pad(outcome.expect.label(), 4),
            pad(&outcome.decided, 6),
            pad(&reply, 8),
            pad(&outcome.counter, 8),
            if outcome.evidence.is_empty() {
                "（这一轮没有判定日志）"
            } else {
                outcome.evidence.as_str()
            }
        );

        if outcome.decided != outcome.expect.label() {
            failures.push(format!(
                "{}（{}）：期望 {}，实际 {}",
                outcome.label, outcome.target, outcome.expect.label(), outcome.decided
            ));
            continue;
        }
        match outcome.expect {
            Expect::Proxy => {
                if !outcome.hit_upstream {
                    failures.push(format!(
                        "{}（{}）：判定为代理，但假上游没收到这条 CONNECT",
                        outcome.label, outcome.target
                    ));
                }
            }
            Expect::Direct => {
                if outcome.hit_upstream {
                    failures.push(format!(
                        "{}（{}）：判定为直连，却仍然发给了假上游",
                        outcome.label, outcome.target
                    ));
                }
            }
        }
        if let Some(false) = outcome.echo_ok {
            failures.push(format!(
                "{}（{}）：隧道打通了但数据没能来回",
                outcome.label, outcome.target
            ));
        }
    }
    println!("{}", "-".repeat(128));
}

fn pad(text: &str, width: usize) -> String {
    let mut out = text.to_string();
    let mut used: usize = text.chars().map(display_width).sum();
    while used < width {
        out.push(' ');
        used += 1;
    }
    out
}

/// 中文字符占两格，否则表格对不齐。
fn display_width(character: char) -> usize {
    match character as u32 {
        0x1100..=0x115F | 0x2E80..=0xA4CF | 0xAC00..=0xD7A3 | 0xF900..=0xFAFF | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60 | 0xFFE0..=0xFFE6 => 2,
        _ => 1,
    }
}

/// 国内 IP 用例：从内置的「国内 IP」分组里取第一条 IPv4 网段的网络地址。
/// 不写死公网地址——清单换了，用例自动跟着换。
fn first_china_ip(library: &Library) -> Option<String> {
    library
        .group_rules(SplitGroup::ChinaIp)
        .iter()
        .find_map(|rule| match &rule.matcher {
            Matcher::Cidr(Cidr::V4 { net, .. }) => {
                Some(format!("{}:80", IpAddr::V4(Ipv4Addr::from(*net))))
            }
            _ => None,
        })
}

#[test]
#[ignore = "会真起分流器、发真实连接：cargo test --offline live_split -- --ignored --nocapture"]
fn live_split_walkthrough() {
    let library = Library::builtin();
    println!("== 内置清单 ==");
    println!(
        "本地网络 {} 条 / 国内 IP {} 条 / 国内域名 {} 条，合计 {} 条",
        library.group_len(SplitGroup::LocalNetworks),
        library.group_len(SplitGroup::ChinaIp),
        library.group_len(SplitGroup::ChinaDomain),
        library.total_len()
    );

    let settings = SplitSettings {
        enabled: true,
        ..SplitSettings::default()
    };
    let rules = library
        .direct_rules(&settings)
        .expect("默认设置应当能编译出规则");
    let (rule_set, stats) = RuleSet::build(&rules);
    println!(
        "按设置筛出 {} 条 → 编译后 {} 条（网段 {}、后缀 {}、精确 {}、关键字 {}）",
        rules.len(),
        stats.total(),
        stats.direct_cidrs + stats.proxy_cidrs,
        stats.suffix,
        stats.exact,
        stats.keyword
    );

    let fake = FakeProxy::start();
    let echo = Echo::start().expect("本机回声服务要能起来");
    let recorder = Arc::new(Recorder::default());
    let sink: Arc<dyn SplitSink> = recorder.clone();
    let service = SplitService::new();
    let addr = service
        .start(
            StartPlan {
                listen: "127.0.0.1:0".parse().expect("监听地址"),
                upstream: Upstream::parse(&format!("socks5://{}", fake.addr))
                    .expect("假上游地址要能解析"),
                rules: rule_set,
                rule_count: rules.len(),
                direct_dns: settings.direct_dns_addr().expect("默认直连 DNS"),
                outlet_hint: settings.direct_interface.clone(),
                fake_pool: None,
            },
            sink,
        )
        .expect("分流器要能起来");
    println!("分流器监听 {addr}，假上游 {}", fake.addr);
    println!("本机回声服务 {}", echo.addr);
    println!();

    let china_ip = first_china_ip(&library).expect("内置清单里必须有国内 IPv4 网段");
    let mut failures: Vec<String> = Vec::new();

    println!("== 第一轮：内置清单按默认设置全部生效 ==");
    let mut cases = vec![
        Case {
            label: "国内域名 · 百度",
            target: "www.baidu.com:443".to_string(),
            expect: Expect::Direct,
            echo: false,
        },
        Case {
            label: "国内域名 · 腾讯",
            target: "www.qq.com:443".to_string(),
            expect: Expect::Direct,
            echo: false,
        },
        Case {
            label: "国内域名 · 哔哩哔哩",
            target: "www.bilibili.com:443".to_string(),
            expect: Expect::Direct,
            echo: false,
        },
        Case {
            label: "国内 IP · 内置网段首条",
            target: china_ip.clone(),
            expect: Expect::Direct,
            echo: false,
        },
        Case {
            label: "局域网 · 本机回声服务",
            target: echo.addr.to_string(),
            expect: Expect::Direct,
            echo: true,
        },
        Case {
            label: "国外域名 · Google",
            target: "www.google.com:443".to_string(),
            expect: Expect::Proxy,
            echo: true,
        },
        Case {
            label: "国外域名 · GitHub",
            target: "github.com:443".to_string(),
            expect: Expect::Proxy,
            echo: true,
        },
        Case {
            label: "国外域名 · YouTube",
            target: "www.youtube.com:443".to_string(),
            expect: Expect::Proxy,
            echo: true,
        },
        Case {
            label: "国外 IP · 文档保留地址",
            target: "203.0.113.10:443".to_string(),
            expect: Expect::Proxy,
            echo: true,
        },
    ];
    let first = run_cases(&service, &recorder, addr, &fake, &cases);
    report(&first, &mut failures);

    println!();
    println!("== 第二轮：单独关掉 suffix:qq.com 这一行（热更新，不重启监听）==");
    let mut rows_off = settings.clone();
    rows_off.disabled = vec!["suffix:qq.com".to_string()];
    let off_rules = library
        .direct_rules(&rows_off)
        .expect("关掉一行之后规则仍要能编译");
    service
        .reload(RuleSet::build(&off_rules).0, off_rules.len())
        .expect("热更新要成功");
    println!(
        "关掉一行后规则 {} 条（比上一轮少 {} 条）",
        off_rules.len(),
        rules.len().saturating_sub(off_rules.len())
    );
    cases = vec![Case {
        label: "行开关关闭 · 腾讯",
        target: "www.qq.com:443".to_string(),
        expect: Expect::Proxy,
        echo: true,
    }];
    let second = run_cases(&service, &recorder, addr, &fake, &cases);
    report(&second, &mut failures);

    println!();
    println!("== 第三轮：自定义规则 proxy:www.baidu.com 盖住内置清单 ==");
    let mut custom = settings.clone();
    custom.custom = "proxy:www.baidu.com".to_string();
    let custom_rules = library
        .direct_rules(&custom)
        .expect("自定义规则要能编译");
    service
        .reload(RuleSet::build(&custom_rules).0, custom_rules.len())
        .expect("热更新要成功");
    cases = vec![Case {
        label: "自定义 proxy · 百度",
        target: "www.baidu.com:443".to_string(),
        expect: Expect::Proxy,
        echo: true,
    }];
    let third = run_cases(&service, &recorder, addr, &fake, &cases);
    report(&third, &mut failures);

    let counters = service.counters();
    println!();
    println!(
        "== 计数器 ==\n连接 {} 条：直连 {} / 代理 {} / 失败 {}",
        counters.sessions, counters.direct, counters.proxy, counters.failed
    );
    println!(
        "假上游一共收到 {} 条 CONNECT：{}",
        fake.seen().len(),
        fake.seen().join(", ")
    );

    service.stop();
    assert!(
        failures.is_empty(),
        "以下用例与预期不符：\n{}",
        failures.join("\n")
    );
}
