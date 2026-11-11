//! 清单生成 / 体检工具。
//!
//! 三个子命令：
//!
//! * `generate` —— 从 APNIC 记录生成 `assets/rules/china-ipv4.txt`、
//!   `china-ipv6.txt`，从 v2fly 清单生成 `china-domain.txt`；
//! * `report` —— 不写文件，只把清单规模、索引构建耗时、查询耗时和一张匹配
//!   结果表打出来（验收、排查「为什么这个站走了代理」都用它）；
//! * `check` —— 不联网、不读文件，体检**已经嵌进二进制的那份快照**
//!   （`src/core/split/builtin.rs`），发版前跑它。
//!
//! # 为什么用 `#[path]` 而不是复制一份代码
//!
//! 生成清单用的聚合逻辑必须和程序运行时用的**完全一致**：两边各写一份，早晚
//! 会出现「工具算出来 3879 条、程序里跑出来 3882 条」这种没人能解释的偏差。
//! 这里直接包含 `src/core/split` 下的源码，编译期就绑死了。
//!
//! 用法与数据来源见同目录 `README.md`。

// 复用过来的模块里有若干「程序运行时会用、这个工具暂时用不到」的方法。
#![allow(dead_code)]

#[path = "../../../src/core/split/builtin.rs"]
mod builtin;
#[path = "../../../src/core/split/ipset.rs"]
mod ipset;
#[path = "../../../src/core/split/lists.rs"]
mod lists;
#[path = "../../../src/core/split/rules.rs"]
mod rules;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use ipset::{Cidr, IpSet};
use lists::{address_count, china_ip_from_apnic, expand_domain_lists, render_cidr_snapshot, render_domain_snapshot};
use rules::{Action, Rule, RuleSet};


fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = argv.first().cloned() else {
        usage();
        return ExitCode::FAILURE;
    };
    if command == "check" {
        // check 不读任何文件，所以不解析那些必填参数。
        return match check() {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("失败：{err}");
                ExitCode::FAILURE
            }
        };
    }
    let opts = match Opts::parse(&argv[1..]) {
        Ok(opts) => opts,
        Err(err) => {
            eprintln!("参数有误：{err}");
            usage();
            return ExitCode::FAILURE;
        }
    };

    let result = match command.as_str() {
        "generate" => generate(&opts),
        "report" => report(&opts),
        "help" | "-h" | "--help" => {
            usage();
            Ok(())
        }
        other => {
            eprintln!("未知子命令：{other}");
            usage();
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("失败：{err}");
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!(
        "regen-rules —— 生成 / 体检内置分流清单

用法：
  regen-rules generate --apnic <delegated-apnic-latest> \\
                      [--domain tld-cn=<path>] [--domain geolocation-cn=<path>] \\
                      [--domain-dir <子清单目录>] \\
                      [--out assets/rules] [--date 2026-11-11]
  regen-rules report  --apnic <delegated-apnic-latest> [同上]
  regen-rules check   体检已嵌进二进制的那份快照（不联网、不读文件）

说明：
  * --domain 可重复，名字只用于报告里的分组统计；
  * --domain-dir 里放 v2fly 的子清单（文件名 = include 名），`include:` 会去那里找；
    找不到的会统计成「未下载」，工具自己**不联网**；
  * report 不写任何文件。"
    );
}

struct Opts {
    apnic: PathBuf,
    domains: Vec<(String, PathBuf)>,
    /// 子清单目录：`include:xxx` 会来这里找 `xxx` 文件。
    domain_dir: Option<PathBuf>,
    out: PathBuf,
    date: String,
}

impl Opts {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut apnic = None;
        let mut domains = Vec::new();
        let mut domain_dir = None;
        let mut out = PathBuf::from("assets/rules");
        let mut date = "未注明".to_string();

        let mut index = 0;
        while index < args.len() {
            let key = args[index].as_str();
            let value = args
                .get(index + 1)
                .ok_or_else(|| format!("{key} 后面缺一个值"))?;
            match key {
                "--apnic" => apnic = Some(PathBuf::from(value)),
                "--domain" => {
                    let (name, path) = value
                        .split_once('=')
                        .ok_or_else(|| "--domain 的写法是 名字=路径".to_string())?;
                    domains.push((name.to_string(), PathBuf::from(path)));
                }
                "--domain-dir" => domain_dir = Some(PathBuf::from(value)),
                "--out" => out = PathBuf::from(value),
                "--date" => date = value.clone(),
                other => return Err(format!("不认识的参数 {other}")),
            }
            index += 2;
        }

        Ok(Self {
            apnic: apnic.ok_or_else(|| "缺少 --apnic".to_string())?,
            domains,
            domain_dir,
            out,
            date,
        })
    }
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|err| format!("读不到 {}：{err}", path.display()))
}

/// 解析输入，产出「国内 IP 规则 + 国内域名规则 + 统计」。
struct Parsed {
    ip_rules: Vec<Rule>,
    domain_rules: Vec<Rule>,
    ip_note: String,
    domain_note: String,
}

fn parse_inputs(opts: &Opts) -> Result<Parsed, String> {
    let apnic = read(&opts.apnic)?;
    let china = china_ip_from_apnic(&apnic);

    let ip_rules: Vec<Rule> = china
        .v4
        .iter()
        .copied()
        .chain(china.v6.iter().copied())
        .map(|cidr| Rule::new(Action::Direct, rules::Matcher::Cidr(cidr), "国内 IP"))
        .collect();
    let ip_note = format!(
        "APNIC 记录 {} 条（IPv4 {} / IPv6 {}），跳过：状态 {}、类型 {} → 聚合为 IPv4 {} 条（{} 个地址）、IPv6 {} 条",
        china.stats.records,
        china.stats.ipv4_records,
        china.stats.ipv6_records,
        china.stats.skipped_status,
        china.stats.skipped_type,
        china.v4.len(),
        address_count(&china.v4),
        china.v6.len()
    );

    // 域名清单：把给到的文件当 root；`include:` 去 `--domain-dir` 里找同名文件，
    // 找不到就只统计（工具本身不联网，谁下载由 README 里的脚本负责）。
    let mut texts: Vec<(String, String)> = opts
        .domains
        .iter()
        .map(|(name, path)| Ok((name.clone(), read(path)?)))
        .collect::<Result<_, String>>()?;
    if let (true, Some(dir)) = (texts.is_empty(), &opts.domain_dir) {
        // 只给了目录时，目录下每个文件都当一份 root 清单（文件名就是清单名）。
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|err| format!("读不到目录 {}：{err}", dir.display()))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_file())
            .collect();
        entries.sort();
        for path in entries {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            texts.push((name.to_string(), read(&path)?));
        }
    }
    let roots: Vec<(&str, &str)> = texts
        .iter()
        .map(|(name, text)| (name.as_str(), text.as_str()))
        .collect();
    let dir = opts.domain_dir.clone();
    let mut fetch = |name: &str| -> Option<String> {
        let dir = dir.as_ref()?;
        std::fs::read_to_string(dir.join(name)).ok()
    };
    let expanded = expand_domain_lists(&roots, &mut fetch, "国内域名");

    let domain_note = format!(
        "读入 {} 个文件，合并去重后 {} 条（后缀 {} / 精确 {} / 关键字 {}）；丢弃 regexp {} 条、无效 {} 条、重复 {} 条；另有 {} 个 include: 子清单未下载",
        expanded.files,
        expanded.rules.len(),
        expanded.rules.iter().filter(|rule| matches!(rule.matcher, rules::Matcher::Suffix(_))).count(),
        expanded.rules.iter().filter(|rule| matches!(rule.matcher, rules::Matcher::Exact(_))).count(),
        expanded.rules.iter().filter(|rule| matches!(rule.matcher, rules::Matcher::Keyword(_))).count(),
        expanded.regexp_skipped,
        expanded.invalid,
        expanded.duplicates,
        expanded.unresolved.len()
    );

    Ok(Parsed {
        ip_rules,
        domain_rules: expanded.rules,
        ip_note,
        domain_note,
    })
}

fn generate(opts: &Opts) -> Result<(), String> {
    let apnic = read(&opts.apnic)?;
    let china = china_ip_from_apnic(&apnic);

    let v4 = render_cidr_snapshot(
        "中国大陆 IPv4 网段（内置快照）",
        &format!(
            "来源：APNIC delegated-apnic-latest，生成于 {}；原始记录 {} 条，聚合后 {} 条",
            opts.date,
            china.stats.ipv4_records,
            china.v4.len()
        ),
        &china.v4,
    );
    let v6 = render_cidr_snapshot(
        "中国大陆 IPv6 网段（内置快照）",
        &format!(
            "来源：APNIC delegated-apnic-latest，生成于 {}；原始记录 {} 条，聚合后 {} 条",
            opts.date,
            china.stats.ipv6_records,
            china.v6.len()
        ),
        &china.v6,
    );

    let parsed = parse_inputs(opts)?;
    let domains = render_domain_snapshot(
        "中国大陆域名（内置快照）",
        &format!(
            "来源：v2fly domain-list-community 的 tld-cn / geolocation-cn，生成于 {}；只取后缀、精确、关键字三类，regexp 行丢弃",
            opts.date
        ),
        &parsed.domain_rules,
    );

    std::fs::create_dir_all(&opts.out)
        .map_err(|err| format!("创建 {} 失败：{err}", opts.out.display()))?;
    for (name, content) in [
        ("china-ipv4.txt", &v4),
        ("china-ipv6.txt", &v6),
        ("china-domain.txt", &domains),
    ] {
        let path = opts.out.join(name);
        std::fs::write(&path, content)
            .map_err(|err| format!("写 {} 失败：{err}", path.display()))?;
        println!(
            "写出 {}：{} 行 / {} 字节",
            path.display(),
            content.lines().count(),
            content.len()
        );
    }
    println!();
    println!("IP ：{}", parsed.ip_note);
    println!("域名：{}", parsed.domain_note);
    Ok(())
}

fn report(opts: &Opts) -> Result<(), String> {
    let parsed = parse_inputs(opts)?;
    println!("== 清单规模 ==");
    println!("IP ：{}", parsed.ip_note);
    println!("域名：{}", parsed.domain_note);
    println!();

    // 规则集：局域网 + 国内 IP + 国内域名，和将来的实际编译顺序一致。
    let mut lan_rules = builtin::local_rules();
    let mut all_rules = Vec::new();
    all_rules.append(&mut lan_rules);
    all_rules.extend(parsed.ip_rules.iter().cloned());
    all_rules.extend(parsed.domain_rules.iter().cloned());

    let started = Instant::now();
    let (rule_set, stats) = RuleSet::build(&all_rules);
    let build_time = started.elapsed();

    println!("== 索引构建 ==");
    println!(
        "输入 {} 条 → 编译后 {} 条（IPv4/IPv6 网段 {}、后缀 {}、精确 {}、关键字 {}），耗时 {:?}",
        all_rules.len(),
        stats.total(),
        stats.direct_cidrs + stats.proxy_cidrs,
        stats.suffix,
        stats.exact,
        stats.keyword,
        build_time
    );
    println!();

    // 查询耗时：一百万次域名判定 + 一百万次 IP 判定。
    let samples = sample_domains(&parsed.domain_rules);
    let started = Instant::now();
    let mut hits = 0usize;
    for index in 0..1_000_000usize {
        let domain = &samples[index % samples.len()];
        if rule_set.match_domain(domain).is_some() {
            hits += 1;
        }
    }
    let domain_time = started.elapsed();
    println!(
        "域名判定 100 万次：{:?}（{:.0} 万次/秒，命中 {hits} 次）",
        domain_time,
        1_000_000.0 / domain_time.as_secs_f64().max(f64::MIN_POSITIVE) / 10_000.0
    );

    let mut ip_set = IpSet::new();
    for rule in &parsed.ip_rules {
        if let rules::Matcher::Cidr(cidr) = &rule.matcher {
            ip_set.insert(*cidr);
        }
    }
    let ip_samples: Vec<u32> = parsed
        .ip_rules
        .iter()
        .filter_map(|rule| match &rule.matcher {
            rules::Matcher::Cidr(Cidr::V4 { net, .. }) => Some(*net),
            _ => None,
        })
        .take(512)
        .collect();
    let started = Instant::now();
    let mut hits = 0usize;
    for index in 0..1_000_000usize {
        let bits = ip_samples[index % ip_samples.len()];
        if ip_set.contains(std::net::IpAddr::V4(std::net::Ipv4Addr::from(bits))) {
            hits += 1;
        }
    }
    let ip_time = started.elapsed();
    println!(
        "IP  判定 100 万次：{:?}（{:.0} 万次/秒，命中 {hits} 次）",
        ip_time,
        1_000_000.0 / ip_time.as_secs_f64().max(f64::MIN_POSITIVE) / 10_000.0
    );
    println!();

    println!("== 匹配结果 ==");
    println!("{:<28} {:<8} {}", "目标", "结论", "命中");
    println!("{}", "-".repeat(78));
    for domain in [
        "baidu.com",
        "www.baidu.com",
        "map.baidu.com:443",
        "qq.com",
        "www.qq.com",
        "notqq.com",
        "www.bilibili.com",
        "www.tsinghua.edu.cn",
        "github.com",
        "www.google.com",
        "api.openai.com",
    ] {
        print_verdict(domain, rule_set.match_domain(domain));
    }
    for text in ["10.0.0.1", "192.168.1.1", "172.31.0.1"] {
        let ip: std::net::IpAddr = text.parse().unwrap();
        print_verdict(text, rule_set.match_ip(ip));
    }
    // 国内外各取一条真实网段里的地址：结论比具体地址更有意义。
    for (label, rule) in [
        ("表内第 1 条（国内 IP）", parsed.ip_rules.first()),
        ("表内最后一条（国内 IP）", parsed.ip_rules.last()),
    ] {
        if let Some(rules::Matcher::Cidr(cidr)) = rule.map(|rule| &rule.matcher) {
            let ip = match *cidr {
                Cidr::V4 { net, .. } => std::net::IpAddr::V4(std::net::Ipv4Addr::from(net)),
                Cidr::V6 { net, .. } => std::net::IpAddr::V6(std::net::Ipv6Addr::from(net)),
            };
            print_verdict(&format!("{label} @ {ip}"), rule_set.match_ip(ip));
        }
    }

    Ok(())
}

fn check() -> Result<(), String> {
    let (local, v4, v6, domain) = builtin::snapshot_size();
    println!("== 内置快照（已经编译进二进制的那一份）==");
    println!("本地网络 {local} 条 / 国内 IP {} 条（IPv4 {v4} + IPv6 {v6}）/ 国内域名 {domain} 条", v4 + v6);
    println!(
        "文本体积：IPv4 {} KB、IPv6 {} KB、域名 {} KB",
        builtin::CHINA_IPV4.len() / 1024,
        builtin::CHINA_IPV6.len() / 1024,
        builtin::CHINA_DOMAIN.len() / 1024
    );

    let all_rules = builtin::rules();
    let started = Instant::now();
    let (rule_set, stats) = RuleSet::build(&all_rules);
    println!(
        "编译 {} 条 → {} 条（网段 {}、后缀 {}、精确 {}、关键字 {}），耗时 {:?}",
        all_rules.len(),
        stats.total(),
        stats.direct_cidrs + stats.proxy_cidrs,
        stats.suffix,
        stats.exact,
        stats.keyword,
        started.elapsed()
    );
    println!();

    println!("== 匹配结果 ==");
    println!("{:<28} {:<8} {}", "目标", "结论", "命中");
    println!("{}", "-".repeat(78));
    for domain in [
        "www.baidu.com",
        "www.qq.com",
        "www.taobao.com",
        "www.bilibili.com",
        "www.163.com",
        "www.tsinghua.edu.cn",
        "notqq.com",
        "github.com",
        "www.google.com",
        "api.openai.com",
        "example.org",
    ] {
        print_verdict(domain, rule_set.match_domain(domain));
    }
    for text in ["10.0.0.1", "192.168.1.1", "172.31.0.1", "127.0.0.1", "::1"] {
        let ip: std::net::IpAddr = text.parse().map_err(|_| format!("{text} 不是地址"))?;
        print_verdict(text, rule_set.match_ip(ip));
    }
    Ok(())
}

fn print_verdict(target: &str, hit: Option<rules::Match<'_>>) {
    match hit {
        Some(hit) => {
            let verdict = match hit.action() {
                Action::Direct => "直连",
                Action::Proxy => "代理",
            };
            println!("{:<28} {:<8} {}", target, verdict, hit.describe());
        }
        None => println!("{:<28} {:<8} {}", target, "代理", "（兜底）"),
    }
}

/// 从清单里取一批域名样本：既有命中的，也有必然不命中的。
fn sample_domains(domain_rules: &[Rule]) -> Vec<String> {
    let mut samples: Vec<String> = domain_rules
        .iter()
        .filter_map(|rule| match &rule.matcher {
            rules::Matcher::Suffix(text) => Some(format!("www.{text}")),
            rules::Matcher::Exact(text) => Some(text.clone()),
            _ => None,
        })
        .take(256)
        .collect();
    if samples.is_empty() {
        samples.push("www.example.com".to_string());
    }
    samples
}
