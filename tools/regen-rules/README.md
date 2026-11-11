# regen-rules

生成并体检「国内 IP / 域名直连」用的清单。产出的三个文本文件在
[`assets/rules/`](../../assets/rules/)，编译期被 `src/core/split/builtin.rs`
用 `include_str!` 嵌进二进制。

```
regen-rules generate --apnic <delegated-apnic-latest> \
                    --domain tld-cn=<path> --domain geolocation-cn=<path> \
                    --domain-dir <子清单目录> \
                    --out assets/rules --date 2026-11-11

regen-rules report  --apnic <同上> --domain ... --domain-dir ...
regen-rules check
```

| 子命令 | 干什么 | 碰网络吗 | 写文件吗 |
|--------|--------|----------|----------|
| `generate` | 生成三个快照文件 | 否（输入是本地文件） | 是 |
| `report` | 打印清单规模、索引构建耗时、查询吞吐、匹配结果表 | 否 | 否 |
| `check` | 体检**已嵌进二进制**的那份快照 | 否 | 否 |

发版前跑 `report`（看新数据）和 `check`（看实际发出去的那份），两个都要过。

## 数据从哪来

| 文件 | 来源 | 说明 |
|------|------|------|
| `delegated-apnic-latest` | <https://ftp.apnic.net/apnic/stats/apnic/delegated-apnic-latest> | 五个 RIR 的委派记录，只取国家 `CN` 的记录 |
| `tld-cn` | <https://raw.githubusercontent.com/v2fly/domain-list-community/master/data/tld-cn> | `.cn` 等 TLD |
| `geolocation-cn` | <https://raw.githubusercontent.com/v2fly/domain-list-community/master/data/geolocation-cn> | 国内域名主清单，大量 `include:` |

两个域名文件都来自 [v2fly/domain-list-community](https://github.com/v2fly/domain-list-community)，
MIT 协议。国内 IP 段的数据源是 APNIC 的公开统计文件，无授权限制。

## 为什么要先下载子清单

`geolocation-cn` 本体只有 1754 条，里面**没有** `baidu.com`、`qq.com`、`taobao.com`、
`163.com`、`bilibili.com` —— 它们全在 158 个 `include:` 子清单里。也就是说：只解析
本体，「国内域名直连」这个功能对主要站点是完全无效的。

工具本身**不联网**（这样它能在没有网络的构建环境里跑，也永远不会因为上游抖动
导致构建失败）。下载由调用方负责，PowerShell 里长这样：

```powershell
$dir = 'tools/regen-rules/cache'
New-Item -ItemType Directory -Force -Path $dir | Out-Null
$base = 'https://raw.githubusercontent.com/v2fly/domain-list-community/master/data/'

# 第一层：geolocation-cn 直接 include 的清单
$names = (Get-Content "$dir/geolocation-cn") |
    Where-Object { $_ -match '^include:' } |
    ForEach-Object { ($_ -replace '^include:','' -split '\s+')[0] } |
    Sort-Object -Unique
$names | ForEach-Object -Parallel {
    Invoke-WebRequest ("$using:base" + $_) -OutFile (Join-Path $using:dir $_) -UseBasicParsing
} -ThrottleLimit 8

# 第二层：子清单里还会 include 别的，再扫一遍（有的名字上游已删，404 属正常）
```

实测这一轮是 158 + 173 个请求、约 174 KB、40 个名字 404（上游已删但引用还在，
`report` 里会统计成「未下载」，不影响其余规则）。

## 多久更新一次

* **IP 段**：APNIC 每天更新，实际国内段几个月才动几行，**一个月一次足够**；
* **域名**：跟着上游走，新增站点比较快，**一到两周一次**；
* 两次之间程序用内置快照跑，离线可用；将来的运行时更新会覆盖快照，不需要发版。

生成时把 `--date` 写成当天，会出现在每个文件的第三行头注释里，方便用户报问题时
一眼看出清单有多旧。

## 为什么用 `#[path]` 复用源码而不是复制

生成清单的聚合逻辑必须和程序运行时**完全一致**，两边各写一份迟早会出现
「工具算出 8592 条、程序里跑出 8595 条」这种没人能解释的偏差。`src/main.rs` 顶部：
工具因此**没有依赖**（`Cargo.toml` 里 `[dependencies]` 是空的），代价是这几个
模块被编进工具时会有 `dead_code` 告警，已在 crate 根用 `#![allow(dead_code)]` 关掉。

同一个原因，`builtin.rs` 也被 `#[path]` 包含进来：这样 `cargo test` 能直接测到
「发出去的那份快照」——包括「主要国内站点是否真的命中」「`github.com` 是否真的
不命中」。

## 当前快照规模（2026-11-11）

| 文件 | 行数 | 体积 |
|------|------|------|
| `china-ipv4.txt` | 5502（5499 条规则） | 84 KB |
| `china-ipv6.txt` | 2022（2019 条规则） | 30 KB |
| `china-domain.txt` | 8595（8592 条规则） | 113 KB |

合计 16 117 条规则；编译成规则集约 3 ms、域名判定约 740 万次/秒、IP 判定约
1000 万次/秒（见 `report` 的实测输出）。
