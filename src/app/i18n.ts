//! 界面文案：中文 / 英文。
//!
//! 和 Rust 那边 `core/i18n.rs` 是同一套做法——**没有翻译文件**，每条文案就地写成
//! `t("中文", "English")`。好处是改文案不用在两处对齐、删一条文案不会留下孤儿 key；
//! 代价是文案散在页面里，找的话直接搜中文。
//!
//! 语言本身来自内核（`api.language()`，`auto` 已经被解析成具体语言），这里只留一个
//! 信号存它。为什么要过信号：nsh 的组件体只跑一次，跟着语言变的值必须在**子节点位置**
//! 求值才会跟着重算——`t()` 每次被求值都读一次这个信号，正好是这个形状。
//!
//! 别把 `t()` 的结果塞进 `style` 字面对象或者模块顶层的 `const`：那两处只求值一次，
//! 换语言不会变。

import { signal } from "nsh";

/** 当前语言：`zh` / `en`。 */
const language = signal<string>("zh");

/** 内核那边报来的语言变了就同步过来（`App` 每秒刷一次，换语言时立刻刷一次）。 */
export function syncLanguage(code: string): void {
  if (code !== language()) language.set(code);
}

/** 当前是不是英文界面。 */
export function isEnglish(): boolean {
  return language() === "en";
}

/**
 * 一条文案的中英两份。
 *
 * 英文那边不用刻意写全称——界面窄的时候长句会被挤成两行，宁可短一点
 * （`Start` 而不是 `Start the tunnel`）。
 */
export function t(zh: string, en: string): string {
  return language() === "en" ? en : zh;
}
