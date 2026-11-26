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
  text: t("common.savedRestart"),
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
          title={t("split.directChina")}
          hint={t("split.directChinaHint")}
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
              ? t("split.off")
              : props.snap().split.running
                ? t("split.running")
                : t("split.on")}
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
            text={t("split.dnsModeWarning", { mode: view().dnsModeLabel })}
          />
        ) : null}
      </Card>

      {/* 内置清单 */}
      <Card>
        <CardHead
          title={t("split.builtinLists")}
          hint={t("split.groupsHint")}
        />
        <View style={{ flexDirection: "column", gap: 12 }}>
          {view().groups.map((group) => (
            <Setting
              label={t("split.groupCount", { label: group.label, count: group.count })}
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
          title={t("split.customRules")}
          hint={t("split.customRulesHint")}
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
            label={t("split.saveRules")}
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
          title={t("split.ruleDetails")}
          hint={t("split.builtinListsHint")}
        />
        <Field
          label={t("split.search")}
          hint={t("split.searchHint")}
        >
          <TextBox
            value={filter()}
            placeholder={t("split.searchPlaceholder")}
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
              ? t("split.emptySearch")
              : t("split.emptyAllOff")
            : rows().length >= RULE_ROWS
              ? t("split.rulesTruncated", { rows: RULE_ROWS })
              : t("split.allListed", { count: rows().length })}
        </Caption>
      </Card>

      {/* 直连出口 */}
      <Card>
        <CardHead
          title={t("split.directEgress")}
          hint={t("split.egressHint")}
        />
        <Field
          label={t("split.directDns")}
          hint={t("split.directDnsHint")}
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
          label={t("split.egressAdapter")}
          hint={t("split.egressAdapterHint")}
        >
          <TextBox
            value={directInterface()}
            placeholder={t("split.auto")}
            onChange={(value) => {
              directInterface.set(value);
              outletVerdict.set(null);
            }}
          />
        </Field>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Button
            label={t("split.saveEgress")}
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
            title={t("split.recent")}
            hint={t("split.recentHint")}
            right={<Caption>{t("split.count", { count: props.snap().split.decisions.length })}</Caption>}
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
                      ? t("common.failed")
                      : decision.route === "direct"
                        ? t("common.direct")
                        : t("common.proxy")}
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
          title={t("split.listUpdates")}
          hint={t("split.listUpdatesHint")}
          right={
            <Button
              label={
                props.snap().splitUpdate.busy ? t("split.updating") : t("split.updateNow")
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
    return t("split.status", { addr, rules: snap.split.rules, direct: snap.split.direct, proxy: snap.split.proxy, failed: snap.split.failed });
  }
  if (enabled) {
    return t("split.statusHint");
  }
  return t("split.disabledHint");
}

/** 动作结论 → 一句提示；成功时顺手让页面重读一次配置。 */
function save(result: api.Outcome, reload: () => void): Verdict {
  if (result.ok) {
    reload();
    return saved();
  }
  return { tone: "bad", text: result.error ?? t("common.saveFailedLog") };
}
