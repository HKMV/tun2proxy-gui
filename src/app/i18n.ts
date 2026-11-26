//! 界面文案：中文 / 英文。
//!
//! 文案全部放在 `locales/` 下的两份 JSON 里，键名是 `<区段>.<名字>`：
//!
//! ```text
//! locales/zh.json   中文，**键的准绳**：key 只在这里定义
//! locales/en.json   英文，类型上要求键集合与 zh.json 完全一致
//! ```
//!
//! 英文少一条键，`MESSAGES` 那个类型标注当场报错，不用等界面上看见中文才知道漏了。
//! 带数字/名字的句子用 `{name}` 占位，调用时传参数——别在代码里拼字符串，否则语序
//! 一变就没法翻（英文的 "3 entries" 和中文的 "3 条" 位置刚好反过来）。
//!
//! 语言本身来自内核（`api.language()`，`auto` 已经被解析成具体语言），这里只留一个
//! 信号存它。为什么要过信号：nsh 的组件体只跑一次，跟着语言变的值必须在**子节点位置**
//! 求值才会跟着重算——`t()` 每次被求值都读一次这个信号，正好是这个形状。
//!
//! 别把 `t()` 的结果塞进 `style` 字面对象或者模块顶层的 `const`：那两处只求值一次，
//! 换语言不会变。

import { signal } from "nsh";

import EN from "./locales/en.json";
import ZH from "./locales/zh.json";

/** 一条文案的键。只在 `zh.json` 里定义，别手写字符串。 */
export type MessageKey = keyof typeof ZH;

/** 占位符的值：`{count}` / `{path}` 这些。 */
export type MessageParams = Record<string, string | number>;

/**
 * 两份词表。
 *
 * 这里的类型标注就是那个编译期检查：`en.json` 少了哪个键、多出哪个键，都在这一行报。
 */
const MESSAGES: Record<"zh" | "en", Record<MessageKey, string>> = { zh: ZH, en: EN };

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
 * 取一条文案。
 *
 * 英文那边不用刻意写全称——界面窄的时候长句会被挤成两行，宁可短一点
 * （`Start` 而不是 `Start the tunnel`）。
 */
export function t(key: MessageKey, params?: MessageParams): string {
  const text = MESSAGES[isEnglish() ? "en" : "zh"][key];
  if (!params) return text;
  return text.replace(/\{(\w+)\}/g, (whole, name: string) =>
    name in params ? String(params[name]) : whole,
  );
}
