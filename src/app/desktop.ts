//! 桌面外壳：托盘图标、关窗拦截、把窗口收进托盘。
//!
//! 这几件事**不是界面**（没有布局、没有配色），但也不是业务（内核不知道窗口
//! 存在）。所以单独一格：只依赖 `nsh/desktop` 和 `api`，页面一行都不用管。
//!
//! # 为什么要拦关闭按钮
//!
//! tun2proxy 是常驻服务：窗口一关，隧道和它建的 TUN 网卡一起没了，用户的网
//! 就断了。所以关窗按配置走三档（`ask` / `minimize` / `quit`），而不是系统默认。

import { effect } from "nsh";
import { Application, currentWindow, Tray } from "nsh/desktop";
import type { WindowRef } from "nsh/desktop";

import type * as api from "./api";
import { isEnglish, t } from "./i18n";

/**
 * 窗口句柄。
 *
 * `currentWindow()` 只能在**渲染期间**调用（它要读「当前组件画在哪个窗口」），
 * 事件回调里再调会直接报错。所以渲染时取一次存这儿，之后所有回调都用它。
 */
let handle: WindowRef | null = null;

/** 在根组件里调一次，把窗口句柄记下来。 */
export function bindWindow(): void {
  if (!handle) handle = currentWindow();
}

/** 把窗口收进托盘（关窗、或「最小化到托盘」按钮走这里）。 */
export function hideToTray(): void {
  handle?.hide();
}

/** 把窗口叫回前台（托盘点一下、或菜单里的「显示窗口」）。 */
export function showWindow(): void {
  handle?.show();
  handle?.focus();
}

/** 真正退出程序。 */
export function quit(): void {
  Application.quit();
}

/**
 * 装托盘图标：一句状态 + 三个菜单项，点图标把窗口叫回来。
 *
 * 只在状态（跑没跑、状态文字）变了的时候重设一次：快照每秒一份，每份都
 * `Tray.set` 等于每秒重建一次原生菜单。
 */
export function installTray(options: {
  snap: () => api.Snapshot;
  onToggle: () => void;
}): void {
  let last = "";
  effect(() => {
    const snap = options.snap();
    // 语言也算进 key：菜单是原生对象，不重设就不会跟着界面换语言
    // （`statusLabel` 通常也变了，但不能指望它一定变）。
    const key = `${snap.active}|${snap.statusLabel}|${isEnglish()}`;
    if (key === last) return;
    last = key;
    Tray.set({
      tooltip: `Tun2Proxy GUI · ${snap.statusLabel}`,
      menu: [
        {
          label: snap.active ? t("停止隧道", "Stop tunnel") : t("启动隧道", "Start tunnel"),
          onSelect: options.onToggle,
        },
        { type: "separator" },
        { label: t("显示窗口", "Show window"), onSelect: showWindow },
        { label: t("退出", "Quit"), onSelect: quit },
      ],
      onClick: showWindow,
    });
  });
}

/**
 * 拦截关闭按钮，按配置的音档处理。
 *
 * `ask` 不在这里弹窗：外壳只负责把「用户想关窗」这件事报给界面，弹什么、有
 * 哪些选项是界面的事（[`onAsk`]）。
 */
export function interceptClose(options: {
  /** 当前配置的关窗行为：`ask` / `minimize` / `quit`。 */
  action: () => string;
  /** `ask` 时由界面弹出确认。 */
  onAsk: () => void;
}): void {
  const win = handle ?? currentWindow();
  handle = win;
  // 关窗交给这里判断：设为 false 之后系统不再自己关，必须由我们决定。
  win.set({ closeOnPress: false });
  win.on("closePress", () => {
    const action = options.action();
    if (action === "quit") {
      quit();
      return;
    }
    if (action === "minimize") {
      hideToTray();
      return;
    }
    options.onAsk();
  });
}
