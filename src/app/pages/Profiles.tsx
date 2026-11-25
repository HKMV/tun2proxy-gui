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
import { hue, ink } from "../theme";

/** DNS 三种模式（值是配置里的键，文案是界面自己的）。 */
const DNS_MODES = [
  {
    id: "direct",
    label: "直连",
    en: "Direct",
    hint: "交给隧道转发给远程 DNS，依赖代理的 UDP 能力",
    hintEn: "Forwarded through the tunnel to the remote DNS — needs UDP support upstream",
  },
  {
    id: "virtual",
    label: "虚拟 IP",
    en: "Virtual IP",
    hint: "本地直接应答，把域名映射进虚拟地址池",
    hintEn: "Answered locally: domain names are mapped into the virtual address pool",
  },
  {
    id: "over-tcp",
    label: "走 TCP",
    en: "Over TCP",
    hint: "DNS 查询改走 TCP，绕过只拦 UDP 53 的网络",
    hintEn: "DNS queries go over TCP, getting around networks that only intercept UDP 53",
  },
] as const;

/** 日志级别的五档。 */
const LEVELS = [
  { id: "error", label: "错误", en: "Error" },
  { id: "warn", label: "警告", en: "Warn" },
  { id: "info", label: "信息", en: "Info" },
  { id: "debug", label: "调试", en: "Debug" },
  { id: "trace", label: "详细", en: "Trace" },
] as const;

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
      error.set(result.error ?? t("保存失败", "Save failed"));
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
        title={t("配置", "Profiles")}
        hint={t(
          `共 ${props.view.items.length} 套，点一行就切过去；启动隧道用的是标着「当前」的那套`,
          `${props.view.items.length} saved. Click a row to switch — the tunnel starts with the one marked Current.`,
        )}
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
        <Button label={t("复制当前", "Duplicate current")} onPress={props.onDuplicate} />
        <Button label={t("新建", "New")} variant="primary" onPress={props.onNew} />
      </View>
      <Caption>
        {t(`配置存在 ${props.view.path}`, `Config file: ${props.view.path}`)}
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
        label={running() ? t("检测中…", "Testing…") : t("检测", "Test")}
        disabled={running()}
        onPress={() => api.checkProxy(props.item.proxy_url)}
      />
      <Button label={t("编辑", "Edit")} onPress={props.onEdit} />
      {props.confirming ? (
        <Button
          label={t("确认删除", "Confirm delete")}
          variant="danger"
          onPress={props.onConfirmDelete}
        />
      ) : (
        <Button
          label={t("删除", "Delete")}
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
        title={t("编辑这套配置", "Edit this profile")}
        hint={t(
          `正在改「${draft().name || "未命名配置"}」`,
          `Editing “${draft().name || "unnamed profile"}”`,
        )}
      />
      {/* 取消 / 保存放在卡片头下面一行：长表单改一两处就能直接保存，不用滚回顶部。 */}
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button label={t("取消", "Cancel")} onPress={props.onCancel} />
        <Button label={t("保存", "Save")} variant="primary" onPress={props.onSave} />
      </View>
      {props.error ? (
        <Text style={{ fontSize: 11, color: hue.danger() }}>{props.error}</Text>
      ) : null}

      <Field
        label={t("配置名称", "Profile name")}
        hint={t("配置列表里显示的名字", "The name shown in the profile list")}
      >
        <TextBox
          value={draft().name}
          placeholder={t("例如：公司代理", "e.g. Office proxy")}
          onChange={(value) => props.onPatch((next) => (next.name = value))}
        />
      </Field>

      <Field
        label={t("上游代理地址", "Upstream proxy")}
        hint={t("socks5:// 或 http:// 开头", "Starts with socks5:// or http://")}
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
          label={probe()?.running ? t("检测中…", "Testing…") : t("检测这个地址", "Test this address")}
          disabled={probe()?.running ?? false}
          onPress={() => api.checkProxy(draft().proxyUrl)}
        />
        {probe()?.detail ? (
          <Text style={{ fontSize: 10, color: probe()?.ok ? hue.ok() : hue.danger() }}>
            {probe()?.detail}
          </Text>
        ) : null}
      </View>

      <Field label={t("DNS 模式", "DNS mode")} hint={t(mode().hint, mode().hintEn)}>
        <Chips>
          {DNS_MODES.map((item) => (
            <Chip
              label={t(item.label, item.en)}
              chosen={draft().dns === item.id}
              onPress={() => props.onPatch((next) => (next.dns = item.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label={t("远程 DNS", "Remote DNS")}
        hint={
          virtual()
            ? t("「虚拟 IP」在本地应答，这一项不参与", "Virtual IP answers locally — this field is unused")
            : t("查询经隧道发往这台服务器", "Queries travel through the tunnel to this server")
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
        label={t("虚拟地址池", "Virtual pool")}
        hint={
          virtual()
            ? t(
                "接管 DNS 时分配给域名的假地址段",
                "The fake range handed out to domain names while DNS is intercepted",
              )
            : t("只有「虚拟 IP」模式会用到", "Only used by Virtual IP mode")
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
        label={t("网卡名称", "Adapter name")}
        hint={t("留空则每次启动随机生成", "Leave empty to generate a random one per start")}
      >
        <TextBox
          value={draft().tunName}
          placeholder="tun0"
          onChange={(value) => props.onPatch((next) => (next.tunName = value))}
        />
      </Field>

      <View style={{ flexDirection: "row", gap: 12 }}>
        <View style={{ flex: 1 }}>
          <Field label="MTU" hint={t("默认 1500", "Default 1500")}>
            <TextBox
              value={draft().mtu}
              onChange={(value) => props.onPatch((next) => (next.mtu = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field
            label={t("最大会话数", "Max sessions")}
            hint={t("并发上限；顶到上限只丢新连接", "Concurrency cap; extras are dropped")}
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
          <Field label={t("TCP 超时", "TCP timeout")} hint={t("秒；默认 600", "Seconds; default 600")}>
            <TextBox
              value={draft().tcpTimeout}
              onChange={(value) => props.onPatch((next) => (next.tcpTimeout = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field label={t("UDP 超时", "UDP timeout")} hint={t("秒；默认 10", "Seconds; default 10")}>
            <TextBox
              value={draft().udpTimeout}
              onChange={(value) => props.onPatch((next) => (next.udpTimeout = value))}
            />
          </Field>
        </View>
      </View>

      <Field
        label={t("日志级别", "Log level")}
        hint={t("子进程输出的详细程度", "How verbose the child process is")}
      >
        <Chips>
          {LEVELS.map((level) => (
            <Chip
              label={t(level.label, level.en)}
              chosen={draft().verbosity === level.id}
              onPress={() => props.onPatch((next) => (next.verbosity = level.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label={t("绕过地址", "Bypass addresses")}
        hint={t(
          "每行一个 IP 或 CIDR；这些地址不进隧道",
          "One IP or CIDR per line — these never enter the tunnel",
        )}
        right={
          <Button
            label={t("填入局域网默认值", "Fill in LAN defaults")}
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
