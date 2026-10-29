//! 用户配置：多套「代理配置」的持久化、校验，以及转换成 tun2proxy 命令行参数。
//!
//! 分两层：
//!
//! * [`Config`] —— 应用级设置：一堆 [`Profile`]、当前选中的是第几套、以及
//!   「自动检查更新」这类与具体代理无关的选项。
//! * [`Profile`] —— 一套代理配置：上游代理地址、DNS 策略、MTU、绕过规则……
//!   用户可以保存多套（家里 / 公司 / 临时节点）随时切换。
//!
//! 点「启动」时由 [`Profile::cli_args`] 把选中的那套翻译成 tun2proxy 的
//! 命令行参数，因此参数映射只在一处维护。

use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::i18n::LangSetting;
use tproxy_config::IpCidr;

use super::i18n::t;
use crate::core::paths;
use crate::tr_format;

/// DNS 查询处理策略，对应 CLI 的 `--dns <strategy>`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DnsMode {
    /// 交给隧道转发给 `--dns-addr` 指定的解析器（最省事）。
    Direct,
    /// Fake-IP：本地直接应答，把域名映射进虚拟地址池。
    Virtual,
    /// DNS 查询改走 TCP，绕过只拦截 UDP 53 的网络。
    OverTcp,
}

impl DnsMode {
    pub const ALL: [DnsMode; 3] = [DnsMode::Direct, DnsMode::Virtual, DnsMode::OverTcp];

    pub fn label(self) -> &'static str {
        match self {
            DnsMode::Direct => t("直连", "Direct"),
            DnsMode::Virtual => t("虚拟 IP", "Virtual IP"),
            DnsMode::OverTcp => t("走 TCP", "Over TCP"),
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            DnsMode::Direct => "direct",
            DnsMode::Virtual => "virtual",
            DnsMode::OverTcp => "over-tcp",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            DnsMode::Direct => t(
                "查询经隧道转发到上面的 DNS 服务器。",
                "Queries are forwarded through the tunnel to the DNS server above.",
            ),
            DnsMode::Virtual => t(
                "本地立即应答，域名映射进虚拟池后再交给代理。",
                "Answered locally; domains map into the virtual pool before reaching the proxy.",
            ),
            DnsMode::OverTcp => t(
                "改用 TCP 转发，可绕过只拦 UDP 53 的网络。",
                "Forwards over TCP, which bypasses networks that block UDP 53.",
            ),
        }
    }
}

/// 日志级别，对应 CLI 的 `--verbosity <level>`。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verbosity {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl Verbosity {
    pub const ALL: [Verbosity; 5] = [
        Verbosity::Error,
        Verbosity::Warn,
        Verbosity::Info,
        Verbosity::Debug,
        Verbosity::Trace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Verbosity::Error => t("错误", "Error"),
            Verbosity::Warn => t("警告", "Warning"),
            Verbosity::Info => t("信息", "Info"),
            Verbosity::Debug => t("调试", "Debug"),
            Verbosity::Trace => t("详细", "Trace"),
        }
    }

    pub fn flag(self) -> &'static str {
        match self {
            Verbosity::Error => "error",
            Verbosity::Warn => "warn",
            Verbosity::Info => "info",
            Verbosity::Debug => "debug",
            Verbosity::Trace => "trace",
        }
    }
}

/// 默认绕过的地址段：局域网与各类保留地址。
///
/// 这些地址本来就不该进代理：TUN 接管全局流量后，如果不绕过它们，
/// 打印机会连不上、NAS 找不到、投屏卡住，而且内网流量绕一圈上游代理再回来，
/// 既慢又可能被上游直接丢弃。
///
/// 每行一个 CIDR（`--bypass` 逐行透传）。不要在这里写注释：校验会把它当成
/// 非法地址拒绝。
pub const DEFAULT_BYPASS: &str = "10.0.0.0/8
172.16.0.0/12
192.168.0.0/16
127.0.0.0/8
169.254.0.0/16
100.64.0.0/10
224.0.0.0/4";

/// 一套代理配置。
///
/// 字段默认值与 tun2proxy CLI 的默认值保持一致；反序列化时容忍缺字段
/// （`#[serde(default)]`），这样旧版本配置文件升级后不会丢设置。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    /// 显示在配置列表里的名字，仅用于区分，不传给 tun2proxy。
    pub name: String,
    pub proxy_url: String,
    pub dns: DnsMode,
    pub dns_addr: String,
    pub virtual_dns_pool: String,
    /// 绕过地址，多行文本，每行一个 IP 或 CIDR。
    pub bypass: String,
    pub tun_name: String,
    pub mtu: u16,
    pub tcp_timeout: u64,
    pub udp_timeout: u64,
    pub max_sessions: usize,
    pub verbosity: Verbosity,
    pub udpgw_server: String,
    pub udpgw_connections: usize,
    pub udpgw_keepalive: u64,
    pub ipv6: bool,
    pub setup: bool,
    pub unshare: bool,
    /// 发生致命错误时立即退出（作为系统服务运行时有用）。
    pub exit_on_fatal: bool,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: t("默认配置", "Default profile").to_string(),
            proxy_url: "socks5://127.0.0.1:1080".to_string(),
            dns: DnsMode::Direct,
            dns_addr: "8.8.8.8".to_string(),
            virtual_dns_pool: "198.18.0.0/15".to_string(),
            bypass: DEFAULT_BYPASS.to_string(),
            tun_name: String::new(),
            mtu: 1500,
            tcp_timeout: 600,
            udp_timeout: 10,
            max_sessions: 200,
            verbosity: Verbosity::Info,
            udpgw_server: String::new(),
            udpgw_connections: 5,
            udpgw_keepalive: 30,
            ipv6: false,
            setup: true,
            unshare: false,
            exit_on_fatal: true,
        }
    }
}

impl Profile {
    /// 列表里显示的一行摘要（代理地址 + DNS 策略）。
    pub fn summary(&self) -> String {
        let proxy = if self.proxy_url.trim().is_empty() {
            t("（未填写代理地址）", "(no proxy address)").to_string()
        } else {
            self.proxy_url.trim().to_string()
        };
        format!("{proxy} · DNS {}", self.dns.label())
    }

    /// 在把配置交给 CLI 之前做一次体检，尽早给出可读的中文错误。
    pub fn validate(&self) -> Result<(), String> {
        let proxy = self.proxy_url.trim();
        if proxy.is_empty() {
            return Err(t("请填写代理地址", "Enter a proxy address").to_string());
        }
        let lower = proxy.to_ascii_lowercase();
        if !(lower.starts_with("socks5://")
            || lower.starts_with("socks4://")
            || lower.starts_with("http://"))
        {
            return Err(t(
                "代理地址需要以 socks5:// 、socks4:// 或 http:// 开头",
                "The proxy address must start with socks5://, socks4:// or http://",
            )
            .to_string());
        }

        if self.dns_addr.trim().parse::<std::net::IpAddr>().is_err() {
            return Err(t(
                "DNS 服务器需要填写 IP 地址",
                "The DNS server must be an IP address",
            )
            .to_string());
        }

        if IpCidr::from_str(self.virtual_dns_pool.trim()).is_err() {
            return Err(t(
                "虚拟 DNS 池需要填写 CIDR，例如 198.18.0.0/15",
                "The virtual DNS pool must be a CIDR, e.g. 198.18.0.0/15",
            )
            .to_string());
        }

        for line in self.bypass_lines() {
            if IpCidr::from_str(line).is_err() {
                return Err(tr_format!(
                    "绕过地址「{line}」不是合法的 IP 或 CIDR",
                    "Bypass entry “{line}” is not a valid IP or CIDR"
                ));
            }
        }

        if !self.udpgw_server.trim().is_empty()
            && self
                .udpgw_server
                .trim()
                .parse::<std::net::SocketAddr>()
                .is_err()
        {
            return Err(t(
                "UDP 网关服务器需要填写 IP:端口",
                "The UDP gateway must be IP:port",
            )
            .to_string());
        }

        #[cfg(target_os = "linux")]
        {
            if !self.setup && !self.unshare && self.tun_name.trim().is_empty() {
                return Err(
                    t("Linux 上未开启「自动配置路由与系统设置」时必须填写网卡名称，或改用用户命名空间", "On Linux, with automatic route setup off you must name the interface or use user namespaces")
                        .to_string(),
                );
            }
        }

        Ok(())
    }

    /// 绕过列表为空时填入默认的局域网与保留地址段；已经有内容就不动。
    ///
    /// 只在**完全为空**时补，不在「和默认值不同」时覆盖：清空列表是「什么都
    /// 不绕过」的合法意图，不能被悄悄改回去。
    pub fn fill_bypass_defaults(&mut self) {
        if self.bypass.trim().is_empty() {
            self.bypass = DEFAULT_BYPASS.to_string();
        }
    }

    /// 多行文本框里的绕过地址，去掉空行与首尾空白。
    pub fn bypass_lines(&self) -> impl Iterator<Item = &str> {
        self.bypass
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    }

    /// 转换成 tun2proxy 的命令行参数（不包含可执行文件本身）。
    ///
    /// 映射规则集中在这里，界面层只负责维护 [`Profile`] 的字段值。
    pub fn cli_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        fn push(args: &mut Vec<String>, flag: &str, value: &str) {
            args.push(flag.to_string());
            args.push(value.to_string());
        }

        push(&mut args, "--proxy", self.proxy_url.trim());
        push(&mut args, "--dns", self.dns.flag());
        push(&mut args, "--dns-addr", self.dns_addr.trim());
        push(
            &mut args,
            "--virtual-dns-pool",
            self.virtual_dns_pool.trim(),
        );

        for line in self.bypass_lines() {
            args.push("--bypass".to_string());
            args.push(line.to_string());
        }

        if !self.tun_name.trim().is_empty() {
            push(&mut args, "--tun", self.tun_name.trim());
        }
        if self.ipv6 {
            args.push("--ipv6-enabled".to_string());
        }
        push(&mut args, "--mtu", &self.mtu.to_string());
        push(&mut args, "--tcp-timeout", &self.tcp_timeout.to_string());
        push(&mut args, "--udp-timeout", &self.udp_timeout.to_string());
        push(&mut args, "--max-sessions", &self.max_sessions.to_string());
        push(&mut args, "--verbosity", self.verbosity.flag());

        if !self.udpgw_server.trim().is_empty() {
            push(&mut args, "--udpgw-server", self.udpgw_server.trim());
            push(
                &mut args,
                "--udpgw-connections",
                &self.udpgw_connections.to_string(),
            );
            push(
                &mut args,
                "--udpgw-keepalive",
                &self.udpgw_keepalive.to_string(),
            );
        }

        #[cfg(target_os = "linux")]
        if self.unshare {
            args.push("--unshare".to_string());
        }

        if self.exit_on_fatal {
            args.push("--exit-on-fatal-error".to_string());
        }

        let needs_setup = self.setup || (cfg!(target_os = "linux") && self.unshare);
        if needs_setup {
            args.push("--setup".to_string());
        }

        args
    }
}

/// 应用级配置：多套代理配置 + 当前选中的下标 + 与代理无关的选项。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub profiles: Vec<Profile>,
    /// 当前使用的配置下标，始终指向 `profiles` 里的有效元素。
    pub active: usize,
    pub auto_check_update: bool,
    pub release_repo: String,
    /// 界面语言。
    pub language: LangSetting,
    /// 点标题栏的叉时，先问「要不要最小化到托盘」而不是直接退出。
    ///
    /// 默认开：tun2proxy 是常驻服务，误关窗口会把正在跑的隧道一起停掉。
    /// 关掉之后就按「关窗即最小化到托盘」处理，退出走托盘菜单。
    pub confirm_close_to_tray: bool,
    /// 配置文件格式版本，用来做一次性迁移（见 [`Config::migrate`]）。
    ///
    /// 注意 `#[serde(default = ...)]` 而不是跟着容器走：容器级的
    /// `#[serde(default)]` 是拿 `Config::default()` 当底，缺字段时会直接填成
    /// [`CONFIG_VERSION`]，迁移就永远不会跑了。这里必须显式回到 0。
    #[serde(default = "missing_config_version")]
    pub config_version: u32,
}

/// 旧文件里没有 `config_version` 这个键，一律当成 0（= 需要迁移）。
fn missing_config_version() -> u32 {
    0
}

/// 当前配置文件格式版本。
///
/// * 1 —— 初始格式；
/// * 2 —— `exit_on_fatal` 默认改为开。
const CONFIG_VERSION: u32 = 2;

impl Default for Config {
    fn default() -> Self {
        Self {
            profiles: vec![Profile::default()],
            active: 0,
            auto_check_update: true,
            release_repo: "tun2proxy/tun2proxy".to_string(),
            language: LangSetting::Auto,
            confirm_close_to_tray: true,
            config_version: CONFIG_VERSION,
        }
    }
}

impl Config {
    /// 读取配置文件；文件不存在或损坏时回退到默认值。
    ///
    /// 兼容旧版「单配置」文件格式（字段直接平铺在根对象上），
    /// 读到时会自动包装成一套名为「默认配置」的配置。
    pub fn load() -> Self {
        let path = paths::config_path();
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => return Config::default(),
        };
        let value: serde_json::Value = match serde_json::from_str(&text) {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "{}",
                    tr_format!(
                        "配置文件 {} 解析失败：{err}",
                        "Failed to parse the config file {}: {err}",
                        path.display()
                    )
                );
                return Config::default();
            }
        };

        let mut config = if value.get("profiles").is_some() {
            serde_json::from_value::<Config>(value).unwrap_or_default()
        } else {
            // 旧格式：整份文件就是一套配置。
            let profile: Profile = serde_json::from_value(value.clone()).unwrap_or_default();
            let mut config = Config {
                profiles: vec![profile],
                // 旧格式比 config_version 还老，强制当成 0，让迁移跑一遍。
                config_version: 0,
                ..Config::default()
            };
            if let Some(flag) = value.get("auto_check_update").and_then(|v| v.as_bool()) {
                config.auto_check_update = flag;
            }
            if let Some(repo) = value.get("release_repo").and_then(|v| v.as_str()) {
                config.release_repo = repo.to_string();
            }
            config
        };
        config.normalize();
        // 迁移改的是「用户没显式设过的东西」，所以必须立刻落盘：
        // 否则每次启动都会把用户手动关掉的选项再改回来。
        if config.migrate() {
            let _ = config.save();
        }
        config
    }

    /// 把旧版本的配置升级到 [`CONFIG_VERSION`]，返回是否真的改过。
    ///
    /// 只做「用户没有表态」的字段。`exit_on_fatal` 在旧版里是显式的
    /// `false`（序列化写进去的），无法与新版的「用户主动关掉」区分，
    /// 所以只能靠版本号判断：v2 之前一律改成开，之后完全听用户的。
    fn migrate(&mut self) -> bool {
        if self.config_version >= CONFIG_VERSION {
            return false;
        }
        if self.config_version < 2 {
            for profile in &mut self.profiles {
                profile.exit_on_fatal = true;
            }
        }
        self.config_version = CONFIG_VERSION;
        true
    }

    /// 写回配置文件，返回实际写入路径。
    pub fn save(&self) -> Result<PathBuf, String> {
        let path = paths::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                tr_format!(
                    "无法创建 {}：{err}",
                    "Cannot create {}: {err}",
                    parent.display()
                )
            })?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| {
            tr_format!(
                "配置序列化失败：{err}",
                "Failed to serialize the config: {err}"
            )
        })?;
        std::fs::write(&path, text).map_err(|err| {
            tr_format!(
                "无法写入 {}：{err}",
                "Cannot write {}: {err}",
                path.display()
            )
        })?;
        Ok(path)
    }

    /// 保证「至少有一套配置、下标有效、名字非空、绕过列表不空」。
    pub fn normalize(&mut self) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default());
        }
        if self.active >= self.profiles.len() {
            self.active = 0;
        }
        for (index, profile) in self.profiles.iter_mut().enumerate() {
            if profile.name.trim().is_empty() {
                profile.name = tr_format!("配置 {}", "Profile {}", index + 1);
            }
            // 绕过列表为空 = 一个地址都不绕过：TUN 接管全局后打印机、NAS、
            // 投屏全部失效，而这恰恰是旧版本留下的默认值（也解释了为什么
            // 界面上那个多行框一直是空的）。读盘时补上，用户不必先去点
            // 「填入默认值」才能正常用。
            profile.fill_bypass_defaults();
        }
    }

    /// 当前选中的那套配置（`normalize` 保证一定存在）。
    pub fn active_profile(&self) -> &Profile {
        &self.profiles[self.active.min(self.profiles.len() - 1)]
    }

    /// 当前选中的那套配置的可变引用。
    pub fn active_profile_mut(&mut self) -> &mut Profile {
        let index = self.active.min(self.profiles.len() - 1);
        &mut self.profiles[index]
    }

    /// 切换到第 `index` 套配置，越界时忽略。
    pub fn select(&mut self, index: usize) {
        if index < self.profiles.len() {
            self.active = index;
        }
    }

    /// 新增一套配置（复制当前选中项的设置，名字自动编号），返回新下标。
    ///
    /// 绕过列表为空时补上默认值：一套没有绕过规则的配置，开起来会把内网
    /// 流量也塞进上游代理（打印机、NAS、投屏全断），而这恰好是新手第一次
    /// 点「新建」时最可能撞上的状态。
    pub fn add_profile(&mut self) -> usize {
        let mut profile = self.active_profile().clone();
        profile.name = self.next_name();
        profile.fill_bypass_defaults();
        self.profiles.push(profile);
        self.active = self.profiles.len() - 1;
        self.active
    }

    /// 复制当前配置（名字加「副本」），返回新下标。
    pub fn duplicate_active(&mut self) -> usize {
        let mut profile = self.active_profile().clone();
        profile.name = tr_format!("{} 副本", "{} copy", profile.name.trim());
        self.profiles.push(profile);
        self.active = self.profiles.len() - 1;
        self.active
    }

    /// 删除第 `index` 套配置；只剩一套时拒绝删除。返回是否真的删掉了。
    ///
    /// 删的不是当前项时，当前下标要往前挪一格，否则会默默指向别的那套。
    pub fn remove(&mut self, index: usize) -> bool {
        if self.profiles.len() <= 1 || index >= self.profiles.len() {
            return false;
        }
        self.profiles.remove(index);
        if index < self.active {
            self.active -= 1;
        } else if self.active >= self.profiles.len() {
            self.active = self.profiles.len() - 1;
        }
        true
    }

    /// 生成一个不与现有名字冲突的默认名。
    fn next_name(&self) -> String {
        let mut index = self.profiles.len() + 1;
        loop {
            let candidate = tr_format!("配置 {index}", "Profile {index}");
            if !self.profiles.iter().any(|p| p.name == candidate) {
                return candidate;
            }
            index += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_management_keeps_state_valid() {
        let mut config = Config::default();
        assert_eq!(config.profiles.len(), 1);
        assert!(!config.remove(0));

        let index = config.add_profile();
        assert_eq!(index, 1);
        assert_eq!(config.profiles.len(), 2);
        assert_ne!(config.profiles[0].name, config.profiles[1].name);

        config.select(0);
        assert_eq!(config.active, 0);
        assert!(config.remove(0));
        assert_eq!(config.active, 0);
        assert_eq!(config.profiles.len(), 1);
    }

    #[test]
    fn old_config_files_switch_exit_on_fatal_on() {
        // v1 的文件里 exit_on_fatal 是显式的 false，靠 serde default 改不动它，
        // 只能靠 config_version 判断「这条设置用户到底表过态没有」。
        let old = r#"{
            "profiles": [{ "name": "默认配置", "proxy_url": "socks5://127.0.0.1:1080",
                           "exit_on_fatal": false }],
            "active": 0
        }"#;
        let mut config: Config = serde_json::from_str(old).expect("v1 config must still parse");
        assert_eq!(config.config_version, 0, "缺字段时必须落到 0，迁移才会跑");
        assert!(config.migrate());
        assert!(config.profiles[0].exit_on_fatal);
        assert_eq!(config.config_version, CONFIG_VERSION);

        // 迁移只跑一次：之后用户自己关掉，再读盘不许被改回来。
        config.profiles[0].exit_on_fatal = false;
        let saved = serde_json::to_string(&config).unwrap();
        let mut reloaded: Config = serde_json::from_str(&saved).unwrap();
        assert!(!reloaded.migrate());
        assert!(!reloaded.profiles[0].exit_on_fatal);
    }

    #[test]
    fn normalize_fills_an_empty_bypass_list() {
        // 旧版本存下来的配置里 bypass 是空字符串；读盘后应该补上默认段，
        // 否则界面上的多行框一直是空的，而空列表意味着内网也走代理。
        let mut config = Config::default();
        config.active_profile_mut().bypass = "   \n ".to_string();
        config.normalize();
        assert_eq!(config.active_profile().bypass, DEFAULT_BYPASS);
    }

    #[test]
    fn a_new_profile_gets_default_bypass_when_the_source_has_none() {
        let mut config = Config::default();
        // 用户把当前配置的绕过列表清空后，再点「新建」：新配置不能是空的，
        // 否则新建完直接启动就会把内网流量也塞进上游代理。
        config.active_profile_mut().bypass.clear();
        let index = config.add_profile();
        assert_eq!(config.profiles[index].bypass, DEFAULT_BYPASS);

        // 已经填过的不能被覆盖。
        config.profiles[index].bypass = "10.9.0.0/16".to_string();
        let index = config.add_profile();
        assert_eq!(config.profiles[index].bypass, "10.9.0.0/16");
    }

    #[test]
    fn cli_args_follow_profile() {
        let profile = Profile {
            proxy_url: "http://127.0.0.1:7890".to_string(),
            dns: DnsMode::Virtual,
            bypass: "192.168.0.0/16\n\n10.0.0.0/8\n".to_string(),
            ..Profile::default()
        };
        let args = profile.cli_args();
        assert_eq!(args[0], "--proxy");
        assert_eq!(args[1], "http://127.0.0.1:7890");
        assert!(args.windows(2).any(|w| w == ["--dns", "virtual"]));
        assert_eq!(args.iter().filter(|a| *a == "--bypass").count(), 2);
    }

    #[test]
    fn removing_an_earlier_profile_moves_the_selection_with_it() {
        let mut config = Config::default();
        config.add_profile();
        config.add_profile();
        // 当前是第 2 套；删掉第 0 套后，第 2 套变成第 1 套，选中不能跑掉。
        config.select(2);
        assert!(config.remove(0));
        assert_eq!(config.active, 1);
        assert_eq!(config.profiles.len(), 2);

        // 删掉自己（下标越界）应该被拒绕。
        assert!(!config.remove(9));
        assert_eq!(config.active, 1);
    }
}
