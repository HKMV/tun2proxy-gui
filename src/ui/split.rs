//! 分流规则页：国内流量直连的开关、清单、自定义规则与直连出口。
//!
//! 这一页的设置**大多要重启隧道才生效**：分流器是隧道启动时按当时的设置起
//! 来的，改完开关只有下一次启动才用得上。所以每块都把这句话写在说明里，
//! 而不是让用户改完去猜为什么没变化。
//!
//! 运行期的数字（生效多少条规则、本次分流了多少）来自每秒一份的 [`Live`]
//! 快照；设置本身存在配置里，改完靠 [`SplitUi::rev`] 触发重绘。

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::core::config::DnsMode;
use crate::core::engine::SplitUpdate;
use crate::core::i18n::t;
use crate::core::split::service::DecisionRoute;
use crate::core::split::settings::SplitGroup;
use crate::core::split::SplitSettings;
use crate::tr_format;

use super::live::Live;
use super::state;
use super::widgets::{
    action_row, card_head, card_shell, danger_color, direct_color, field, field_wrapped,
    ghost_button_maybe, note, note_result, ok_color, primary_button, proxy_color, text_area,
    text_field, toggle, warn_color, Note,
};

/// 这一页的界面状态。
///
/// 自定义规则与直连出口是**草稿**：边打边改信号里的值，点保存才写进配置。
/// 直接写配置的话，敲到一半的 `10.0.0.0/3` 会被当成规则去校验，一路上红。
#[derive(Clone, Copy)]
pub struct SplitUi {
    /// 配置被改过几次（配置在锁里，没有信号可订阅，改完把它加一让页面重绘）。
    rev: Signal<u64>,
    custom: Signal<String>,
    custom_note: Signal<Option<Note>>,
    direct_dns: Signal<String>,
    direct_interface: Signal<String>,
    outlet_note: Signal<Option<Note>>,
}

impl SplitUi {
    pub fn new() -> Self {
        let settings = state::config().map(|config| config.split).unwrap_or_default();
        Self {
            rev: signal(0u64),
            custom: signal(settings.custom),
            custom_note: signal(None),
            direct_dns: signal(settings.direct_dns),
            direct_interface: signal(settings.direct_interface),
            outlet_note: signal(None),
        }
    }

    fn bump(self) {
        self.rev.set(self.rev.get() + 1);
    }
}

/// 当前的分流设置（订阅 `rev`，开关一改就重读）。
fn current(ui: SplitUi) -> SplitSettings {
    let _ = ui.rev.get();
    state::config().map(|config| config.split).unwrap_or_default()
}

/// 页面入口。
pub fn page(ui: SplitUi, live: Signal<Live>) -> impl IntoElement {
    scroll().flex_1().child(
        view()
            .flex_col()
            .gap_4()
            .child(switch_card(ui, live))
            .child(lists_card(ui))
            .child(custom_card(ui))
            .child(outlet_card(ui))
            .child(decisions_card(live))
            .child(update_card(live)),
    )
}

// —— 开关与状态 ——

fn switch_card(ui: SplitUi, live: Signal<Live>) -> View {
    let settings = current(ui);
    card_shell()
        .child(card_head(
            t("国内流量直连", "Route China traffic directly"),
            Some(
                t(
                    "命中国内清单的域名与 IP 用本地网络直连，其余照常走代理。改完要重启隧道才生效。",
                    "Domains and IPs on the China lists are dialed directly; the rest keep using the proxy. A restart applies changes.",
                )
                .to_string(),
            ),
            toggle(settings.enabled, move || {
                let next = !current(ui).enabled;
                if state::set_split_enabled(next) {
                    ui.bump();
                }
            }),
        ))
        .child(dynamic(move || {
            let snapshot = live.get();
            let runtime = snapshot.split.clone();
            // 三种状态分开写：关着时解释它为什么不生效；开着但隧道没跑就说
            // 「等启动」；真跑起来了才把监听地址和生效条数摆出来——用户核对
            // 「到底有没有用上」靠的就是这两个数。
            let (label, color) = if !current(ui).enabled {
                (t("已关闭", "Disabled").to_string(), theme().muted_foreground)
            } else if runtime.running {
                (t("运行中", "Running").to_string(), ok_color())
            } else {
                (
                    t("已开启，隧道启动后生效", "Enabled; applies when the tunnel starts")
                        .to_string(),
                    warn_color(),
                )
            };
            let detail = if runtime.running {
                let addr = runtime
                    .addr
                    .map(|addr| addr.to_string())
                    .unwrap_or_else(|| "127.0.0.1".to_string());
                tr_format!(
                    "监听 {addr}，生效 {} 条规则 · 本次直连 {} / 走代理 {} / 失败 {}",
                    "Listening on {addr} with {} rules · this run: {} direct / {} proxied / {} failed",
                    runtime.rules,
                    runtime.direct,
                    runtime.proxy,
                    runtime.failed
                )
            } else if current(ui).enabled {
                // 上面那句 label 已经说了「隧道启动后生效」，这里再说一遍是废话。
                String::new()
            } else {
                t(
                    "所有流量照常走上游代理，与没有这个功能时完全一致",
                    "Everything still goes through the upstream proxy, exactly as before",
                )
                .to_string()
            };
            view()
                .flex_col()
                .gap_2()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_3()
                        .child(
                            text(label)
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || color),
                        )
                        .child(
                            text(format!("· {detail}"))
                                .text_xs()
                                .text_color(move || theme().muted_foreground),
                        ),
                )
                .child(dns_warning(ui))
                .into_element()
        }))
}

/// 域名清单只在「虚拟 IP」策略下才看得见。
///
/// tun2proxy 在其它策略里交给分流器的只有 IP 字面量，域名清单无从匹配。不提示
/// 的话用户会以为「国内域名都配上了」，实际只有 IP 段在起作用。
fn dns_warning(ui: SplitUi) -> View {
    if !current(ui).enabled {
        return view();
    }
    let Some(config) = state::config() else {
        return view();
    };
    let mode = config.active_profile().dns;
    if mode == DnsMode::Virtual {
        return view();
    }
    note(
        tr_format!(
            "当前 DNS 策略是「{}」：tun2proxy 只能看到 IP，域名清单不会生效，只有 IP 段在分流。把 DNS 策略改成「虚拟 IP」才能用上域名规则。",
            "DNS mode is “{}”: tun2proxy only sees IPs, so the domain lists cannot match and only IP ranges are split. Switch DNS to “Virtual IP” to use domain rules.",
            mode.label()
        ),
        warn_color(),
    )
}

// —— 内置清单 ——

fn lists_card(ui: SplitUi) -> View {
    let settings = current(ui);
    let (local, china_ip, china_domain) = state::split_counts();
    let counts = [local, china_ip, china_domain];
    card_shell()
        .child(card_head(
            t("内置清单", "Built-in lists"),
            Some(
                t(
                    "按组开关；关掉的那一组不参与匹配，流量照常走代理。",
                    "Switch whole groups on or off; disabled groups simply do not match.",
                )
                .to_string(),
            ),
            view(),
        ))
        .children(SplitGroup::ALL.iter().enumerate().map(|(index, group)| {
            let group = *group;
            let hint = tr_format!("{}（{} 条）", "{} ({} entries)", group.hint(), counts[index]);
            field_wrapped(
                group.label(),
                Some(hint),
                toggle(settings.group_enabled(group), move || {
                    let next = !current(ui).group_enabled(group);
                    if state::set_split_group(group, next) {
                        ui.bump();
                    }
                }),
            )
        }))
}

// —— 自定义规则 ——

fn custom_card(ui: SplitUi) -> View {
    card_shell()
        .child(card_head(
            t("自定义规则", "Custom rules"),
            Some(
                t(
                    "一行一条；支持 qq.com（后缀）、full:www.qq.com（精确）、keyword:bilili（包含）、203.0.113.0/24（网段）、proxy:example.org（强制走代理）。# 后面是注释。",
                    "One rule per line: qq.com (suffix), full:www.qq.com (exact), keyword:bilili (substring), 203.0.113.0/24 (CIDR), proxy:example.org (force proxy). Everything after # is a comment.",
                )
                .to_string(),
            ),
            view(),
        ))
        .child(text_area(
            move || ui.custom.get(),
            t(
                "qq.com\nproxy:img.example.org",
                "qq.com\nproxy:img.example.org",
            ),
            148.0,
            move |text| {
                ui.custom.set(text);
                // 改了就把上一次的结论收起来：那句话说的是改之前的内容。
                ui.custom_note.set(None);
            },
        ))
        .child(action_row(vec![primary_button(
            t("保存规则", "Save rules"),
            move || {
                let verdict = state::save_split_custom(ui.custom.get());
                ui.custom_note.set(Some(match verdict {
                    Ok(()) => Note::ok(t(
                        "已保存，重启隧道后生效",
                        "Saved; applies after a restart",
                    )),
                    Err(error) => Note::bad(error),
                }));
            },
        )]))
        .child(dynamic(move || note_result(ui.custom_note.get()).into_element()))
}

// —— 直连出口 ——

fn outlet_card(ui: SplitUi) -> View {
    card_shell()
        .child(card_head(
            t("直连出口", "Direct outlet"),
            Some(
                t(
                    "直连的流量从哪块网卡出去、用哪个 DNS 解析。留空网卡表示自动判断。",
                    "Which adapter direct traffic leaves from and which DNS resolves it. Leave the adapter empty to auto-detect.",
                )
                .to_string(),
            ),
            view(),
        ))
        .child(field(
            t("直连 DNS", "Direct DNS"),
            Some(
                t(
                    "只填 IP（固定 53 端口）。选国内递归解析，直连站点才不会拿到境外节点。",
                    "An IP only (port 53). A domestic resolver keeps direct sites on nearby nodes.",
                )
                .to_string(),
            ),
            text_field(
                move || ui.direct_dns.get(),
                "223.5.5.5",
                true,
                move |text| {
                    ui.direct_dns.set(text);
                    ui.outlet_note.set(None);
                },
            ),
        ))
        .child(field(
            t("出口网卡", "Outlet adapter"),
            Some(
                t(
                    "网卡名或接口索引；留空则按路由表自动判断。",
                    "Adapter name or interface index; empty means auto-detect from the routing table.",
                )
                .to_string(),
            ),
            text_field(
                move || ui.direct_interface.get(),
                t("自动", "auto"),
                true,
                move |text| {
                    ui.direct_interface.set(text);
                    ui.outlet_note.set(None);
                },
            ),
        ))
        .child(action_row(vec![primary_button(
            t("保存出口", "Save outlet"),
            move || {
                let verdict =
                    state::save_split_outlet(ui.direct_dns.get(), ui.direct_interface.get());
                ui.outlet_note.set(Some(match verdict {
                    Ok(()) => Note::ok(t(
                        "已保存，重启隧道后生效",
                        "Saved; applies after a restart",
                    )),
                    Err(error) => Note::bad(error),
                }));
            },
        )]))
        .child(dynamic(move || note_result(ui.outlet_note.get()).into_element()))
}

// —— 最近判定 ——

fn decisions_card(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let runtime = snapshot.split.clone();
        if !runtime.running || runtime.decisions.is_empty() {
            return view().into_element();
        }
        card_shell()
            .child(card_head(
                t("最近判定", "Recent decisions"),
                Some(
                    t(
                        "最新的在最上面：这条连接走了直连还是代理、命中了什么。",
                        "Newest first: which way a connection went and what matched.",
                    )
                    .to_string(),
                ),
                view(),
            ))
            .children(runtime.decisions.iter().map(|decision| {
                let (label, color) = match (decision.route, decision.ok) {
                    (_, false) => (t("失败", "Failed"), danger_color()),
                    (DecisionRoute::Direct, true) => (t("直连", "Direct"), direct_color()),
                    (DecisionRoute::Proxy, true) => (t("代理", "Proxy"), proxy_color()),
                };
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        view().w(px(36.0)).child(
                            text(label)
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(move || color),
                        ),
                    )
                    .child(
                        view().flex_1().child(
                            text(decision.target.clone())
                                .text_xs()
                                .truncate()
                                .text_color(move || theme().card_foreground),
                        ),
                    )
                    .child(
                        text(decision.detail.clone())
                            .text_xs()
                            .truncate()
                            .text_color(move || theme().muted_foreground),
                    )
            }))
            .into_element()
    })
}

// —— 清单更新 ——

fn update_card(live: Signal<Live>) -> impl IntoElement {
    dynamic(move || {
        let snapshot = live.get();
        let update = snapshot.split_update.clone();
        let mut card = card_shell().child(card_head(
            t("清单更新", "List updates"),
            Some(
                t(
                    "内置清单随程序发布；也可以从这里从 APNIC 与 v2fly 重新拉一份，更新后重启隧道生效。",
                    "The lists ship with the app; you can also pull a fresh copy from APNIC and v2fly here. A restart applies it.",
                )
                .to_string(),
            ),
            ghost_button_maybe(
                t("立即更新", "Update now"),
                !update.is_busy(),
                move || state::update_split_lists(),
            ),
        ));
        if let Some(line) = update.text() {
            let tone = if matches!(update, SplitUpdate::Failed(_)) {
                danger_color()
            } else {
                ok_color()
            };
            card = card.child(note(line.to_string(), tone));
        }
        card.into_element()
    })
}
