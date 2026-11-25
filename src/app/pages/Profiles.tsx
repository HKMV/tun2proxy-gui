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
import { hue, ink } from "../theme";

/** DNS 三种模式（值是配置里的键，文案是界面自己的）。 */
const DNS_MODES = [
  { id: "direct", label: "直连", hint: "交给隧道转发给远程 DNS，依赖代理的 UDP 能力" },
  { id: "virtual", label: "虚拟 IP", hint: "本地直接应答，把域名映射进虚拟地址池" },
  { id: "over-tcp", label: "走 TCP", hint: "DNS 查询改走 TCP，绕过只拦 UDP 53 的网络" },
] as const;

/** 日志级别的五档。 */
const LEVELS = [
  { id: "error", label: "错误" },
  { id: "warn", label: "警告" },
  { id: "info", label: "信息" },
  { id: "debug", label: "调试" },
  { id: "trace", label: "详细" },
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
      error.set(result.error ?? "保存失败");
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
      ) : draft() === null ? null : (
        <Editor
          index={editing() as number}
          draft={draft()}
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
        title="配置"
        hint={`共 ${props.view.items.length} 套，点一行就切过去；启动隧道用的是标着「当前」的那套`}
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
        <Button label="复制当前" onPress={props.onDuplicate} />
        <Button label="新建" variant="primary" onPress={props.onNew} />
      </View>
      <Caption>{`配置存在 ${props.view.path}`}</Caption>
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
        label={running() ? "检测中…" : "检测"}
        disabled={running()}
        onPress={() => api.checkProxy(props.item.proxy_url)}
      />
      <Button label="编辑" onPress={props.onEdit} />
      {props.confirming ? (
        <Button label="确认删除" variant="danger" onPress={props.onConfirmDelete} />
      ) : (
        <Button
          label="删除"
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
  draft: api.Draft | null;
  error: string | null;
  snap: api.Snapshot;
  onPatch: (change: (draft: api.Draft) => void) => void;
  onCancel: () => void;
  onSave: () => void;
}): JSX.Element {
  // 组件体只跑一次，所以跟着信号变的东西都写成函数，在 JSX 里求值。
  const draft = () => props.draft as api.Draft;

  // DNS 模式决定哪两个字段参与：虚拟 IP 本地应答，远程 DNS 不参与；虚拟池只有它用。
  const virtual = () => draft().dns === "virtual";
  const mode = () => DNS_MODES.find((item) => item.id === draft().dns) ?? DNS_MODES[0];
  const probe = () => props.snap.probes[probeKey(draft().proxyUrl)];

  return (
    <Card>
      <CardHead title="编辑这套配置" hint={`正在改「${draft().name || "未命名配置"}」`} />
      {/* 取消 / 保存放在卡片头下面一行：长表单改一两处就能直接保存，不用滚回顶部。 */}
      <View style={{ flexDirection: "row", gap: 8 }}>
        <Button label="取消" onPress={props.onCancel} />
        <Button label="保存" variant="primary" onPress={props.onSave} />
      </View>
      {props.error ? (
        <Text style={{ fontSize: 11, color: hue.danger() }}>{props.error}</Text>
      ) : null}

      <Field label="配置名称" hint="配置列表里显示的名字">
        <TextBox
          value={draft().name}
          placeholder="例如：公司代理"
          onChange={(value) => props.onPatch((next) => (next.name = value))}
        />
      </Field>

      <Field label="上游代理地址" hint="socks5:// 或 http:// 开头">
        <TextBox
          value={draft().proxyUrl}
          placeholder="socks5://127.0.0.1:7890"
          onChange={(value) => props.onPatch((next) => (next.proxyUrl = value))}
        />
      </Field>
      {/* 填完地址就地测一次：这里才是「刚输完」的位置，回列表再测要多跳一步。 */}
      <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
        <Button
          label={probe()?.running ? "检测中…" : "检测这个地址"}
          disabled={probe()?.running ?? false}
          onPress={() => api.checkProxy(draft().proxyUrl)}
        />
        {probe()?.detail ? (
          <Text style={{ fontSize: 10, color: probe()?.ok ? hue.ok() : hue.danger() }}>
            {probe()?.detail}
          </Text>
        ) : null}
      </View>

      <Field label="DNS 模式" hint={mode().hint}>
        <Chips>
          {DNS_MODES.map((item) => (
            <Chip
              label={item.label}
              chosen={draft().dns === item.id}
              onPress={() => props.onPatch((next) => (next.dns = item.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label="远程 DNS"
        hint={virtual() ? "「虚拟 IP」在本地应答，这一项不参与" : "查询经隧道发往这台服务器"}
      >
        <TextBox
          value={draft().dnsAddr}
          placeholder="8.8.8.8"
          disabled={virtual()}
          onChange={(value) => props.onPatch((next) => (next.dnsAddr = value))}
        />
      </Field>

      <Field
        label="虚拟地址池"
        hint={virtual() ? "接管 DNS 时分配给域名的假地址段" : "只有「虚拟 IP」模式会用到"}
      >
        <TextBox
          value={draft().virtualDnsPool}
          placeholder="198.18.0.0/15"
          disabled={!virtual()}
          onChange={(value) => props.onPatch((next) => (next.virtualDnsPool = value))}
        />
      </Field>

      <Field label="网卡名称" hint="留空则每次启动随机生成">
        <TextBox
          value={draft().tunName}
          placeholder="tun0"
          onChange={(value) => props.onPatch((next) => (next.tunName = value))}
        />
      </Field>

      <View style={{ flexDirection: "row", gap: 12 }}>
        <View style={{ flex: 1 }}>
          <Field label="MTU" hint="默认 1500">
            <TextBox
              value={draft().mtu}
              onChange={(value) => props.onPatch((next) => (next.mtu = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field label="最大会话数" hint="并发上限；顶到上限只丢新连接">
            <TextBox
              value={draft().maxSessions}
              onChange={(value) => props.onPatch((next) => (next.maxSessions = value))}
            />
          </Field>
        </View>
      </View>

      <View style={{ flexDirection: "row", gap: 12 }}>
        <View style={{ flex: 1 }}>
          <Field label="TCP 超时" hint="秒；默认 600">
            <TextBox
              value={draft().tcpTimeout}
              onChange={(value) => props.onPatch((next) => (next.tcpTimeout = value))}
            />
          </Field>
        </View>
        <View style={{ flex: 1 }}>
          <Field label="UDP 超时" hint="秒；默认 10">
            <TextBox
              value={draft().udpTimeout}
              onChange={(value) => props.onPatch((next) => (next.udpTimeout = value))}
            />
          </Field>
        </View>
      </View>

      <Field label="日志级别" hint="子进程输出的详细程度">
        <Chips>
          {LEVELS.map((level) => (
            <Chip
              label={level.label}
              chosen={draft().verbosity === level.id}
              onPress={() => props.onPatch((next) => (next.verbosity = level.id))}
            />
          ))}
        </Chips>
      </Field>

      <Field
        label="绕过地址"
        hint="每行一个 IP 或 CIDR；这些地址不进隧道"
        right={
          <Button
            label="填入局域网默认值"
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
