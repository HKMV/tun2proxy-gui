//! 分流规则页：总开关、内置清单、自定义规则、明细、直连出口、最近判定。
//!
//! 这一页的设置**大多要重启隧道才生效**：分流器是隧道启动时按当时的设置拉起来
//! 的。所以每块都把这句话写在说明里，而不是让用户改完去猜为什么没变化。
//!
//! # 哪些是草稿
//!
//! 只有两段长文本是草稿（自定义规则、直连出口）：用户一行行敲出来的东西，点
//! 保存才进配置。开关类（总闸、分组、单条规则）改完立刻落盘——它们没有「敲到
//! 一半」的中间态，多一步保存只会让人以为没生效。

import { effect, signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "../api";
import {
  Button,
  Card,
  CardHead,
  Caption,
  Chip,
  Chips,
  Field,
  Note,
  ScrollArea,
  Setting,
  TextBox,
  Toggle,
} from "../components";
import { t } from "../i18n";
import { hue, ink } from "../theme";

/** 明细最多列出多少条（和后端 `RULE_ROWS` 对齐）。 */
const RULE_ROWS = 100;

/** 最近判定显示多少条。 */
const DECISIONS_SHOWN = 12;

/** 一句结论（保存成功 / 失败原因）。 */
type Verdict = { tone: "ok" | "bad"; text: string } | null;

/** 保存成功的统一说法：这一页改完都要重启。写成函数——语言会变，常量会冻住。 */
const saved = (): Verdict => ({
  tone: "ok",
  text: t("已保存，重启隧道后生效", "Saved — restart the tunnel to apply"),
});

export function SplitPage(props: { snap: () => api.Snapshot }): JSX.Element {
  // 设置本体 + 一个「刚改过一次」的计数器：明细要跟着组开关、行开关一起变。
  const view = signal<api.SplitView>(api.splitView());
  const rev = signal(0);
  const rows = signal<api.RuleView[]>(api.splitRules("", RULE_ROWS));

  // 两段草稿各自的结论。
  const custom = signal(view().custom);
  const customVerdict = signal<Verdict>(null);
  const directDns = signal(view().directDns);
  const directInterface = signal(view().directInterface);
  const outletVerdict = signal<Verdict>(null);
  // 明细的查找词（只影响显示，不落盘）。
  const filter = signal("");

  const reload = () => {
    view.set(api.splitView());
    rev.update((tick) => tick + 1);
  };

  // 查找词或设置一变就重取明细：一次只取 100 条，后端按组顺序截断。
  effect(() => {
    rev();
    rows.set(api.splitRules(filter(), RULE_ROWS));
  });

  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      {/* 总开关 */}
      <Card>
        <CardHead
          title={t("国内流量直连", "Direct China traffic")}
          hint={t(
            "命中国内清单的域名与 IP 用本地网络直连，其余照常走代理。改完要重启隧道才生效。",
            "Domains and IPs on the China lists go out over the local network; everything else still uses the proxy. Restart the tunnel to apply.",
          )}
          right={
            <Toggle
              on={view().enabled}
              onToggle={() => {
                api.splitToggleEnabled();
                reload();
              }}
            />
          }
        />
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Text
            style={{
              fontSize: 12,
              fontWeight: "600",
              color: !view().enabled
                ? ink.muted()
                : props.snap().split.running
                  ? hue.ok()
                  : hue.warn(),
            }}
          >
            {!view().enabled
              ? t("已关闭", "Off")
              : props.snap().split.running
                ? t("运行中", "Running")
                : t("已开启，隧道启动后生效", "On — applies once the tunnel starts")}
          </Text>
          <Text style={{ fontSize: 11, color: ink.muted(), flex: 1 }}>
            {splitDetail(props.snap(), view().enabled)}
          </Text>
        </View>
        {/* 域名清单只在「虚拟 IP」策略下看得见：别的策略里 tun2proxy 交给分流器的
            只有 IP 字面量，不解释的话用户会以为国内域名都配上了。 */}
        {view().enabled && view().dnsMode !== "virtual" ? (
          <Note
            tone="bad"
            text={t(
              `当前 DNS 策略是「${view().dnsModeLabel}」：tun2proxy 只能看到 IP，域名清单不会生效，只有 IP 段在分流。把 DNS 策略改成「虚拟 IP」才能用上域名规则。`,
              `The DNS mode is “${view().dnsModeLabel}”: tun2proxy only sees IPs, so the domain lists do nothing and only IP ranges are split. Switch the DNS mode to “Virtual IP” to use the domain rules.`,
            )}
          />
        ) : null}
      </Card>

      {/* 内置清单 */}
      <Card>
        <CardHead
          title={t("内置清单", "Built-in lists")}
          hint={t(
            "按组开关；关掉的那一组不参与匹配，流量照常走代理。",
            "Toggle per group — a disabled group is not matched and its traffic goes through the proxy.",
          )}
        />
        <View style={{ flexDirection: "column", gap: 12 }}>
          {view().groups.map((group) => (
            <Setting
              label={t(`${group.label}（${group.count} 条）`, `${group.label} (${group.count} rules)`)}
              hint={group.hint}
            >
              <Toggle
                on={group.enabled}
                onToggle={() => {
                  api.splitSetGroup(group.id, !group.enabled);
                  reload();
                }}
              />
            </Setting>
          ))}
        </View>
      </Card>

      {/* 自定义规则 */}
      <Card>
        <CardHead
          title={t("自定义规则", "Custom rules")}
          hint={t(
            "一行一条：qq.com（后缀）、full:www.qq.com（精确）、keyword:bilili（包含）、203.0.113.0/24（网段）、proxy:example.org（强制走代理）。# 后面是注释。",
            "One per line: qq.com (suffix), full:www.qq.com (exact), keyword:bilili (contains), 203.0.113.0/24 (range), proxy:example.org (force through the proxy). Text after # is a comment.",
          )}
        />
        <TextBox
          value={custom()}
          placeholder={"qq.com\nproxy:img.example.org"}
          multiline
          height={148}
          onChange={(value) => {
            custom.set(value);
            // 改了就把上一次的结论收起来：那句话说的是改之前的内容。
            customVerdict.set(null);
          }}
        />
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Button
            label={t("保存规则", "Save rules")}
            variant="primary"
            onPress={() => customVerdict.set(save(api.splitSaveCustom(custom()), reload))}
          />
          {customVerdict() ? (
            <Note tone={customVerdict()!.tone} text={customVerdict()!.text} />
          ) : null}
        </View>
      </Card>

      {/* 规则明细 */}
      <Card>
        <CardHead
          title={t("规则明细", "Rule details")}
          hint={t(
            "内置清单展开成一条条规则；关掉的那条不再直连。改完要重启隧道才生效。",
            "The built-in lists expanded rule by rule — a disabled rule no longer goes direct. Restart the tunnel to apply.",
          )}
        />
        <Field
          label={t("查找", "Search")}
          hint={t("输入域名或地址的一部分，只影响这里的显示", "Part of a domain or address — filters this list only")}
        >
          <TextBox
            value={filter()}
            placeholder={t("例如 qq.com", "e.g. qq.com")}
            onChange={(value) => filter.set(value)}
          />
        </Field>
        {/* 明细单独限高：一百行全摊开会把下面两张卡推到几千像素之外，
            用户想改出口就得滚很久。 */}
        <View style={{ height: 340, borderRadius: 10, backgroundColor: hue.muted() }}>
          <ScrollArea style={{ padding: 6 }}>
            {rows().map((row) => (
              <View
                style={{
                  flexDirection: "row",
                  alignItems: "center",
                  gap: 12,
                  paddingLeft: 6,
                  // 右边多留 10 点：滚动条是自绘的、压在内容上，留出这条空隙
                  // 开关才不会被它压住。
                  paddingRight: 14,
                  // 上下 5 点：开关 22 高，行距太窄时一排胶囊会挤成一条。
                  paddingTop: 5,
                  paddingBottom: 5,
                  borderRadius: 6,
                }}
                hoverStyle={{ backgroundColor: hue.accent() }}
              >
                <Text
                  style={{
                    flex: 1,
                    fontSize: 11,
                    color: row.enabled ? ink.card() : ink.muted(),
                  }}
                >
                  {row.text}
                </Text>
                <Text style={{ width: 60, fontSize: 10, color: ink.muted() }}>{row.kind}</Text>
                <Text style={{ width: 96, fontSize: 10, color: ink.muted() }}>{row.source}</Text>
                <Toggle
                  on={row.enabled}
                  onToggle={() => {
                    api.splitToggleRow(row.key);
                    reload();
                  }}
                />
              </View>
            ))}
          </ScrollArea>
        </View>
        <Caption>
          {rows().length === 0
            ? filter().trim().length > 0
              ? t("没有匹配的规则", "No matching rule")
              : t("没有可显示的规则：上面三组清单都关掉了", "Nothing to show — all three groups above are off")
            : rows().length >= RULE_ROWS
              ? t(
                  `只显示前 ${RULE_ROWS} 条，用上面的查找框定位更靠后的规则`,
                  `Showing the first ${RULE_ROWS} — use the search box above to reach later rules`,
                )
              : t(`共 ${rows().length} 条，已全部列出`, `${rows().length} rules, all listed`)}
        </Caption>
      </Card>

      {/* 直连出口 */}
      <Card>
        <CardHead
          title={t("直连出口", "Direct egress")}
          hint={t(
            "直连的流量从哪块网卡出去、用哪个 DNS 解析。留空网卡表示自动判断。",
            "Which adapter direct traffic leaves through, and which DNS resolves it. Leave the adapter empty to pick automatically.",
          )}
        />
        <Field
          label={t("直连 DNS", "Direct DNS")}
          hint={t(
            "只填 IP（固定 53 端口）。选国内递归解析，直连站点才不会拿到境外节点。",
            "IP only (port 53). Use a domestic resolver so direct sites are not handed overseas nodes.",
          )}
        >
          <TextBox
            value={directDns()}
            placeholder={view().defaultDirectDns}
            onChange={(value) => {
              directDns.set(value);
              outletVerdict.set(null);
            }}
          />
        </Field>
        <Field
          label={t("出口网卡", "Egress adapter")}
          hint={t(
            "网卡名或接口索引；留空则按路由表自动判断。",
            "Adapter name or interface index; empty means the routing table decides.",
          )}
        >
          <TextBox
            value={directInterface()}
            placeholder={t("自动", "Auto")}
            onChange={(value) => {
              directInterface.set(value);
              outletVerdict.set(null);
            }}
          />
        </Field>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Button
            label={t("保存出口", "Save egress")}
            variant="primary"
            onPress={() =>
              outletVerdict.set(
                save(api.splitSaveOutlet(directDns(), directInterface()), reload),
              )
            }
          />
          {outletVerdict() ? (
            <Note tone={outletVerdict()!.tone} text={outletVerdict()!.text} />
          ) : null}
        </View>
      </Card>

      {/* 最近判定：没有记录时整张卡不出现，免得空着占一屏。 */}
      {props.snap().split.decisions.length === 0 ? null : (
        <Card>
          <CardHead
            title={t("最近判定", "Recent decisions")}
            hint={t(
              "最新的在最上面：这条连接走了直连还是代理、命中了什么。",
              "Newest first — whether the connection went direct or through the proxy, and why.",
            )}
            right={<Caption>{t(`${props.snap().split.decisions.length} 条`, `${props.snap().split.decisions.length} entries`)}</Caption>}
          />
          <View style={{ flexDirection: "column", gap: 4 }}>
            {props.snap()
              .split.decisions.slice(0, DECISIONS_SHOWN)
              .map((decision) => (
                <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
                  <Text
                    style={{
                      width: 36,
                      fontSize: 10,
                      fontWeight: "600",
                      color: !decision.ok
                        ? hue.danger()
                        : decision.route === "direct"
                          ? hue.direct()
                          : hue.proxy(),
                    }}
                  >
                    {!decision.ok
                      ? t("失败", "Failed")
                      : decision.route === "direct"
                        ? t("直连", "Direct")
                        : t("代理", "Proxy")}
                  </Text>
                  <Text style={{ flex: 1, fontSize: 10, color: ink.card() }}>
                    {decision.target}
                  </Text>
                  <Text style={{ width: 220, fontSize: 10, color: ink.muted() }}>
                    {decision.detail}
                  </Text>
                </View>
              ))}
          </View>
        </Card>
      )}

      {/* 清单更新 */}
      <Card>
        <CardHead
          title={t("清单更新", "List updates")}
          hint={t(
            "内置清单随程序发布；也可以从这里从 APNIC 与 v2fly 重新拉一份，更新后重启隧道生效。",
            "The lists ship with the app; you can also re-fetch them from APNIC and v2fly here. Restart the tunnel to apply.",
          )}
          right={
            <Button
              label={
                props.snap().splitUpdate.busy ? t("更新中…", "Updating…") : t("立即更新", "Update now")
              }
              disabled={props.snap().splitUpdate.busy}
              onPress={() => api.splitUpdateLists()}
            />
          }
        />
        {props.snap().splitUpdate.text ? (
          <Note
            tone={props.snap().splitUpdate.failed ? "bad" : "ok"}
            text={props.snap().splitUpdate.text ?? ""}
          />
        ) : null}
      </Card>
    </View>
  );
}

/** 总开关下面那行说明：关着时解释为什么不生效，跑起来才摆出地址与计数。 */
function splitDetail(snap: api.Snapshot, enabled: boolean): string {
  if (snap.split.running) {
    const addr = snap.split.addr ?? "127.0.0.1";
    return t(
      `监听 ${addr}，生效 ${snap.split.rules} 条规则 · 本次直连 ${snap.split.direct} / 走代理 ${snap.split.proxy} / 失败 ${snap.split.failed}`,
      `Listening on ${addr}, ${snap.split.rules} rules active · this run: direct ${snap.split.direct} / proxied ${snap.split.proxy} / failed ${snap.split.failed}`,
    );
  }
  if (enabled) {
    return t(
      "隧道启动后这里显示监听地址与生效条数",
      "Once the tunnel starts, the listen address and active rule count show up here",
    );
  }
  return t(
    "所有流量照常走上游代理，与没有这个功能时完全一致",
    "All traffic goes through the upstream proxy, exactly as without this feature",
  );
}

/** 动作结论 → 一句提示；成功时顺手让页面重读一次配置。 */
function save(result: api.Outcome, reload: () => void): Verdict {
  if (result.ok) {
    reload();
    return saved();
  }
  return { tone: "bad", text: result.error ?? t("保存失败，看看日志里的原因", "Save failed — check the log for the reason") };
}
