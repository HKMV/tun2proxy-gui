//! 页面共用的那几个小块：卡片、分段胶囊、按钮、状态灯。
//!
//! 和 Rust 版 `ui/widgets.rs` 一一对应——不是为了怀旧，而是两边描述的必须是
//! 同一套观感：卡片是「底色 + 1dp 描边 + 大圆角 + 20dp 内边距」，胶囊是
//! 「高 24、选中的用主色实心」。样式只写在这一处，页面里不重复。

import { effect, Scroll, signal, View, Text } from "nsh";
import { Input } from "nsh/base";
import type { JSX, ScrollRef } from "nsh";
import { hue, ink, radius, surface } from "./theme";

/** 一张卡片。 */
export function Card(props: { children?: JSX.Element; style?: object }): JSX.Element {
  return (
    <View
      style={{
        backgroundColor: surface.card(),
        borderColor: surface.border(),
        borderWidth: 1,
        borderRadius: radius() + 4,
        padding: 20,
        flexDirection: "column",
        gap: 16,
        ...props.style,
      }}
    >
      {props.children}
    </View>
  );
}

/** 卡片头：标题 + 一句说明（放不下就省略）+ 右侧动作。 */
export function CardHead(props: {
  title: string;
  hint?: string;
  right?: JSX.Element;
}): JSX.Element {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
      <View style={{ flex: 1, flexDirection: "column", gap: 4 }}>
        <Text style={{ fontSize: 13, fontWeight: "600", color: ink.card() }}>{props.title}</Text>
        {props.hint ? (
          <Text style={{ fontSize: 11, color: ink.muted() }}>{props.hint}</Text>
        ) : null}
      </View>
      {props.right}
    </View>
  );
}

/** 一段说明文字。 */
export function Caption(props: { children: string; style?: object }): JSX.Element {
  return <Text style={{ fontSize: 11, color: ink.muted(), ...props.style }}>{props.children}</Text>;
}

/** 分段控件的一格。 */
export function Chip(props: {
  label: string;
  chosen: boolean;
  onPress: () => void;
}): JSX.Element {
  return (
    <View
      onPress={props.chosen ? undefined : props.onPress}
      style={{
        height: 24,
        paddingLeft: 12,
        paddingRight: 12,
        borderRadius: 12,
        alignItems: "center",
        justifyContent: "center",
        cursor: props.chosen ? "default" : "pointer",
        backgroundColor: props.chosen ? hue.primary() : "transparent",
        transitionProperty: "background-color",
        transitionDuration: "120ms",
      }}
      hoverStyle={props.chosen ? undefined : { backgroundColor: hue.accent() }}
    >
      <Text
        style={{
          fontSize: 11,
          fontWeight: props.chosen ? "600" : "400",
          color: props.chosen ? hue.primaryInk() : ink.muted(),
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}

/** 分段控件的容器。 */
export function Chips(props: { children?: JSX.Element; style?: object }): JSX.Element {
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 4,
        padding: 4,
        height: 32,
        borderRadius: 16,
        backgroundColor: hue.muted(),
        ...props.style,
      }}
    >
      {props.children}
    </View>
  );
}

/** 按钮的三种样子。 */
export type ButtonVariant = "primary" | "ghost" | "danger";

/** 按钮。禁用时不接点击，也不给悬停反馈——「灰着但能点」最容易让人点半天没反应。 */
export function Button(props: {
  label: string;
  variant?: ButtonVariant;
  disabled?: boolean;
  onPress?: () => void;
  style?: object;
}): JSX.Element {
  const variant = props.variant ?? "ghost";
  // disabled 必须现读：写成常量会冻在挂载那一刻，之后 busy 变 true 也照样能点。
  const disabled = () => props.disabled ?? false;
  // 颜色一律在 style 字面量里现取。先算成对象再展开会被冻在挂载那一刻的主题上：
  // 状态卡那颗「启动」因此一直停在旧配色的青色，换配色、换明暗都不跟着走。
  return (
    <View
      onPress={disabled() ? undefined : props.onPress}
      style={{
        height: 28,
        paddingLeft: 12,
        paddingRight: 12,
        borderRadius: 14,
        alignItems: "center",
        justifyContent: "center",
        borderWidth: 1,
        borderColor:
          variant === "primary" ? "transparent" : variant === "danger" ? hue.danger() : surface.border(),
        // 禁用态同样不能用 hue.muted()：列表行本身就是这个色，按钮的胶囊
        // 形状会整块消失，只剩一行灰字，看着像排版没对齐。
        backgroundColor: disabled() ? surface.border() : variant === "primary" ? hue.primary() : "transparent",
        cursor: disabled() ? "default" : "pointer",
        opacity: disabled() ? 0.5 : 1,
        transitionProperty: "background-color",
        transitionDuration: "120ms",
        ...props.style,
      }}
      hoverStyle={disabled() || variant !== "ghost" ? undefined : { backgroundColor: hue.accent() }}
    >
      <Text
        style={{
          fontSize: 11,
          fontWeight: variant === "primary" ? "600" : "400",
          color: disabled()
            ? ink.muted()
            : variant === "primary"
              ? hue.primaryInk()
              : variant === "danger"
                ? hue.danger()
                : ink.muted(),
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}

/** 状态灯。`pulse` 时呼吸（nsh 的内建动画，不用自己起定时器）。 */
export function Dot(props: { color: string; size?: number; pulse?: boolean }): JSX.Element {
  const size = props.size ?? 8;
  return (
    <View
      style={{
        width: size,
        height: size,
        borderRadius: size / 2,
        backgroundColor: props.color,
        animationName: props.pulse ? "pulse" : "none",
        animationDuration: "1600ms",
        animationIterationCount: "infinite",
      }}
    />
  );
}

/** 一组「标题 + 大数字」的读数。 */
export function Metric(props: { label: string; value: string; color: string }): JSX.Element {
  return (
    <View style={{ flexDirection: "column", gap: 2, alignItems: "flex-end" }}>
      <Text style={{ fontSize: 10, color: ink.muted() }}>{props.label}</Text>
      <Text
        style={{
          fontSize: 13,
          fontWeight: "600",
          color: props.color,
          fontVariant: ["tabular-nums"],
        }}
      >
        {props.value}
      </Text>
    </View>
  );
}

/** 表单里的一行：标题 + 一句说明（右侧可以挂个动作）+ 控件。 */
export function Field(props: {
  label: string;
  hint?: string;
  right?: JSX.Element;
  children: JSX.Element;
}): JSX.Element {
  return (
    <View style={{ flexDirection: "column", gap: 8 }}>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
        <Text style={{ fontSize: 11, fontWeight: "600", color: ink.primary() }}>{props.label}</Text>
        {props.hint ? (
          <Text style={{ fontSize: 10, color: ink.muted(), flex: 1 }}>{props.hint}</Text>
        ) : (
          <View style={{ flex: 1 }} />
        )}
        {props.right}
      </View>
      {props.children}
    </View>
  );
}

/** 文本输入。`disabled` 时只读且变淡——「能点但没用」最容易让人白填一遍。 */
export function TextBox(props: {
  value: string;
  placeholder?: string;
  multiline?: boolean;
  height?: number;
  disabled?: boolean;
  onChange: (value: string) => void;
}): JSX.Element {
  return (
    <Input
      value={props.value}
      placeholder={props.placeholder}
      multiline={props.multiline}
      readonly={props.disabled}
      onChange={(event) => props.onChange(event.value)}
      style={{
        fontSize: 12,
        color: props.disabled ? ink.muted() : ink.primary(),
        backgroundColor: surface.background(),
        borderWidth: 1,
        borderColor: surface.border(),
        borderRadius: 8,
        paddingLeft: 10,
        paddingRight: 10,
        paddingTop: 8,
        paddingBottom: 8,
        height: props.height ?? (props.multiline ? 120 : 34),
      }}
      focusStyle={{ borderColor: hue.primary() }}
    />
  );
}

/** 一行设置：左边标题与说明、右边控件（开关、胶囊、按钮）。 */
export function Setting(props: {
  label: string;
  hint?: string;
  children: JSX.Element;
}): JSX.Element {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
      <View style={{ flex: 1, flexDirection: "column", gap: 3 }}>
        <Text style={{ fontSize: 12, fontWeight: "600", color: ink.primary() }}>{props.label}</Text>
        {props.hint ? (
          <Text style={{ fontSize: 10, color: ink.muted() }}>{props.hint}</Text>
        ) : null}
      </View>
      {props.children}
    </View>
  );
}

/**
 * 开关。
 *
 * 自己画而不用现成组件：它出现在十几行规则的右侧，「40×22 的轨道 + 16 的滑块」
 * 这个尺寸和一页里其他胶囊（高 24）能对齐，现成组件的高度不受控。
 */
export function Toggle(props: {
  on: boolean;
  disabled?: boolean;
  onToggle: () => void;
}): JSX.Element {
  const disabled = () => props.disabled ?? false;
  return (
    <View
      onPress={disabled() ? undefined : props.onToggle}
      style={{
        width: 40,
        height: 22,
        borderRadius: 11,
        padding: 3,
        flexDirection: "row",
        alignItems: "center",
        justifyContent: props.on ? "flex-end" : "flex-start",
        // 关态轨道不能用 hue.muted()：那个色同时是列表行的底色，放在行里
        // 轨道就和底色并成一片，只剩一个圆点，看不出这是个开关。
        backgroundColor: props.on ? hue.primary() : surface.border(),
        cursor: disabled() ? "default" : "pointer",
        opacity: disabled() ? 0.4 : 1,
        transitionProperty: "background-color",
        transitionDuration: "140ms",
      }}
    >
      <View
        style={{
          width: 16,
          height: 16,
          borderRadius: 8,
          backgroundColor: props.on ? hue.primaryInk() : ink.muted(),
        }}
      />
    </View>
  );
}

/** 一句结论（「已保存」「MTU 要在 576 到 9000 之间」）。 */
export function Note(props: { tone: "ok" | "bad"; text: string }): JSX.Element {
  return (
    <Text style={{ fontSize: 11, color: props.tone === "ok" ? hue.ok() : hue.danger() }}>
      {props.text}
    </Text>
  );
}

/**
 * 可滚动区域：内容 + 右侧一条常驻的细指示条。
 *
 * 指示条是自己画的，不用 nsh 内置的那条：`Scroll` 的 `scrollbar` 在 Windows 后端
 * 什么都不画（`Scrollbar.setTheme` 设了也没出现，实测截图里一条都没有），而
 * 「能滚但看不出有东西可滚」正是之前被点名的毛病。自己画只依赖 `onScroll` 与
 * `onLayout` 给的两个数，任何后端都一样。
 */
export function ScrollArea(props: {
  style?: object;
  /** 这个信号一变就滚回顶部（换页时用）。 */
  resetOn?: () => unknown;
  /** 这个信号一变就贴到底部（日志那种「最新在下面」的内容）。 */
  stickToBottom?: () => unknown;
  children?: JSX.Element;
}): JSX.Element {
  let node: ScrollRef | null = null;
  const viewport = signal(0);
  const content = signal(0);
  const offset = signal(0);

  // 首次不滚：挂载时本来就该在顶部，白滚一次会让第一帧抖一下。
  let first = true;
  if (props.resetOn) {
    const resetOn = props.resetOn;
    effect(() => {
      resetOn();
      if (first) {
        first = false;
        return;
      }
      // 等新内容摆好再滚，否则那次调用落在旧布局上（等于没滚）。
      setTimeout(() => node?.scrollTo({ y: 0 }), 0);
    });
  }

  /**
   * 「贴着底」这件事靠**内容变高的那一刻**判断，不靠每秒一次的快照。
   *
   * `atBottom` 由 `onScroll` 维护：用户滚上去了就是 false，滚到底（或程序刚把
   * 它滚到底）就是 true。日志多出来一行时，只有本来就贴着底才继续跟随——
   * 这样翻旧日志不会被打断，也不会像之前那样因为「内容长了、偏移没跟上」而
   * 误判成"用户在翻旧的"。
   */
  let atBottom = true;
  const stickToEnd = () => node?.scrollTo({ y: Math.max(0, content() - viewport()) });

  /**
   * 滚轮自己接。
   *
   * 嵌在页面滚动区里的内层 `Scroll` 拿不到滚轮（实测：日志区里滚动，动的是外层
   * 页面），而 `onWheel` 是「最近的那个监听者听到」，所以这里接管：按视口当前
   * 偏移自己算目标位置。
   */
  const wheel = (event: { delta: { y: number }; deltaMode: "points" | "lines" }) => {
    // 鼠标档位一格 48 点，触摸板按点原样算。
    const delta = event.delta.y * (event.deltaMode === "lines" ? 48 : 1);
    const limit = Math.max(0, content() - viewport());
    node?.scrollTo({ y: Math.min(limit, Math.max(0, offset() + delta)) });
  };

  const overflow = () => content() - viewport();
  const thumbHeight = () => Math.max(32, (viewport() / Math.max(1, content())) * viewport());
  const thumbTop = () =>
    overflow() > 0 ? (offset() / overflow()) * Math.max(0, viewport() - thumbHeight()) : 0;

  return (
    <View style={{ flex: 1, ...props.style }}>
      <Scroll
        ref={(next: ScrollRef) => (node = next)}
        style={{ flex: 1 }}
        scrollEventThrottle={16}
        onWheel={wheel}
        onScroll={(event) => {
          viewport.set(event.size.height);
          content.set(event.contentSize.height);
          offset.set(event.offset.y);
          // 8 点容差：贴到底时浮点数常常差一两个点，不给余量会一直判成「没到底」。
          atBottom = event.contentSize.height - event.size.height - event.offset.y <= 8;
        }}
        onLayout={(event) => viewport.set(event.bounds.size.height)}
      >
        <View
          onLayout={(event) => {
            const height = event.bounds.size.height;
            const grew = height > content();
            content.set(height);
            // 内容变高了、而且用户本来就贴着底：跟着到底（首次挂载也走这条）。
            if (grew && props.stickToBottom && atBottom) setTimeout(stickToEnd, 0);
          }}
          style={{ flexDirection: "column" }}
        >
          {props.children}
        </View>
      </Scroll>
      {overflow() > 1 ? (
        <View style={{ position: "absolute", right: 2, top: 4, bottom: 4, width: 4 }}>
          <View
            style={{
              height: thumbHeight(),
              marginTop: thumbTop(),
              borderRadius: 2,
              backgroundColor: ink.muted(),
              opacity: 0.45,
            }}
          />
        </View>
      ) : null}
    </View>
  );
}

