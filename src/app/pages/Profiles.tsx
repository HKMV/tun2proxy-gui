//! 「代理配置」页：多套配置的切换与编辑。
//!
//! 一页两态：**列表**（点一行切过去、删、检测）和**编辑**（一套配置的全部参数）。
//! 没有做成两个页面：这一页的常驻问题是「我现在用的是哪套、它配的是什么」，
//! 切换和编辑挤在同一屏里更好比；但窗口最小 780dp 放不下左右两栏，所以用同一
//! 位置的两种状态。
//!
//! 表单是**草稿态**：改字段只改内存里的 [`api.Draft`]，点「保存」才落盘——改一半
//! 反悔可以直接返回，不会把配置写成半成品。数字字段保持字符串，解析与范围检查
//! 在后端（`api::profiles`），错误文案也由后端给。

import { signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "../api";
import { Button, Card, CardHead, Caption, Chip, Chips, Field, TextBox } from "../components";
import { t } from "../i18n";
import type { MessageKey } from "../i18n";
import { hue, ink } from "../theme";

/** DNS 三种模式（值是配置里的键，文案在 `locales/` 里）。 */
const DNS_MODES = [
  {
    id: "direct",
    label: "profiles.dns.direct",
    hint: "profiles.dns.direct.hint",
  },
  {
    id: "virtual",
    label: "profiles.dns.virtual",
    hint: "profiles.dns.virtual.hint",
  },
  {
    id: "over-tcp",
    label: "profiles.dns.overTcp",
    hint: "profiles.dns.overTcp.hint",
  },
] as const satisfies readonly { id: string; label: MessageKey; hint: MessageKey }[];

/** 日志级别的五档。 */
const LEVELS = [
  { id: "error", label: "profiles.level.error" },
  { id: "warn", label: "profiles.level.warn" },
  { id: "info", label: "profiles.level.info" },
  { id: "debug", label: "profiles.level.debug" },
  { id: "trace", label: "profiles.level.trace" },
] as const satisfies readonly { id: string; label: MessageKey }[];

/** 检测结论按地址存（去空白 + 小写），和内核那边同一个键。 */
function probeKey(url: string): string {
  return url.trim().toLowerCase();
}

export function ProfilesPage(props: { snap: () => api.Snapshot }): JSX.Element {
  // 配置、列表/编辑态、草稿、错误、待确认删除的下标。
  const view = signal<api.ProfilesView>(api.profiles());
  const editing = signal<number | null>(null);
  const draft = signal<api.Draft | null>(null);
  const error = signal<string | null>(null);
  const confirm = signal<number | null>(null);

  const reload = () => {
    view.set(api.profiles());
  };

  const beginEdit = (index: number) => {
    const next = api.profileDraft(index);
    if (!next) return;
    draft.set(next);
    editing.set(index);
    error.set(null);
    confirm.set(null);
  };

  const endEdit = () => {
    editing.set(null);
    draft.set(null);
    error.set(null);
  };

  /** 改草稿里的一个字段：改了就把上一次的错误收起来（那条结论说的是改之前的值）。 */
  const patch = (change: (draft: api.Draft) => void) => {
    const current = draft();
    if (!current) return;
    const next = { ...current };
    change(next);
    draft.set(next);
    error.set(null);
  };

  const save = (index: number) => {
    const current = draft();
    if (!current) return;
    const result = api.saveProfile(index, current);
    if (!result.ok) {
      error.set(result.error ?? t("profiles.saveFailed"));
      return;
    }
    reload();
    endEdit();
  };

  const create = (make: () => { ok: boolean; index: number | null }) => {
    const result = make();
    if (!result.ok || result.index === null) return;
    reload();
    beginEdit(result.index);
  };

  const choose = (index: number) => {
    if (api.selectProfile(index).ok) reload();
  };

  const remove = (index: number) => {
    confirm.set(null);
    if (api.removeProfile(index).ok) reload();
  };

  // 列表 / 编辑态的选择必须在 JSX 里做：nsh 的组件体只跑一次，body 里的
  // `if (editing())` 会永远停在首次的结论上。
  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      {editing() === null ? (
        <List
          view={view()}
          snap={props.snap()}
          confirm={confirm()}
          onChoose={choose}
          onEdit={beginEdit}
          onConfirm={remove}
          onAskDelete={(at: number) => confirm.set(at)}
          onCancelDelete={() => confirm.set(null)}
          onDuplicate={() => create(api.duplicateProfile)}
          onNew={() => create(api.addProfile)}
        />
      ) : (
        <Editor
          index={editing() as number}
          // 传访问器而不是 `draft()`：这个位置每求值一次就会重建整棵表单，
          // 而草稿每敲一个字都会换一个对象——求值成对象的话输入框会被
          // 重新挂载、焦点跟着丢，表现就是「只能打进去一个字」。
          draft={draft}
          error={error()}
          snap={props.snap()}
          onPatch={patch}
          onCancel={endEdit}
          onSave={() => save(editing() as number)}
        />
      )}
    </View>
  );
}

/** 列表态。 */
function List(props: {
  view: api.ProfilesView;
  snap: api.Snapshot;
  confirm: number | null;
  onChoose: (index: number) => void;
  onEdit: (index: number) => void;
  onConfirm: (index: number) => void;
  onAskDelete: (index: number) => void;
  onCancelDelete: () => void;
  onDuplicate: () => void;
  onNew: () => void;
}): JSX.Element {
  return (
    <Card>
      <CardHead
        title={t("profiles.title")}
        hint={t("profiles.savedCount", { count: props.view.items.length })}
      />
      <View style={{ flexDirection: "column", gap: 4 }}>
        {props.view.items.map((item, index) => (
          <ProfileRow
            item={item}
            active={props.view.active === index}
            onlyOne={props.view.items.length <= 1}
            confirming={props.confirm === index}
            probe={props.snap.probes[probeKey(item.proxy_url)]}
            onChoose={() => props.onChoose(index)}
            onEdit={() => props.onEdit(index)}
            onAskDelete={() => props.onAskDelete(index)}
            onCancelDelete={props.onCancelDelete}
            onConfirmDelete={() => props.onConfirm(index)}
          />
        ))}
      </View>
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button label={t("profiles.duplicate")} onPress={props.onDuplicate} />
        <Button label={t("common.new")} variant="primary" onPress={props.onNew} />
      </View>
      <Caption>
        {t("profiles.configFile", { path: props.view.path })}
      </Caption>
    </Card>
  );
}

/** 列表里的一套配置。 */
function ProfileRow(props: {
  item: api.ProfileItem;
  active: boolean;
  onlyOne: boolean;
  confirming: boolean;
  probe: api.Probe | undefined;
  onChoose: () => void;
  onEdit: () => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onConfirmDelete: () => void;
}): JSX.Element {
  const running = () => props.probe?.running ?? false;
  const done = () => (props.probe?.detail ? props.probe : undefined);
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 12,
        padding: 12,
        borderRadius: 10,
      }}
    >
      {/* 竖条只在当前那套上出现：颜色之外再给一个形状上的区别。 */}
      <View
        style={{
          width: 4,
          height: 28,
          borderRadius: 2,
          backgroundColor: props.active ? hue.primary() : "transparent",
        }}
      />
      {/* 整块文字区才是「切过去」的点击区：右侧三个按钮不能被整行点击抢掉。 */}
      <View
        onPress={props.active ? undefined : props.onChoose}
        style={{
          flex: 1,
          flexDirection: "column",
          gap: 2,
          paddingLeft: 8,
          paddingRight: 8,
          paddingTop: 4,
          paddingBottom: 4,
          borderRadius: 8,
          cursor: props.active ? "default" : "pointer",
          transitionProperty: "background-color",
          transitionDuration: "120ms",
        }}
        hoverStyle={props.active ? undefined : { backgroundColor: hue.accent() }}
      >
        <Text
          style={{
            fontSize: 12,
            fontWeight: "600",
            color: props.active ? hue.primary() : ink.card(),
          }}
        >
          {props.item.displayName}
        </Text>
        <Text style={{ fontSize: 10, color: ink.muted() }}>{props.item.summary}</Text>
        {done() ? (
          <Text style={{ fontSize: 10, color: done()!.ok ? hue.ok() : hue.danger() }}>
            {done()!.detail ?? ""}
          </Text>
        ) : null}
      </View>
      <Button
        label={running() ? t("profiles.testing") : t("profiles.test")}
        disabled={running()}
        onPress={() => api.checkProxy(props.item.proxy_url)}
      />
      <Button label={t("common.edit")} onPress={props.onEdit} />
      {props.confirming ? (
        <Button
          label={t("profiles.deleteConfirm")}
          variant="danger"
          onPress={props.onConfirmDelete}
        />
      ) : (
        <Button
          label={t("common.delete")}
          disabled={props.onlyOne}
          onPress={props.onlyOne ? undefined : props.onAskDelete}
        />
      )}
    </View>
  );
}

/** 编辑态：一套配置的全部参数。 */
function Editor(props: {
  index: number;
  draft: () => api.Draft | null;
  error: string | null;
  snap: api.Snapshot;
  onPatch: (change: (draft: api.Draft) => void) => void;
  onCancel: () => void;
  onSave: () => void;
}): JSX.Element {
  // 组件体只跑一次，所以跟着信号变的东西都写成函数，在 JSX 里求值。
  const draft = () => props.draft() as api.Draft;

  // DNS 模式决定哪两个字段参与：虚拟 IP 本地应答，远程 DNS 不参与；虚拟池只有它用。
  const virtual = () => draft().dns === "virtual";
  const mode = () => DNS_MODES.find((item) => item.id === draft().dns) ?? DNS_MODES[0];
  const probe = () => props.snap.probes[probeKey(draft().proxyUrl)];

  return (
    <Card>
      <CardHead
        title={t("profiles.editThis")}
        hint={t("profiles.editing", { name: draft().name || t("profiles.unnamed") })}
      />
      {/* 取消 / 保存放在卡片头下面一行：长表单改一两处就能直接保存，不用滚回顶部。 */}
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button label={t("common.cancel")} onPress={props.onCancel} />
        <Button label={t("common.save")} variant="primary" onPress={props.onSave} />
      </View>
      {props.error ? (
        <Text style={{ fontSize: 11, color: hue.danger() }}>{props.error}</Text>
      ) : null}

      <Field
        label={t("profiles.name")}
        hint={t("profiles.nameHint")}
      >
        <TextBox
          value={draft().name}
          placeholder={t("profiles.namePlaceholder")}
          onChange={(value) => props.onPatch((next) => (next.name = value))}
        />
      </Field>

      <Field
        label={t("profiles.upstream")}
        hint={t("profiles.upstreamHint")}
      >
        <TextBox
          value={draft().proxyUrl}
          placeholder="socks5://127.0.0.1:7890"
          onChange={(value) => props.onPatch((next) => (next.proxyUrl = value))}
        />
      </Field>
      {/* 填完地址就地测一次：这里才是「刚输完」的位置，回列表再测要多跳一步。 */}
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        <Button
          label={probe()?.running ? t("profiles.testing") : t("profiles.testThis")}
          disabled={probe()?.running ?? false}
          onPress={() => api.checkProxy(draft().proxyUrl)}
        />
        {probe()?.detail ? (
          <Text style={{ fontSize: 10, color: probe()?.ok ? hue.ok() : hue.danger() }}>
            {probe()?.detail}
          </Text>
        ) : null}
      </View>

      <Field label={t("profiles.dnsMode")} hint={t(mode().hint)}>
        <Chips>
          {DNS_MODES.map((item) => (
            <Chip
              label={t(item.label)}
              chosen={draft().dns === item.id}
              onPress={() => props.onPatch((next) => (next.dns = item.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label={t("profiles.remoteDns")}
        hint={
          virtual()
            ? t("profiles.virtualIpUnused")
            : t("profiles.remoteDnsHint")
        }
      >
        <TextBox
          value={draft().dnsAddr}
          placeholder="8.8.8.8"
          disabled={virtual()}
          onChange={(value) => props.onPatch((next) => (next.dnsAddr = value))}
        />
      </Field>

      <Field
        label={t("profiles.virtualPool")}
        hint={
          virtual()
            ? t("profiles.virtualPoolFieldHint")
            : t("profiles.onlyVirtualIp")
        }
      >
        <TextBox
          value={draft().virtualDnsPool}
          placeholder="198.18.0.0/15"
          disabled={!virtual()}
          onChange={(value) => props.onPatch((next) => (next.virtualDnsPool = value))}
        />
      </Field>

      <Field
        label={t("profiles.adapter")}
        hint={t("profiles.virtualPoolHint")}
      >
        <TextBox
          value={draft().tunName}
          placeholder="tun0"
          onChange={(value) => props.onPatch((next) => (next.tunName = value))}
        />
      </Field>

      <View style={{ flexDirection: "row", gap: 12 }}>
        <View style={{ flex: 1 }}>
          <Field label="MTU" hint={t("profiles.mtuHint")}>
            <TextBox
              value={draft().mtu}
              onChange={(value) => props.onPatch((next) => (next.mtu = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field
            label={t("profiles.maxSessions")}
            hint={t("profiles.maxSessionsHint")}
          >
            <TextBox
              value={draft().maxSessions}
              onChange={(value) => props.onPatch((next) => (next.maxSessions = value))}
            />
          </Field>
        </View>
      </View>

      <View style={{ flexDirection: "row", gap: 12 }}>
        <View style={{ flex: 1 }}>
          <Field label={t("profiles.tcpTimeout")} hint={t("profiles.udpTimeoutHint")}>
            <TextBox
              value={draft().tcpTimeout}
              onChange={(value) => props.onPatch((next) => (next.tcpTimeout = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field label={t("profiles.udpTimeout")} hint={t("profiles.tcpTimeoutHint")}>
            <TextBox
              value={draft().udpTimeout}
              onChange={(value) => props.onPatch((next) => (next.udpTimeout = value))}
            />
          </Field>
        </View>
      </View>

      <Field
        label={t("profiles.logLevel")}
        hint={t("profiles.logLevelHint")}
      >
        <Chips>
          {LEVELS.map((level) => (
            <Chip
              label={t(level.label)}
              chosen={draft().verbosity === level.id}
              onPress={() => props.onPatch((next) => (next.verbosity = level.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label={t("profiles.bypass")}
        hint={t("profiles.bypassHint")}
        right={
          <Button
            label={t("profiles.fillLanDefaults")}
            onPress={() =>
              props.onPatch((next) => (next.bypass = api.defaults().bypass))
            }
          />
        }
      >
        <TextBox
          value={draft().bypass}
          placeholder="10.0.0.0/8"
          multiline
          height={132}
          onChange={(value) => props.onPatch((next) => (next.bypass = value))}
        />
      </Field>
    </Card>
  );
}
