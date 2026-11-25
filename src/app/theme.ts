//! 这套界面的「颜色语义」。
//!
//! 页面里不写死十六进制，也不直接读 `token("chart-3")` 这种编号——编号是主题
//! 文件的实现细节，换一套配色时编号的含义会变。这里给每个语义一个名字，改配色
//! 只改 `themes/*.json`，`.tsx` 不动。
//!
//! # 为什么每个取色函数都要读一下 `themeTick`
//!
//! `token()` 在**读的那一刻**取值：`style={{ color: ink.primary() }}` 求值时就已经
//! 是字符串了。换配色只换了主题表，已经求过值的 `style` 不会跟着变，除非那个
//! 组件重新求值。所以取色函数里先读一次 [`themeTick`]——凡是画颜色的组件都因此
//! 订阅了「主题变了」，换配色/明暗时自动重画。界面其他部分不受影响。

import { signal, token } from "nsh";

/**
 * 「主题变了」的计数器：换配色或明暗时 +1。
 *
 * 用私有名字导出给同目录的判断用；页面里只在 [`bumpTheme`] 里写它。
 */
const themeTick = signal(0);

/** 告诉所有画颜色的组件：主题变了，重新取色。 */
export function bumpTheme(): void {
  themeTick.update((tick) => tick + 1);
}

/** 取一个 token，并订阅主题变化。 */
function color(name: Parameters<typeof token>[0]): string {
  themeTick();
  return token(name) as string;
}

/** 底色、卡片、描边这些「骨架色」。 */
export const surface = {
  background: () => color("background"),
  card: () => color("card"),
  border: () => color("border"),
  sidebar: () => color("sidebar"),
  sidebarBorder: () => color("sidebar-border"),
};

/** 文字的三档。 */
export const ink = {
  /** 正文 / 标题。 */
  primary: () => color("foreground"),
  /** 卡片标题。 */
  card: () => color("card-foreground"),
  /** 说明、单位、次要信息。 */
  muted: () => color("muted-foreground"),
};

/** 状态与两路流量的颜色。 */
export const hue = {
  /** 直连（青）。 */
  direct: () => color("chart-1"),
  /** 走代理（紫）。 */
  proxy: () => color("chart-2"),
  /** 一切正常。 */
  ok: () => color("chart-3"),
  /** 需要注意（未启动、会话打满）。 */
  warn: () => color("chart-4"),
  /** 失败。 */
  danger: () => color("destructive"),
  /** 主色（选中的胶囊、主按钮）。 */
  primary: () => color("primary"),
  primaryInk: () => color("primary-foreground"),
  /** 悬停时的浅底。 */
  accent: () => color("accent"),
  /** 分段控件、禁用态的那种浅底。 */
  muted: () => color("muted"),
};

/** 圆角半径（主题里的 `radius`）。 */
export function radius(): number {
  themeTick();
  return token("radius") as number;
}

/** 字节/秒 → `12.3 MB/s`。 */
export function rateText(bytes: number): string {
  return `${bytesText(bytes)}/s`;
}

/** 字节数 → `12.3 MB`。 */
export function bytesText(bytes: number): string {
  const KB = 1024;
  const MB = KB * 1024;
  const GB = MB * 1024;
  if (bytes < KB) return `${bytes} B`;
  if (bytes < MB) return `${(bytes / KB).toFixed(1)} KB`;
  if (bytes < GB) return `${(bytes / MB).toFixed(1)} MB`;
  return `${(bytes / GB).toFixed(2)} GB`;
}
