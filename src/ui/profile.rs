//! 「代理配置」页：多套配置的切换与编辑。
//!
//! 两种状态共用一个页面：**列表**（点一行就切过去，右侧能改能删）和**编辑**
//! （一套配置的全部参数）。没有做成「列表 → 另一个页面」的跳转，是因为这一页
//! 的常驻问题是「我现在用的是哪套、它配的是什么」，切换和编辑挤在同一屏里
//! 更好比；但窗口最小 780dp 放不下左右两栏，所以用同一位置的两种状态。
//!
//! 表单是**草稿态**：改字段只改内存里的 [`ProfileDraft`]，点「保存」才落盘。
//! 这样改一半反悔可以直接返回，不会把配置写成半成品。

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::core::config::{DnsMode, Profile, Verbosity};
use crate::core::engine::ProbeState;
use crate::core::i18n::t;
use crate::core::paths;
use crate::core::probe;
use crate::core::Config;
use crate::tr_format;

use super::live::Live;
use super::state;
use super::widgets::{
    action_row, card_head, card_shell, chip, chips, danger_color, field, ghost_button,
    ghost_button_maybe, ok_color, primary_button, text_area, text_field,
};

/// 这一页的界面状态。
///
/// `rev` 是「配置被改过几次」的计数：配置本身存在 `state` 的锁里，没有信号可
/// 订阅，所以每次写盘之后把这个数加一，界面据此重新读一份配置并重绘。
#[derive(Clone, Copy)]
pub struct ProfileUi {
    rev: Signal<u64>,
    /// 正在编辑哪一套（`None` = 列表态）。
    editing: Signal<Option<usize>>,
    /// 编辑中的草稿。
    draft: Signal<Option<ProfileDraft>>,
    /// 上一次保存失败的结论。
    error: Signal<Option<String>>,
}

impl ProfileUi {
    pub fn new() -> Self {
        Self {
            rev: signal(0),
            editing: signal(None),
            draft: signal(None),
            error: signal(None),
        }
    }

    /// 当前配置的一份副本。
    fn config(&self) -> Config {
        // 读一下 rev 让调用方成为它的依赖：rev 变了这一页才重绘。
        let _ = self.rev.get();
        state::config().unwrap_or_default()
    }

    /// 配置刚被改过：让这一页重新读一遍。
    fn bump(&self) {
        self.rev.set(self.rev.get() + 1);
    }
}

/// 表单里的值。
///
/// 数字字段先存字符串：用户敲到一半的「15」不该被当成 15 写进配置，等他按
/// 保存时再解析，解析失败就当场说哪个字段不对。
#[derive(Clone, PartialEq)]
struct ProfileDraft {
    name: String,
    proxy_url: String,
    dns: DnsMode,
    dns_addr: String,
    virtual_dns_pool: String,
    tun_name: String,
    mtu: String,
    tcp_timeout: String,
    udp_timeout: String,
    max_sessions: String,
    bypass: String,
    verbosity: Verbosity,
}

impl ProfileDraft {
    fn from(profile: &Profile) -> Self {
        Self {
            name: profile.name.clone(),
            proxy_url: profile.proxy_url.clone(),
            dns: profile.dns,
            dns_addr: profile.dns_addr.clone(),
            virtual_dns_pool: profile.virtual_dns_pool.clone(),
            tun_name: profile.tun_name.clone(),
            mtu: profile.mtu.to_string(),
            tcp_timeout: profile.tcp_timeout.to_string(),
            udp_timeout: profile.udp_timeout.to_string(),
            max_sessions: profile.max_sessions.to_string(),
            bypass: profile.bypass.clone(),
            verbosity: profile.verbosity,
        }
    }

    /// 合回一套完整配置：数字字段在这里解析，失败给出「哪个字段、要什么」。
    fn to_profile(&self, base: &Profile) -> Result<Profile, String> {
        let mut profile = base.clone();
        profile.name = self.name.trim().to_string();
        profile.proxy_url = self.proxy_url.trim().to_string();
        profile.dns = self.dns;
        profile.dns_addr = self.dns_addr.trim().to_string();
        profile.virtual_dns_pool = self.virtual_dns_pool.trim().to_string();
        profile.tun_name = self.tun_name.trim().to_string();
        profile.bypass = self.bypass.clone();
        profile.verbosity = self.verbosity;
        profile.mtu = number(&self.mtu, t("MTU", "MTU"), 576, 9000)? as u16;
        profile.tcp_timeout = number(&self.tcp_timeout, t("TCP 超时", "TCP timeout"), 1, 86_400)?;
        profile.udp_timeout = number(&self.udp_timeout, t("UDP 超时", "UDP timeout"), 1, 86_400)?;
        profile.max_sessions = number(
            &self.max_sessions,
            t("最大会话数", "Max sessions"),
            1,
            1_000_000,
        )? as usize;
        profile.validate()?;
        Ok(profile)
    }
}

/// 解析一个整数并检查范围，错误里带上字段名。
fn number(raw: &str, label: &'static str, low: u64, high: u64) -> Result<u64, String> {
    state::parse_number(raw, label, low, high)
}

/// 页面入口：列表态或编辑态。
pub fn page(ui: ProfileUi, live: Signal<Live>) -> impl IntoElement {
    scroll().flex_1().child(dynamic(move || {
        match ui.editing.get() {
            Some(index) => editor(ui, index, live).into_element(),
            None => list(ui, live).into_element(),
        }
    }))
}

// —— 列表态 ——

fn list(ui: ProfileUi, live: Signal<Live>) -> View {
    let config = ui.config();
    let count = config.profiles.len();
    card_shell()
        .child(card_head(
            t("配置", "Profiles"),
            Some(tr_format!(
                "共 {} 套，点一行就切过去；启动隧道用的是标着「当前」的那套。",
                "{} profiles; click a row to switch. The tunnel starts with the one marked current.",
                count
            )),
            view(),
        ))
        .children((0..count).map(|index| row(ui, live, &config, index)))
        .child(action_row(vec![
            ghost_button(t("复制当前", "Duplicate"), move || {
                if let Some(index) = state::duplicate_profile() {
                    ui.bump();
                    begin_edit(ui, index);
                }
            }),
            primary_button(t("新建", "New"), move || {
                if let Some(index) = state::add_profile() {
                    ui.bump();
                    begin_edit(ui, index);
                }
            }),
        ]))
        .child(
            view().child(
                text(tr_format!(
                    "配置存在 {}",
                    "Saved to {}",
                    paths::config_path().display()
                ))
                .text_xs()
                .text_color(move || theme().muted_foreground),
            ),
        )
}

/// 列表里的一套配置。
///
/// 点击只挂在**左边那块内容**上，不挂整行：右侧的「编辑 / 删除」是行内的
/// 独立按钮，整行再接一个点击的话，两个都响（点删除会顺带切过去）。
fn row(ui: ProfileUi, live: Signal<Live>, config: &Config, index: usize) -> View {
    let profile = &config.profiles[index];
    let active = config.active == index;
    let only_one = config.profiles.len() <= 1;
    let url = profile.proxy_url.clone();
    let url_note = url.clone();
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .p_3()
        .rounded_lg()
        .child(
            // 竖条只在当前那套上出现：颜色之外再给一个形状上的区别。
            view()
                .w(px(4.0))
                .h(px(28.0))
                .rounded_full()
                .bg(move || {
                    if active {
                        theme().primary
                    } else {
                        AdaptiveColor::TRANSPARENT
                    }
                }),
        )
        .child({
            let mut body = view()
                .flex_1()
                .flex_col()
                .gap_1()
                .child(
                    text(profile.display_name())
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(move || {
                            if active {
                                theme().primary
                            } else {
                                theme().card_foreground
                            }
                        }),
                )
                .child(
                    text(profile.summary())
                        .text_xs()
                        .text_color(move || theme().muted_foreground),
                )
                .child(dynamic(move || probe_note(live, url_note.clone())));
            if !active {
                body = body.cursor_pointer().on_press(move || {
                    if state::select_profile(index) {
                        ui.bump();
                    }
                });
            }
            body
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(dynamic(move || probe_button(live, url.clone())))
                .child(ghost_button(t("编辑", "Edit"), move || begin_edit(ui, index)))
                .child(ghost_button_maybe(
                    t("删除", "Delete"),
                    !only_one,
                    move || {
                        if state::remove_profile(index) {
                            ui.bump();
                        }
                    },
                )),
        )
}

/// 一行里那个「检测 / 结论」位。
///
/// 结论按**地址**存（内核的 `Shared::probes`，键是归一化后的地址），所以改过
/// 地址之后旧结论自己就查不到了——不会出现「测过」的标签挂在没测过的新地址上。
///
/// 结论文字和按钮分两处画：结论可能很长（`连不上 192.168.10.121:1010：端口没有在监听`），
/// 和按钮挤在同一行会把左边那列摘要压成两行，整行高度跟着跳。
fn probe_note(live: Signal<Live>, url: String) -> AnyElement {
    match probe_of(live, &url) {
        Some(ProbeState::Done(report)) => text(report.detail.clone())
            .text_xs()
            .text_color(if report.ok {
                ok_color()
            } else {
                danger_color()
            })
            .into_element(),
        _ => view().into_element(),
    }
}

/// 「检测 / 检测中…」按钮。
fn probe_button(live: Signal<Live>, url: String) -> AnyElement {
    let running = matches!(probe_of(live, &url), Some(ProbeState::Running));
    let label = if running {
        t("检测中…", "Checking…")
    } else {
        t("检测", "Check")
    };
    ghost_button_maybe(label, !running, move || state::check_proxy(&url)).into_element()
}

/// 某个地址当前的检测状态。
fn probe_of(live: Signal<Live>, url: &str) -> Option<ProbeState> {
    live.with(|snapshot| snapshot.probes.get(&probe::key(url)).cloned())
}

// —— 编辑态 ——

/// 进入编辑：把这一套读成草稿。
fn begin_edit(ui: ProfileUi, index: usize) {
    let config = ui.config();
    let Some(profile) = config.profiles.get(index) else {
        return;
    };
    ui.draft.set(Some(ProfileDraft::from(profile)));
    ui.editing.set(Some(index));
    ui.error.set(None);
}

fn end_edit(ui: ProfileUi) {
    ui.editing.set(None);
    ui.draft.set(None);
    ui.error.set(None);
}

/// 改草稿里的一个字段。
fn edit(ui: ProfileUi, change: impl FnOnce(&mut ProfileDraft)) {
    let Some(mut draft) = ui.draft.get() else {
        return;
    };
    change(&mut draft);
    ui.draft.set(Some(draft));
    // 改了东西就把上一次的错误收起来：那个结论说的是改之前的值。
    ui.error.set(None);
}

fn editor(ui: ProfileUi, index: usize, live: Signal<Live>) -> View {
    let Some(draft) = ui.draft.get() else {
        return card_shell();
    };
    let config = ui.config();
    let name = config
        .profiles
        .get(index)
        .map(|profile| profile.display_name())
        .unwrap_or_default();
    // DNS 模式决定哪两个字段参与（见下面两行的说明）。
    let virtual_dns = draft.dns == DnsMode::Virtual;

    card_shell()
        .child(card_head(
            t("编辑这套配置", "Edit this profile"),
            Some(tr_format!("正在改「{}」", "Editing “{}”", name)),
            view(),
        ))
        // 「取消 / 保存」放在卡片头**下面一行**，而不是卡片头的动作区：那一行
        // 在页面滚动之后的命中区还停在没滚动时的位置，点了没反应。放在内容流
        // 里就能一直点得动，而且仍在顶部——长表单改一两个字段就能直接保存。
        .child(action_row(vec![
            ghost_button(t("取消", "Cancel"), move || end_edit(ui)),
            primary_button(t("保存", "Save"), move || save(ui, index)),
        ]))
        // 校验失败的结论也放顶部：底部在长表单里要滚到最后才看得见，
        // 用户点完保存只会觉得「没反应」。
        .child(dynamic(move || match ui.error.get() {
            Some(error) => text(error)
                .text_sm()
                .text_color(move || danger_color())
                .into_element(),
            None => view().into_element(),
        }))
        // —— 名字与代理 ——
        .child(field(
            t("配置名称", "Profile name"),
            Some(t("配置列表里显示的名字", "Shown in the profile list").to_string()),
            text_field(
                move || ui.draft.get().map(|d| d.name).unwrap_or_default(),
                t("例如：公司代理", "e.g. office proxy"),
                true,
                move |text| edit(ui, |draft| draft.name = text),
            ),
        ))
        .child(field(
            t("上游代理地址", "Upstream proxy"),
            Some(
                t(
                    "socks5:// 或 http:// 开头",
                    "Starts with socks5:// or http://",
                )
                .to_string(),
            ),
            text_field(
                move || ui.draft.get().map(|d| d.proxy_url).unwrap_or_default(),
                "socks5://127.0.0.1:7890",
                true,
                move |text| edit(ui, |draft| draft.proxy_url = text),
            ),
        ))
        // 填完地址就地测一次：这里才是「刚输完」的位置，回列表再测要多跳一步。
        // 测的是**草稿里**的地址，改了地址结论自己就消失（结论按地址存）。
        .child(action_row(vec![ghost_button(
            t("检测这个地址", "Test this address"),
            move || {
                if let Some(draft) = ui.draft.get() {
                    state::check_proxy(draft.proxy_url.trim());
                }
            },
        )]))
        .child(dynamic(move || {
            let url = ui
                .draft
                .get()
                .map(|draft| draft.proxy_url)
                .unwrap_or_default();
            probe_note(live, url)
        }))
        // —— DNS ——
        .child(field(
            t("DNS 模式", "DNS mode"),
            Some(draft.dns.hint().to_string()),
            chips(DnsMode::ALL.iter().map(|mode| {
                let mode = *mode;
                let chosen = draft.dns == mode;
                chip(mode.label(), chosen, move || {
                    edit(ui, |draft| draft.dns = mode)
                })
            })),
        ))
        // 「远程 DNS」和「虚拟池」各自只服务一半的模式：前者在「虚拟 IP」下
        // 完全不参与（本地应答，查询不出网），后者只有「虚拟 IP」用得到。
        // 两个框以前一直可编辑，填了不生效也没人说——现在跟着模式灰掉，并在
        // 说明里写清为什么不生效。
        .child(field(
            t("远程 DNS", "Remote DNS"),
            Some(if virtual_dns {
                t(
                    "「虚拟 IP」在本地应答，这一项不参与",
                    "Virtual IP answers locally, so this is not used",
                )
                .to_string()
            } else {
                t("查询经隧道发往这台服务器", "Queries go to this server through the tunnel").to_string()
            }),
            text_field(
                move || ui.draft.get().map(|d| d.dns_addr).unwrap_or_default(),
                "8.8.8.8",
                !virtual_dns,
                move |text| edit(ui, |draft| draft.dns_addr = text),
            ),
        ))
        .child(field(
            t("虚拟地址池", "Virtual pool"),
            Some(if virtual_dns {
                t(
                    "接管 DNS 时分配给域名的假地址段",
                    "Fake range handed to domains when DNS is intercepted",
                )
                .to_string()
            } else {
                t("只有「虚拟 IP」模式会用到", "Only the Virtual IP mode uses this").to_string()
            }),
            text_field(
                move || {
                    ui.draft
                        .get()
                        .map(|d| d.virtual_dns_pool)
                        .unwrap_or_default()
                },
                "198.18.0.0/15",
                virtual_dns,
                move |text| edit(ui, |draft| draft.virtual_dns_pool = text),
            ),
        ))
        // —— 隧道 ——
        .child(field(
            t("网卡名称", "Adapter name"),
            Some(t("留空则每次启动随机生成", "Empty: a random name each start").to_string()),
            text_field(
                move || ui.draft.get().map(|d| d.tun_name).unwrap_or_default(),
                "tun0",
                true,
                move |text| edit(ui, |draft| draft.tun_name = text),
            ),
        ))
        .child(field(
            t("MTU", "MTU"),
            Some(t("默认 1500", "Default 1500").to_string()),
            text_field(
                move || ui.draft.get().map(|d| d.mtu).unwrap_or_default(),
                "1500",
                true,
                move |text| edit(ui, |draft| draft.mtu = text),
            ),
        ))
        .child(field(
            t("TCP 超时", "TCP timeout"),
            Some(t("秒；默认 600", "Seconds; default 600").to_string()),
            text_field(
                move || ui.draft.get().map(|d| d.tcp_timeout).unwrap_or_default(),
                "600",
                true,
                move |text| edit(ui, |draft| draft.tcp_timeout = text),
            ),
        ))
        .child(field(
            t("UDP 超时", "UDP timeout"),
            Some(t("秒；默认 10", "Seconds; default 10").to_string()),
            text_field(
                move || ui.draft.get().map(|d| d.udp_timeout).unwrap_or_default(),
                "10",
                true,
                move |text| edit(ui, |draft| draft.udp_timeout = text),
            ),
        ))
        .child(field(
            t("最大会话数", "Max sessions"),
            Some(
                t(
                    "并发上限；顶到上限只丢新连接，隧道不会退出",
                    "Concurrent limit; new connections are dropped at the limit",
                )
                .to_string(),
            ),
            text_field(
                move || {
                    ui.draft
                        .get()
                        .map(|d| d.max_sessions)
                        .unwrap_or_default()
                },
                "5000",
                true,
                move |text| edit(ui, |draft| draft.max_sessions = text),
            ),
        ))
        .child(field(
            t("日志级别", "Log level"),
            Some(t("子进程输出的详细程度", "How much the child process logs").to_string()),
            chips(Verbosity::ALL.iter().map(|level| {
                let level = *level;
                let chosen = draft.verbosity == level;
                chip(level.label(), chosen, move || {
                    edit(ui, |draft| draft.verbosity = level)
                })
            })),
        ))
        // —— 绕过 ——
        .child(
            view()
                .flex_col()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            text(t("绕过地址", "Bypass"))
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || theme().secondary_foreground),
                        )
                        .child(
                            text(t(
                                "每行一个 IP 或 CIDR；这些地址不进隧道",
                                "One IP or CIDR per line; these skip the tunnel",
                            ))
                            .text_xs()
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(view().flex_1())
                        .child(ghost_button(t("填入局域网默认值", "Fill LAN defaults"), move || {
                            edit(ui, |draft| draft.bypass = crate::core::config::DEFAULT_BYPASS.to_string())
                        })),
                )
                .child(text_area(
                    move || ui.draft.get().map(|d| d.bypass).unwrap_or_default(),
                    t("10.0.0.0/8", "10.0.0.0/8"),
                    132.0,
                    move |text| edit(ui, |draft| draft.bypass = text),
                )),
        )
}

/// 保存：解析 → 校验 → 落盘 → 回列表。
///
/// 校验失败就留在编辑态，结论显示在卡片顶部（用户改完再点一次）。
fn save(ui: ProfileUi, index: usize) {
    let config = ui.config();
    let Some(base) = config.profiles.get(index) else {
        end_edit(ui);
        return;
    };
    let Some(draft) = ui.draft.get() else {
        end_edit(ui);
        return;
    };
    let profile = match draft.to_profile(base) {
        Ok(profile) => profile,
        Err(error) => {
            ui.error.set(Some(error));
            return;
        }
    };
    if state::save_profile(index, profile) {
        ui.bump();
        end_edit(ui);
    } else {
        ui.error
            .set(Some(t("保存失败，看看日志里的原因", "Save failed; see the log").to_string()));
    }
}
