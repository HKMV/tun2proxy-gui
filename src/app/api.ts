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

/** 分流器的运行情况。 */
export interface SplitInfo {
  running: boolean;
  rules: number;
  direct: number;
  proxy: number;
  failed: number;
  sessions: number;
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
  split: SplitInfo;
  update: UpdateInfo;
}

/** 动作的结论：`ok` 为假时 `error` 是可以直接显示的原因。 */
export interface Outcome {
  ok: boolean;
  error?: string;
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

/** 某套配色的 JSON 原文（换配色时用）。 */
export function paletteJson(palette: string): string {
  return Backend.paletteJson(palette);
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
