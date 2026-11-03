//! 应用状态与消息定义。
//!
//! 界面仍然是单向数据流：控件只发出 [`Msg`]，[`crate::ui::update`] 修改
//! [`App`]，然后 GPUI 重新调用 `Render` 画一帧。
//!
//! 需要特别说明的是**输入框**：GPUI 的文本输入是有状态的实体
//! （[`InputState`]），值活在实体里而不是活在每次重建的视图里。所以
//! [`App`] 额外持有这些实体（[`Inputs`] / [`Selects`]），并在两处保持同步：
//!
//! - 用户打字 → 实体发 `TextChanged` → 订阅回调把它转成 [`Msg`] 写回配置；
//! - 程序改值（切配置、填默认值、换语言）→ [`App::sync_inputs`] 把配置写回实体。
//!
//! 这是「有状态控件」与「单一数据源」之间的桥：数据源仍然是 [`App`]，
//! 实体只是它的一份可编辑副本。

use std::sync::{Arc, Mutex};

use gpui::{App as GpuiApp, AppContext, Context, Entity, ScrollHandle, Subscription, Window, px};
use gpuikit::elements::dialog::DialogState;
use gpuikit::elements::select::{SelectState, select};
use gpuikit::input::InputState;
use gpuikit::traits::control_sized::ControlSized;

use crate::core::engine::ProbeState;
use crate::core::i18n::{LangSetting, t, tf};
use crate::core::{
    Config, DnsMode, Engine, Profile, Shared, Status, ThemeSetting, TrafficSample, Update,
    Verbosity,
};
use crate::ui::widgets::ROW_CONTROL;

/// 日志区最多显示多少行（完整日志仍在内存里，最多 2000 行）。
pub const LOG_VIEW_LINES: usize = 200;

/// 左侧导航里的页面。
///
/// 分页是为了让每屏只放一组相关内容：仪表盘看状态，代理配置管多套配置，
/// 网络与绕过调参数，程序与日志管二进制、日志级别和输出。
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

    /// 这一页的滚动位置存在哪个槽里。
    ///
    /// 每页一个槽：共用一个的话，切页时会带着上一页的偏移量进来
    /// （「程序与日志」一打开就是半截）。
    pub fn slot(self) -> usize {
        Page::ALL
            .iter()
            .position(|candidate| *candidate == self)
            .unwrap_or(0)
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
    /// 修改「关闭窗口时询问」这个开关。
    SetConfirmCloseToTray(bool),
    /// 直接最小化到托盘（不弹窗）。
    MinimizeToTray,
    /// 切换界面语言（跟随系统 / 中文 / English）。
    SetLanguage(LangSetting),
    /// 切换界面主题（跟随系统 / 浅色 / 深色）。
    SetTheme(ThemeSetting),
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

/// 有状态文本输入框的逻辑字段。
///
/// 每个字段一个 [`InputState`] 实体，[`Field`] 是「这个实体对应配置里的哪一格」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    ProxyUrl,
    DnsAddr,
    VirtualPool,
    Bypass,
    Udpgw,
    TunName,
    Mtu,
    TcpTimeout,
    UdpTimeout,
    MaxSessions,
    UdpgwConnections,
}

impl Field {
    /// 建实体时的顺序（也决定订阅的建立顺序）。
    pub const ALL: [Field; 12] = [
        Field::Name,
        Field::ProxyUrl,
        Field::DnsAddr,
        Field::VirtualPool,
        Field::Bypass,
        Field::Udpgw,
        Field::TunName,
        Field::Mtu,
        Field::TcpTimeout,
        Field::UdpTimeout,
        Field::MaxSessions,
        Field::UdpgwConnections,
    ];

    /// 是不是多行（只有绕过列表是）。
    pub fn multiline(self) -> bool {
        matches!(self, Field::Bypass)
    }

    /// 占位文字。
    pub fn placeholder(self) -> &'static str {
        match self {
            Field::Name => t("配置名称", "Profile name"),
            Field::ProxyUrl => "socks5://127.0.0.1:7890",
            Field::DnsAddr => "8.8.8.8",
            Field::VirtualPool => "198.18.0.0/15",
            Field::Bypass => t("每行一个 IP 或 CIDR", "One IP or CIDR per line"),
            Field::Udpgw => t("留空则不转发 UDP", "Leave empty to drop UDP"),
            Field::TunName => t("自动生成", "Auto-generated"),
            Field::Mtu => "",
            Field::TcpTimeout => "",
            Field::UdpTimeout => "",
            Field::MaxSessions => t("默认", "Default"),
            Field::UdpgwConnections => "",
        }
    }

    /// 把输入框里的文字变成一条消息。
    pub fn msg(self, value: String) -> Msg {
        match self {
            Field::Name => Msg::ProfileRenamed(value),
            Field::ProxyUrl => Msg::ProxyUrl(value),
            Field::DnsAddr => Msg::DnsAddr(value),
            Field::VirtualPool => Msg::VirtualPool(value),
            Field::Bypass => Msg::BypassChanged(value),
            Field::Udpgw => Msg::UdpgwChanged(value),
            Field::TunName => Msg::TunName(value),
            Field::Mtu => Msg::MtuChanged(value),
            Field::TcpTimeout => Msg::TcpTimeout(value),
            Field::UdpTimeout => Msg::UdpTimeout(value),
            Field::MaxSessions => Msg::MaxSessions(value),
            Field::UdpgwConnections => Msg::UdpgwConnections(value),
        }
    }
}

/// 全部文本输入框实体。
pub struct Inputs {
    entries: Vec<(Field, Entity<InputState>)>,
}

impl Inputs {
    /// 按给定初值建出所有输入框。
    fn new(values: &[(Field, String)], cx: &mut Context<App>) -> Self {
        let entries = values
            .iter()
            .map(|(field, value)| {
                let field = *field;
                let value = value.clone();
                let entity = cx.new(|cx| {
                    let mut state = if field.multiline() {
                        InputState::new_multiline(cx)
                    } else {
                        InputState::new_singleline(cx)
                    };
                    state.set_placeholder(field.placeholder(), cx);
                    state.set_content_silent(value, cx);
                    state
                });
                (field, entity)
            })
            .collect();
        Self { entries }
    }

    /// 取某个字段的实体。
    pub fn get(&self, field: Field) -> Entity<InputState> {
        self.entries
            .iter()
            .find(|(candidate, _)| *candidate == field)
            .map(|(_, entity)| entity.clone())
            .expect("每个字段都有输入框")
    }

    /// 逐个遍历（建订阅、回写内容时用）。
    pub fn iter(&self) -> impl Iterator<Item = (Field, Entity<InputState>)> + '_ {
        self.entries
            .iter()
            .map(|(field, entity)| (*field, entity.clone()))
    }
}

/// 全部下拉框实体。
///
/// 选项值统一用「下标」：`usize` 可比较、可存，而且不需要给每个枚举实现
/// `PartialEq + Clone` 之外的东西。真值从下标反查。
pub struct Selects {
    pub profile: Entity<SelectState<usize>>,
    pub dns: Entity<SelectState<usize>>,
    pub verbosity: Entity<SelectState<usize>>,
    pub language: Entity<SelectState<usize>>,
    pub theme: Entity<SelectState<usize>>,
}

impl Selects {
    fn new(config: &Config, cx: &mut Context<App>) -> Self {
        Self {
            profile: cx.new(|_| SelectState::new(profile_select(config))),
            dns: cx.new(|_| SelectState::new(dns_select(config.active_profile()))),
            verbosity: cx.new(|_| {
                SelectState::new(verbosity_select(config.active_profile()))
            }),
            language: cx.new(|_| SelectState::new(language_select(config))),
            theme: cx.new(|_| SelectState::new(theme_select(config))),
        }
    }
}

/// 「当前配置」下拉框。
fn profile_select(config: &Config) -> gpuikit::elements::select::Select<usize> {
    let options: Vec<(usize, String)> = config
        .profiles
        .iter()
        .enumerate()
        .map(|(index, profile)| (index, profile.name.trim().to_string()))
        .collect();
    select("profile-select", t("配置文件", "Profile"), options)
        .large()
        .selected(config.active)
        .full_width(true)
        // 弹层默认按内容排版，会比 220dp 的触发框窄一截，看着不像同一个控件。
        .popup_width(px(ROW_CONTROL))
        .on_change(|index, _window, cx| {
            with_app(cx, |app, cx| app.handle(Msg::ProfileSelected(index), cx));
        })
}

/// DNS 模式下拉框。
fn dns_select(profile: &Profile) -> gpuikit::elements::select::Select<usize> {
    let options: Vec<(usize, String)> = DnsMode::ALL
        .iter()
        .enumerate()
        .map(|(index, mode)| (index, mode.label().to_string()))
        .collect();
    let current = DnsMode::ALL
        .iter()
        .position(|mode| *mode == profile.dns)
        .unwrap_or(0);
    select("dns-select", t("DNS 模式", "DNS mode"), options)
        .large()
        .selected(current)
        .full_width(true)
        // 弹层默认按内容排版，会比 220dp 的触发框窄一截，看着不像同一个控件。
        .popup_width(px(ROW_CONTROL))
        .on_change(|index, _window, cx| {
            let mode = DnsMode::ALL[index.min(DnsMode::ALL.len() - 1)];
            with_app(cx, |app, cx| app.handle(Msg::SetDns(mode), cx));
        })
}

/// 日志级别下拉框。
fn verbosity_select(profile: &Profile) -> gpuikit::elements::select::Select<usize> {
    let options: Vec<(usize, String)> = Verbosity::ALL
        .iter()
        .enumerate()
        .map(|(index, level)| (index, level.label().to_string()))
        .collect();
    let current = Verbosity::ALL
        .iter()
        .position(|level| *level == profile.verbosity)
        .unwrap_or(0);
    select("verbosity-select", t("日志级别", "Log level"), options)
        .large()
        .selected(current)
        .full_width(true)
        // 弹层默认按内容排版，会比 220dp 的触发框窄一截，看着不像同一个控件。
        .popup_width(px(ROW_CONTROL))
        .on_change(|index, _window, cx| {
            let level = Verbosity::ALL[index.min(Verbosity::ALL.len() - 1)];
            with_app(cx, |app, cx| app.handle(Msg::SetVerbosity(level), cx));
        })
}

/// 界面语言下拉框。
fn language_select(config: &Config) -> gpuikit::elements::select::Select<usize> {
    let options: Vec<(usize, String)> = LangSetting::ALL
        .iter()
        .enumerate()
        .map(|(index, setting)| (index, setting.label().to_string()))
        .collect();
    let current = LangSetting::ALL
        .iter()
        .position(|setting| *setting == config.language)
        .unwrap_or(0);
    select("language-select", t("界面语言", "Language"), options)
        .large()
        .selected(current)
        .full_width(true)
        // 弹层默认按内容排版，会比 220dp 的触发框窄一截，看着不像同一个控件。
        .popup_width(px(ROW_CONTROL))
        .on_change(|index, _window, cx| {
            let setting = LangSetting::ALL[index.min(LangSetting::ALL.len() - 1)];
            with_app(cx, |app, cx| app.handle(Msg::SetLanguage(setting), cx));
        })
}

/// 界面主题下拉框。
fn theme_select(config: &Config) -> gpuikit::elements::select::Select<usize> {
    let options: Vec<(usize, String)> = ThemeSetting::ALL
        .iter()
        .enumerate()
        .map(|(index, setting)| (index, setting.label().to_string()))
        .collect();
    let current = ThemeSetting::ALL
        .iter()
        .position(|setting| *setting == config.theme)
        .unwrap_or(0);
    select("theme-select", t("界面主题", "Theme"), options)
        .large()
        .selected(current)
        .full_width(true)
        // 弹层默认按内容排版，会比 220dp 的触发框窄一截，看着不像同一个控件。
        .popup_width(px(ROW_CONTROL))
        .on_change(|index, _window, cx| {
            let setting = ThemeSetting::ALL[index.min(ThemeSetting::ALL.len() - 1)];
            with_app(cx, |app, cx| app.handle(Msg::SetTheme(setting), cx));
        })
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
    /// 累计写入过多少行日志（不受 [`Snapshot::logs`] 的条数上限影响）。
    pub log_total: u64,
    /// 最近一分钟的流量采样（旧→新），空表示还没开始采。
    pub traffic: Vec<TrafficSample>,
    /// 最新一次采样的瞬时速率。
    pub rate: TrafficSample,
    /// 上游代理检测状态（`None` = 还没检测过）。
    pub probe: Option<ProbeState>,
    /// 启动被拦下的原因（上游代理连不通等），由后台线程写入。
    pub blocked: Option<String>,
}

/// 应用根状态，同时也是 GPUI 的根视图实体。
pub struct App {
    /// 当前显示的页面（左侧导航选中项）。
    pub page: Page,
    /// 侧边栏是否收起成「图标栏」。
    ///
    /// 收起后侧边栏只剩一列图标（宽度 [`super::sidebar::RAIL_WIDTH`]），
    /// 把宽度让给内容区。状态只活在本次运行里：这是个临时视角切换，
    /// 不是需要落盘的偏好。
    pub sidebar_collapsed: bool,
    /// 「代理配置」页是否处在「编辑某套配置」的详情态。
    ///
    /// 列表才是默认态：大多数时候用户是来「换一套」而不是「改参数」。
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

    /// 文本输入框实体。
    pub inputs: Inputs,
    /// 下拉框实体。
    pub selects: Selects,
    /// 「最小化到托盘？」对话框。按需创建：正文随服务状态变，建一次就只能
    /// 显示一种说法。
    pub close_dialog: Option<Entity<DialogState>>,
    /// 日志窗格的滚动位置（跟随最新一行）。
    pub log_scroll: ScrollHandle,
    /// 日志窗格是否贴住最新一行。
    ///
    /// 用户往回翻就置 false，翻回底部再置 true。没有它的话「每来一行就滚到
    /// 底」会让往回翻变得没意义——整机流量下每秒几百行，翻上去立刻被拽回来。
    pub log_follow: bool,
    /// 每页各自的滚动位置。
    pub page_scroll: [ScrollHandle; 4],
    /// 上一次渲染时的日志累计行数：变了才把日志滚到底，否则每帧都在动。
    last_log_len: u64,
    /// 输入框 / 对话框的订阅，随实体一起活着。
    _subscriptions: Vec<Subscription>,
}

impl App {
    pub fn new(
        config: Config,
        engine: Engine,
        shared: Arc<Mutex<Shared>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let draft = Draft::from_profile(config.active_profile());
        let inputs = Inputs::new(&field_values(&config, &draft), cx);
        let selects = Selects::new(&config, cx);

        // 用户打字 → 一条 Msg。
        let mut subscriptions = Vec::new();
        for (field, entity) in inputs.iter() {
            subscriptions.push(cx.subscribe(
                &entity,
                move |app: &mut App, _input, event: &gpuikit::input::InputStateEvent, cx| {
                    if !matches!(event, gpuikit::input::InputStateEvent::TextChanged) {
                        return;
                    }
                    let text = app.inputs.get(field).read(cx).content().to_string();
                    app.handle(field.msg(text), cx);
                },
            ));
        }

        // 引擎在后台线程改状态时只调一个回调：把「该重画了」送进界面线程。
        //
        // 回调是 `Send + Sync` 的普通闭包，而 `cx.notify()` 只能在界面线程上调，
        // 所以中间垫一个 channel：回调只管往里丢信号，界面线程上跑的任务收到
        // 就通知重绘。gpui 的 foreground executor 保证这段跑在界面线程上。
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        engine.set_rebuild_hook(Arc::new(move || {
            let _ = tx.send(());
        }));
        cx.spawn(async move |this, cx| {
            while rx.recv().await.is_some() {
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        // 启动时静默检查一次更新（只查询，不下载）。
        //
        // 延后 0.8 秒再查：窗口刚建好时先让首帧画出来，用户看到的是一个
        // 有内容的界面，而不是等版本号查完才出现的空壳。
        if config.auto_check_update {
            let engine = engine.clone();
            let repo = config.release_repo.clone();
            cx.spawn(async move |_this, cx| {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(800))
                    .await;
                engine.check_update(repo);
            })
            .detach();
        }

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
            inputs,
            selects,
            close_dialog: None,
            log_scroll: ScrollHandle::default(),
            log_follow: true,
            page_scroll: std::array::from_fn(|_| ScrollHandle::default()),
            last_log_len: 0,
            _subscriptions: subscriptions,
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

    /// 切换配置：草稿与输入框同步刷新成新配置里的数值。
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
            log_total: shared.log_total,
            traffic: shared.traffic.iter().copied().collect(),
            rate: shared.rate,
            probe: shared.probe.clone(),
            blocked: shared.start_blocked.clone(),
        }
    }

    /// 把配置里的值写回输入框实体。
    ///
    /// 只在「程序改的值」之后调用（切配置、填默认值、增删配置、换语言）：
    /// 用户打字时不能回写，否则每敲一个字都会把光标打回开头。
    pub fn sync_inputs(&mut self, cx: &mut Context<Self>) {
        for (field, value) in field_values(&self.config, &self.draft) {
            let entity = self.inputs.get(field);
            let current = entity.read(cx).content().to_string();
            if current != value {
                entity.update(cx, |state, cx| state.set_content_silent(value, cx));
            }
        }
    }

    /// 语言变了：占位文字跟着换，输入框重建一次（值不变）。
    pub fn rebuild_inputs(&mut self, cx: &mut Context<Self>) {
        let values = field_values(&self.config, &self.draft);
        self.inputs = Inputs::new(&values, cx);
        let mut subscriptions = Vec::new();
        for (field, entity) in self.inputs.iter() {
            subscriptions.push(cx.subscribe(
                &entity,
                move |app: &mut App, _input, event: &gpuikit::input::InputStateEvent, cx| {
                    if !matches!(event, gpuikit::input::InputStateEvent::TextChanged) {
                        return;
                    }
                    let text = app.inputs.get(field).read(cx).content().to_string();
                    app.handle(field.msg(text), cx);
                },
            ));
        }
        self._subscriptions = subscriptions;
    }

    /// 配置列表或语言变了：下拉框的选项是建实体时定死的，只能整组重建。
    pub fn rebuild_selects(&mut self, cx: &mut Context<Self>) {
        self.selects = Selects::new(&self.config, cx);
    }

    /// 让下拉框显示「现在真正的值」。
    pub fn sync_selects(&mut self, cx: &mut Context<Self>) {
        let profile = self.config.active;
        let dns = DnsMode::ALL
            .iter()
            .position(|mode| *mode == self.config.active_profile().dns)
            .unwrap_or(0);
        let verbosity = Verbosity::ALL
            .iter()
            .position(|level| *level == self.config.active_profile().verbosity)
            .unwrap_or(0);
        let language = LangSetting::ALL
            .iter()
            .position(|setting| *setting == self.config.language)
            .unwrap_or(0);
        let theme = ThemeSetting::ALL
            .iter()
            .position(|setting| *setting == self.config.theme)
            .unwrap_or(0);

        self.selects
            .profile
            .update(cx, |state, cx| state.set_selected(Some(profile), cx));
        self.selects
            .dns
            .update(cx, |state, cx| state.set_selected(Some(dns), cx));
        self.selects
            .verbosity
            .update(cx, |state, cx| state.set_selected(Some(verbosity), cx));
        self.selects
            .language
            .update(cx, |state, cx| state.set_selected(Some(language), cx));
        self.selects
            .theme
            .update(cx, |state, cx| state.set_selected(Some(theme), cx));
    }

    /// 日志行数变了就滚到底（贴住最新一行）。
    ///
    /// `total` 是累计条数，不是当前窗口里的条数——后者到上限就不动了。
    /// 用户往回翻过（[`App::log_follow`] 为 false）就不动，否则每来一行都会
    /// 把人拽回底部。
    pub fn follow_logs(&mut self, total: u64) {
        if self.last_log_len != total {
            self.last_log_len = total;
            self.sync_log_follow();
        }
    }

    /// 让「是否贴底」与滚动位置对齐，然后决定要不要滚到底。
    ///
    /// 由渲染前和滚轮处理共用：滚轮动过之后来新行，判断依据必须是用户最后
    /// 停下的位置。
    pub fn sync_log_follow(&mut self) {
        let offset = self.log_scroll.offset().y;
        let max = self.log_scroll.max_offset().y;
        // 偏移是 `[-max, 0]`：底部是 -max。`max + offset` 就是「离底部还有
        // 多远」，差一行以内算贴底——滚轮一行一行走，正好停在底线时会有
        // 不到 1px 的浮点残差。
        self.log_follow = max + offset <= px(16.0);
        if self.log_follow {
            self.log_scroll.scroll_to_bottom();
        }
    }
}

/// 配置里每个字段现在的文本值。
fn field_values(config: &Config, draft: &Draft) -> Vec<(Field, String)> {
    let profile = config.active_profile();
    Field::ALL
        .into_iter()
        .map(|field| {
            let value = match field {
                Field::Name => profile.name.clone(),
                Field::ProxyUrl => profile.proxy_url.clone(),
                Field::DnsAddr => profile.dns_addr.clone(),
                Field::VirtualPool => profile.virtual_dns_pool.clone(),
                Field::Bypass => profile.bypass.clone(),
                Field::Udpgw => profile.udpgw_server.clone(),
                Field::TunName => profile.tun_name.clone(),
                Field::Mtu => draft.mtu.clone(),
                Field::TcpTimeout => draft.tcp_timeout.clone(),
                Field::UdpTimeout => draft.udp_timeout.clone(),
                Field::MaxSessions => draft.max_sessions.clone(),
                Field::UdpgwConnections => draft.udpgw_connections.clone(),
            };
            (field, value)
        })
        .collect()
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

/// 界面实体在 `App` 全局里的句柄。
///
/// 组件的回调（下拉框的 `on_change`、对话框的 `on_confirm`）只能拿到
/// `&mut App`，拿不到 `&mut Context<App>`。把「界面实体是谁」存成一个全局，
/// 这些回调就能顺着它找回来，不必给每个组件再包一层中转实体。
pub struct UiHandle(pub gpui::WeakEntity<App>);

impl gpui::Global for UiHandle {}

/// 从任意 `&mut App` 拿到界面实体。
pub fn ui_entity(cx: &GpuiApp) -> Option<Entity<App>> {
    cx.try_global::<UiHandle>().and_then(|handle| handle.0.upgrade())
}

/// 在界面实体上执行一段逻辑（组件回调的常用写法）。
///
/// 末尾统一 `notify()`：`Entity::update` 只负责借出状态，不会替我们标脏，
/// 少了这一步界面会停在旧画面上（数据变了，但没人重画）。
pub fn with_app(cx: &mut GpuiApp, f: impl FnOnce(&mut App, &mut Context<App>)) {
    if let Some(app) = ui_entity(cx) {
        app.update(cx, |app, cx| {
            f(app, cx);
            cx.notify();
        });
    }
}
