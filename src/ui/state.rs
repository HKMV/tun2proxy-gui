//! 应用状态与消息定义。
//!
//! 界面遵循 ZSUI 的单向数据流：控件只发出 [`Msg`]，[`crate::ui::update`]
//! 修改 [`App`]，然后整棵视图重新构建。因此这里只放**数据**，不放绘制逻辑。

use std::sync::{Arc, Mutex};

use zsui::ZsIcon;

use crate::core::engine::ProbeState;
use crate::core::i18n::{t, tf};
use crate::core::{
    Config, DnsMode, Engine, Profile, Shared, Status, TrafficSample, Update, Verbosity,
};

/// 日志区最多显示多少行（完整日志仍在内存里，最多 2000 行）。
pub const LOG_VIEW_LINES: usize = 200;

/// 左侧导航里的页面。
///
/// 分页是为了让每屏只放一组相关内容：仪表盘看状态，代理配置管多套配置，
/// 网络与绕过调参数，程序与日志管二进制、日志级别和输出。
///
/// 原本还有一个「高级选项」页（日志级别 / UDPGW / Linux 免 root）。它被拆掉了：
/// 那三项各属于「日志」「UDP 转发」「运行方式」，放在它们该在的页里更好找，
/// 一个只有三个开关的页面只会让导航多一次点击。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    /// 服务状态 + 当前配置速览。
    Dashboard,
    /// 多套代理配置的列表与代理 / DNS 参数。
    Profiles,
    /// 网络参数、UDP 转发与路由绕过规则。
    Network,
    /// tun2proxy 二进制的下载 / 更新、运行方式与运行日志。
    Program,
}

impl Page {
    /// 侧边栏里的顺序。
    pub const ALL: [Page; 4] = [
        Page::Dashboard,
        Page::Profiles,
        Page::Network,
        Page::Program,
    ];

    /// 导航按钮与页面标题用的名字。
    pub fn label(self) -> &'static str {
        match self {
            Page::Dashboard => t("仪表盘", "Dashboard"),
            Page::Profiles => t("代理配置", "Profiles"),
            Page::Network => t("网络与绕过", "Network"),
            Page::Program => t("程序与日志", "Program"),
        }
    }

    /// 侧边栏图标（由 ZSUI 映射到各平台自己的图标集）。
    ///
    /// 图标要能一眼看出「这页管什么」：
    /// 仪表盘 = 应用总览（App），代理配置 = 配置工具（Tool，扳手），
    /// 网络 = 流量出口（Send，纸飞机），程序与日志 = 时间线输出（History）。
    /// 之前用的是 Group / Edit / Tool / File：人群图标与「仪表盘」无关，
    /// 铅笔与「配置列表」无关，文档图标又和「程序」对不上。
    /// 网络页原先用 Send（纸飞机），和侧边栏底部的「启动」按钮撞图，改用 Pin。
    pub fn icon(self) -> ZsIcon {
        match self {
            Page::Dashboard => ZsIcon::App,
            Page::Profiles => ZsIcon::Tool,
            Page::Network => ZsIcon::Pin,
            Page::Program => ZsIcon::History,
        }
    }

    /// 页面标题下面的一行说明。
    pub fn subtitle(self) -> &'static str {
        match self {
            Page::Dashboard => t(
                "服务状态、当前使用的配置与最近输出。",
                "Service status, the active profile and recent output.",
            ),
            Page::Profiles => t(
                "维护多套代理配置，随时切换要用的那套。",
                "Keep several profiles and switch between them.",
            ),
            Page::Network => t(
                "TUN 网卡、UDP 转发与绕过规则。",
                "TUN adapter, UDP relay and bypass rules.",
            ),
            Page::Program => t(
                "tun2proxy 二进制的下载更新与实时日志。",
                "Binary updates and the live log.",
            ),
        }
    }
}

/// 界面上的每一次交互。
#[derive(Clone, Debug)]
pub enum Msg {
    /// 切换到左侧导航的某一页。
    PageSelected(Page),
    /// 侧边栏在「完整」与「图标栏」之间切换。
    ToggleSidebar,

    // —— 配置切换 ——
    /// 切换到第 N 套代理配置。
    ProfileSelected(usize),
    /// 开始编辑第 N 套配置（先选中它，再打开详情）。
    ProfileEditStart(usize),
    /// 从详情返回配置列表。
    ProfileEditDone,
    /// 删除第 N 套配置。
    ProfileRemovedAt(usize),
    /// 新增一套配置（复制当前设置）。
    ProfileAdded,
    /// 复制当前配置。
    ProfileDuplicated,
    /// 修改当前配置的名字。
    ProfileRenamed(String),

    // —— 代理设置 ——
    ProxyUrl(String),
    SetDns(DnsMode),
    DnsAddr(String),
    VirtualPool(String),
    BypassChanged(String),
    /// 把绕过列表填成默认的局域网 / 保留地址段。
    BypassDefaults,

    // —— 网络参数 ——
    SetIpv6(bool),
    MtuChanged(String),
    TunName(String),
    SetSetupRoutes(bool),

    // —— 高级选项 ——
    SetVerbosity(Verbosity),
    SetUnshare(bool),
    UdpgwChanged(String),
    UdpgwConnections(String),
    TcpTimeout(String),
    UdpTimeout(String),
    MaxSessions(String),
    SetExitOnFatal(bool),

    // —— 操作 ——
    Start,
    Stop,
    SaveConfig,
    ClearLogs,
    /// 以管理员身份重启自己（UAC 提权后新进程接手）。
    RelaunchElevated,
    /// 检测上游代理地址是否连得通。
    CheckProxy,
    CheckUpdate,
    InstallUpdate,

    // —— 窗口与托盘 ——
    /// 用户点了标题栏的叉。要不要真退，由这里决定。
    CloseRequested,
    /// 「要不要最小化到托盘」弹窗的回应。
    CloseDialog(zsui::ZsContentDialogResult),
    /// 修改「关闭窗口时询问」这个开关。
    SetConfirmCloseToTray(bool),
    /// 直接最小化到托盘（不弹窗）。
    MinimizeToTray,
    /// 切换界面语言（跟随系统 / 中文 / English）。
    SetLanguage(crate::core::i18n::LangSetting),
}

/// 数字输入框的原始文本。
///
/// 输入过程中允许暂时不是合法数字（用户正在删改），所以先留在草稿里，
/// 等启动/保存时再通过 [`App::apply_draft`] 解析成真正的数值。
/// 切换配置时草稿会按新配置重新填充。
#[derive(Clone, Debug)]
pub struct Draft {
    pub mtu: String,
    pub tcp_timeout: String,
    pub udp_timeout: String,
    pub max_sessions: String,
    pub udpgw_connections: String,
}

impl Draft {
    fn from_profile(profile: &Profile) -> Self {
        Self {
            mtu: profile.mtu.to_string(),
            tcp_timeout: profile.tcp_timeout.to_string(),
            udp_timeout: profile.udp_timeout.to_string(),
            max_sessions: profile.max_sessions.to_string(),
            udpgw_connections: profile.udpgw_connections.to_string(),
        }
    }
}

/// 界面渲染时读到的共享状态快照。
///
/// 一次性把锁里的数据拷出来，视图层不再碰 [`Mutex`]，避免一帧内多次加锁。
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: Status,
    pub update: Update,
    pub version: Option<String>,
    pub logs: Vec<String>,
    /// 最近一分钟的流量采样（旧→新），空表示还没开始采。
    pub traffic: Vec<TrafficSample>,
    /// 最新一次采样的瞬时速率。
    pub rate: TrafficSample,
    /// 上游代理检测状态（`None` = 还没检测过）。
    pub probe: Option<ProbeState>,
    /// 启动被拦下的原因（上游代理连不通等），由后台线程写入。
    pub blocked: Option<String>,
}

/// 应用根状态。
pub struct App {
    /// 当前显示的页面（左侧导航选中项）。
    pub page: Page,
    /// 侧边栏是否收起成「图标栏」。
    ///
    /// 收起后侧边栏只剩一列图标（宽度 [super::sidebar::RAIL_WIDTH]），
    /// 把宽度让给内容区。状态只活在本次运行里：这是个临时视角切换，
    /// 不是需要落盘的偏好。
    pub sidebar_collapsed: bool,
    /// 「代理配置」页是否处在「编辑某套配置」的详情态。
    ///
    /// 列表才是默认态：大多数时候用户是来「换一套」而不是「改参数」，
    /// 一进页就是四个输入框会把人带到错误的动作上。
    pub profile_editing: bool,
    /// 应用级配置：多套代理配置 + 当前选中的下标。
    pub config: Config,
    /// 当前配置里数字输入框的草稿。
    pub draft: Draft,
    /// tun2proxy 子进程引擎。
    pub engine: Engine,
    /// 与后台线程共享的运行时状态。
    pub shared: Arc<Mutex<Shared>>,
    /// 绿色提示（保存成功等）。
    pub notice: Option<String>,
    /// 红色错误（启动失败等）。
    pub error: Option<String>,
    /// 「关闭窗口时要不要最小化到托盘」的弹窗是否打开。
    ///
    /// 只管这一次提问：关掉窗口或做了选择就复位。真正「以后还问不问」
    /// 存在 `config.confirm_close_to_tray` 里。
    pub close_prompt: bool,
}

impl App {
    pub fn new(config: Config, engine: Engine, shared: Arc<Mutex<Shared>>) -> Self {
        let draft = Draft::from_profile(config.active_profile());
        Self {
            page: Page::Dashboard,
            sidebar_collapsed: false,
            profile_editing: false,
            config,
            draft,
            engine,
            shared,
            notice: None,
            error: None,
            close_prompt: false,
        }
    }

    /// 当前选中的那套代理配置。
    pub fn profile(&self) -> &Profile {
        self.config.active_profile()
    }

    /// 当前选中的那套代理配置（可变）。
    pub fn profile_mut(&mut self) -> &mut Profile {
        self.config.active_profile_mut()
    }

    /// 切换配置：草稿同步刷新成新配置里的数值。
    pub fn select_profile(&mut self, index: usize) {
        self.config.select(index);
        self.draft = Draft::from_profile(self.config.active_profile());
        self.error = None;
        self.notice = None;
    }

    /// 新增 / 复制配置后，草稿跟着切到新配置。
    pub fn after_profile_created(&mut self) {
        self.draft = Draft::from_profile(self.config.active_profile());
        self.error = None;
        self.notice = None;
    }

    /// 把数字输入框的草稿写回当前配置，并做一次整体校验。
    pub fn apply_draft(&mut self) -> Result<(), String> {
        let mtu = parse_number(&self.draft.mtu, "MTU", 576, 65535)?;
        let tcp = parse_number(
            &self.draft.tcp_timeout,
            t("TCP 超时", "TCP timeout"),
            1,
            86_400,
        )?;
        let udp = parse_number(
            &self.draft.udp_timeout,
            t("UDP 超时", "UDP timeout"),
            1,
            86_400,
        )?;
        let sessions = parse_number(
            &self.draft.max_sessions,
            t("最大会话数", "Max sessions"),
            1,
            100_000,
        )?;
        let udpgw = parse_number(
            &self.draft.udpgw_connections,
            t("UDPGW 连接数", "UDPGW connections"),
            1,
            1_024,
        )?;

        let profile = self.profile_mut();
        profile.mtu = mtu as u16;
        profile.tcp_timeout = tcp;
        profile.udp_timeout = udp;
        profile.max_sessions = sessions as usize;
        profile.udpgw_connections = udpgw as usize;
        profile.validate()
    }

    /// 读一份共享状态快照给视图用。
    pub fn snapshot(&self) -> Snapshot {
        let shared = self.shared.lock().expect("shared state poisoned");
        Snapshot {
            status: shared.status.clone(),
            update: shared.update.clone(),
            version: shared.version.clone(),
            logs: shared.log_tail(LOG_VIEW_LINES),
            traffic: shared.traffic.iter().copied().collect(),
            rate: shared.rate,
            probe: shared.probe.clone(),
            blocked: shared.start_blocked.clone(),
        }
    }
}

/// 解析一个带范围检查的整数输入。
fn parse_number(raw: &str, label: &str, min: u64, max: u64) -> Result<u64, String> {
    let value = raw.trim().parse::<u64>().map_err(|_| {
        tf(
            t("{label} 必须是数字", "{label} must be a number"),
            &[("label", label)],
        )
    })?;
    if !(min..=max).contains(&value) {
        return Err(tf(
            t(
                "{label} 应在 {min}~{max} 之间",
                "{label} must be between {min} and {max}",
            ),
            &[
                ("label", label),
                ("min", &min.to_string()),
                ("max", &max.to_string()),
            ],
        ));
    }
    Ok(value)
}
