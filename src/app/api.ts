//! 后端接口的唯一入口。
//!
//! 界面这一侧只认这个文件：它把 Rust 那边 `Backend` 模块的 JSON 文本解析成
//! 类型化的对象，也把「要成功还是要错误」的约定收在一处。页面里不出现
//! `JSON.parse`，也不直接 import `tun2proxy/native`——将来后端换形状、加字段，
//! 改这里一个文件就够。
//!
//! 类型是手写的，和 `src/api/snapshot.rs` 一一对应；字段改名时两边一起改。

import { Backend } from "tun2proxy/native";

/** 一个方向的瞬时速率（字节/秒）。 */
export interface Rates {
  up: number;
  down: number;
}

/** 流量历史里的一个点（每秒一个）。 */
export interface HistoryPoint {
  direct: Rates;
  proxy: Rates;
}

/** 日志列表里的一行。 */
export interface LogEntry {
  kind: "output" | "app" | "split" | "other";
  text: string;
}

/** 分流器的一次判定。 */
export interface DecisionView {
  route: "direct" | "proxy";
  /** 判定这么走，但没连上。 */
  ok: boolean;
  /** 目标，形如 `www.qq.com:443`。 */
  target: string;
  /** 直连时是对端地址，走代理时是命中原因，失败时是失败原因。 */
  detail: string;
}

/** 分流器的运行情况。 */
export interface SplitInfo {
  running: boolean;
  /** 监听地址；没在跑时是 `null`。 */
  addr: string | null;
  rules: number;
  direct: number;
  proxy: number;
  failed: number;
  sessions: number;
  /** 最近的判定，新的在前。 */
  decisions: DecisionView[];
}

/** 内置清单的更新进度。 */
export interface SplitUpdateInfo {
  busy: boolean;
  text: string | null;
  failed: boolean;
}

/** 版本检查 / 下载安装的进度。 */
export interface UpdateInfo {
  busy: boolean;
  progress: number | null;
  downloaded: number;
  text: string | null;
}

/** 服务状态。 */
export type Status = "stopped" | "starting" | "running" | "stopping" | "failed";

/** 一帧要用的全部实时数据。 */
export interface Snapshot {
  status: Status;
  statusLabel: string;
  detail: string | null;
  active: boolean;
  busy: boolean;
  busyLabel: string;
  installed: boolean;
  version: string | null;
  blocked: string | null;
  sessionsFull: boolean;
  direct: Rates;
  proxy: Rates;
  history: HistoryPoint[];
  logs: LogEntry[];
  logTotal: number;
  /** 代理地址的检测结论，键是地址去掉空白 + 小写。 */
  probes: Record<string, Probe>;
  split: SplitInfo;
  /** 内置清单更新的进度。 */
  splitUpdate: SplitUpdateInfo;
  update: UpdateInfo;
}

/** 动作的结论：`ok` 为假时 `error` 是可以直接显示的原因。 */
export interface Outcome {
  ok: boolean;
  error?: string;
}

/** 一个代理地址的检测状态。 */
export interface Probe {
  /** 还在测。 */
  running: boolean;
  /** 通过了没有；没结论时是 `null`。 */
  ok: boolean | null;
  /** 一句话结论。 */
  detail: string | null;
}

/** 一套配置：字段名就是配置文件里的键（下划线），改一个再传回去不用翻译。 */
export interface Profile {
  name: string;
  proxy_url: string;
  dns: string;
  dns_addr: string;
  virtual_dns_pool: string;
  bypass: string;
  tun_name: string;
  mtu: number;
  tcp_timeout: number;
  udp_timeout: number;
  max_sessions: number;
  verbosity: string;
  udpgw_server: string;
  udpgw_connections: number;
  udpgw_keepalive: number;
  ipv6: boolean;
  setup: boolean;
  unshare: boolean;
}

/** 列表里的一套配置：`Profile` 的全部字段 + 两句现成的显示文本。 */
export interface ProfileItem extends Profile {
  /** 列表主标题（名字为空时是「未命名配置」）。 */
  displayName: string;
  /** 列表副标题：地址 · DNS 模式。 */
  summary: string;
}

/** 「代理配置」页的全部数据。 */
export interface ProfilesView {
  /** 配置文件路径。 */
  path: string;
  /** 当前生效那套的下标。 */
  active: number;
  items: ProfileItem[];
}

/** 表单草稿：数字字段是字符串（还没解析）。 */
export interface Draft {
  name: string;
  proxyUrl: string;
  dns: string;
  dnsAddr: string;
  virtualDnsPool: string;
  tunName: string;
  mtu: string;
  tcpTimeout: string;
  udpTimeout: string;
  maxSessions: string;
  bypass: string;
  verbosity: string;
}

/** 启动时要的那一份（[`boot`]）。 */
export interface Boot {
  /** 当前配色：`cyber` / `matrix` / `ember` / `aurora`。 */
  palette: string;
  /** 明暗设置：`auto` / `light` / `dark`。 */
  theme: string;
  /** 界面语言设置：`auto` / `zh` / `en`。 */
  language: string;
  /** 点关闭按钮怎么办：`ask` / `minimize` / `quit`。 */
  closeAction: string;
  /** 当前配色的 JSON 原文，`JSON.parse` 后交给 nsh 的 `setTheme()`。 */
  themeJson: string;
  /** 程序版本。 */
  version: string;
}

/** 界面启动时要的那一份：配色、明暗、语言、关窗行为、版本。 */
export function boot(): Boot {
  return JSON.parse(Backend.boot()) as Boot;
}

/** 某套配置的 JSON 原文（换配色时用）。 */
export function paletteJson(palette: string): string {
  return Backend.paletteJson(palette);
}

// —— 代理配置页 ——

/** 全部配置 + 当前是哪套 + 配置文件路径。 */
export function profiles(): ProfilesView {
  return JSON.parse(Backend.profiles()) as ProfilesView;
}

/** 某套配置的表单草稿（进编辑态时取）。 */
export function profileDraft(index: number): Draft | null {
  return JSON.parse(Backend.profileDraft(index)) as Draft | null;
}

/** 切到某套配置。 */
export function selectProfile(index: number): Outcome {
  return JSON.parse(Backend.selectProfile(index)) as Outcome;
}

/** 复制当前这套，返回新那套的下标。 */
export function duplicateProfile(): { ok: boolean; index: number | null } {
  return JSON.parse(Backend.duplicateProfile()) as { ok: boolean; index: number | null };
}

/** 新建一套，返回它的下标。 */
export function addProfile(): { ok: boolean; index: number | null } {
  return JSON.parse(Backend.addProfile()) as { ok: boolean; index: number | null };
}

/** 删掉一套（只有一套时后端会拒绝）。 */
export function removeProfile(index: number): Outcome {
  return JSON.parse(Backend.removeProfile(index)) as Outcome;
}

/** 保存一套配置；失败时 `error` 是能直接显示的原因。 */
export function saveProfile(index: number, draft: Draft): Outcome {
  return JSON.parse(Backend.saveProfile(index, JSON.stringify(draft))) as Outcome;
}

/** 检测一个代理地址（异步，结论从快照的 `probes` 里读）。 */
export function checkProxy(url: string): Outcome {
  return JSON.parse(Backend.checkProxy(url)) as Outcome;
}

/** 表单里「一键填默认」的值。 */
export function defaults(): { bypass: string } {
  return JSON.parse(Backend.defaults()) as { bypass: string };
}

/** 这一刻的全部实时数据。 */
export function snapshot(): Snapshot {
  return JSON.parse(Backend.snapshot()) as Snapshot;
}

/** 启动隧道。失败时 `error` 是本地校验的结论（「还没下载 tun2proxy」这种）。 */
export function start(): Outcome {
  return JSON.parse(Backend.start()) as Outcome;
}

/** 停止隧道（不等结果，状态从下一帧快照里读）。 */
export function stop(): Outcome {
  return JSON.parse(Backend.stop()) as Outcome;
}

/** 换「点关闭按钮怎么办」。 */
export function setCloseAction(action: string): Outcome {
  return JSON.parse(Backend.setCloseAction(action)) as Outcome;
}

/** 换界面语言；`changed` 为真表示真的变了（要整棵重建）。 */
export function setLanguage(language: string): { changed: boolean } {
  return JSON.parse(Backend.setLanguage(language)) as { changed: boolean };
}

/** 换配色。 */
export function setPalette(palette: string): Outcome {
  return JSON.parse(Backend.setPalette(palette)) as Outcome;
}

/** 换明暗。 */
export function setTheme(theme: string): Outcome {
  return JSON.parse(Backend.setTheme(theme)) as Outcome;
}

// —— 分流规则页 ——

/** 一组内置清单。 */
export interface GroupView {
  /** `local` / `chinaIp` / `chinaDomain`。 */
  id: string;
  label: string;
  hint: string;
  count: number;
  enabled: boolean;
}

/** 「分流规则」页的全部设置。 */
export interface SplitView {
  enabled: boolean;
  groups: GroupView[];
  custom: string;
  directDns: string;
  directInterface: string;
  /** 当前配置的 DNS 策略：`virtual` / `direct` / `tcp`。 */
  dnsMode: string;
  dnsModeLabel: string;
  defaultDirectDns: string;
}

/** 规则明细里的一行。 */
export interface RuleView {
  /** 行开关的键（`suffix:qq.com`）。 */
  key: string;
  text: string;
  kind: string;
  source: string;
  enabled: boolean;
}

/** 分流设置：总开关、三组清单、自定义规则、直连出口。 */
export function splitView(): SplitView {
  return JSON.parse(Backend.splitView()) as SplitView;
}

/** 规则明细（`filter` 为空表示不过滤，`limit` 截断条数）。 */
export function splitRules(filter: string, limit: number): RuleView[] {
  return JSON.parse(Backend.splitRules(filter, limit)) as RuleView[];
}

/** 开关总闸，返回改后的值。 */
export function splitToggleEnabled(): { enabled: boolean } {
  return JSON.parse(Backend.splitToggleEnabled()) as { enabled: boolean };
}

/** 开关一组内置清单，返回改后的值。 */
export function splitSetGroup(group: string, enabled: boolean): { enabled: boolean } {
  return JSON.parse(Backend.splitSetGroup(group, enabled)) as { enabled: boolean };
}

/** 翻转一条规则，返回改后的状态。 */
export function splitToggleRow(key: string): { enabled: boolean } {
  return JSON.parse(Backend.splitToggleRow(key)) as { enabled: boolean };
}

/** 保存自定义规则。 */
export function splitSaveCustom(custom: string): Outcome {
  return JSON.parse(Backend.splitSaveCustom(custom)) as Outcome;
}

/** 保存直连出口。 */
export function splitSaveOutlet(directDns: string, directInterface: string): Outcome {
  return JSON.parse(Backend.splitSaveOutlet(directDns, directInterface)) as Outcome;
}

/** 从 APNIC / v2fly 重新拉清单（进度看快照的 `splitUpdate`）。 */
export function splitUpdateLists(): Outcome {
  return JSON.parse(Backend.splitUpdateLists()) as Outcome;
}

// —— 网络与绕过页 ——

/** 「网络与绕过」页的全部数据（改的是当前那套配置）。 */
export interface NetworkView {
  profile: string;
  ipv6: boolean;
  setup: boolean;
  unshare: boolean;
  udpgwServer: string;
  udpgwConnections: string;
  udpgwKeepalive: string;
  bypass: string;
  defaultBypass: string;
}

/** 当前那套配置的网络参数。 */
export function networkView(): NetworkView {
  return JSON.parse(Backend.networkView()) as NetworkView;
}

/** 开关 IPv6 / 系统路由 / 卸载模式，返回改后的值。 */
export function networkSetFlag(flag: "ipv6" | "setup" | "unshare", value: boolean): { value: boolean } {
  return JSON.parse(Backend.networkSetFlag(flag, value)) as { value: boolean };
}

/** 保存 UDP 转发（连接数与保活间隔是表单里的字符串）。 */
export function networkSaveUdpgw(
  server: string,
  connections: string,
  keepalive: string,
): Outcome {
  return JSON.parse(Backend.networkSaveUdpgw(server, connections, keepalive)) as Outcome;
}

/** 保存绕过清单。 */
export function networkSaveBypass(bypass: string): Outcome {
  return JSON.parse(Backend.networkSaveBypass(bypass)) as Outcome;
}

// —— 程序与日志页 ——

/** 一个「三选一」的选项。 */
export interface OptionView {
  id: string;
  label: string;
}

/** 「程序与日志」页除日志以外的数据。 */
export interface ProgramView {
  version: string;
  closeAction: string;
  closeOptions: OptionView[];
  language: string;
  languageOptions: OptionView[];
  configPath: string;
  dataDir: string;
  binDir: string;
  logsDir: string;
}

/** 程序本身的信息：版本、关窗行为、语言、四个路径。 */
export function programView(): ProgramView {
  return JSON.parse(Backend.programView()) as ProgramView;
}

/** 清空内存里的日志（磁盘文件保留）。 */
export function clearLogs(): Outcome {
  return JSON.parse(Backend.clearLogs()) as Outcome;
}

/** 检查内核有没有新版本（结论看快照的 `update`）。 */
export function checkUpdate(): Outcome {
  return JSON.parse(Backend.checkUpdate()) as Outcome;
}

/** 下载并安装内核（进度看快照的 `update`）。 */
export function installUpdate(): Outcome {
  return JSON.parse(Backend.installUpdate()) as Outcome;
}
