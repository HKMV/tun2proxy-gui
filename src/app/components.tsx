//! 页面共用的那几个小块：卡片、分段胶囊、按钮、状态灯。
//!
//! 和 Rust 版 `ui/widgets.rs` 一一对应——不是为了怀旧，而是两边描述的必须是
//! 同一套观感：卡片是「底色 + 1dp 描边 + 大圆角 + 20dp 内边距」，胶囊是
//! 「高 24、选中的用主色实心」。样式只写在这一处，页面里不重复。

import { View, Text } from "nsh";
import type { JSX } from "nsh";
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
  const disabled = props.disabled ?? false;
  const palette = {
    primary: { bg: hue.primary(), ink: hue.primaryInk(), border: "transparent" },
    ghost: { bg: "transparent", ink: ink.muted(), border: surface.border() },
    danger: { bg: "transparent", ink: hue.danger(), border: hue.danger() },
  }[variant];
  return (
    <View
      onPress={disabled ? undefined : props.onPress}
      style={{
        height: 28,
        paddingLeft: 12,
        paddingRight: 12,
        borderRadius: 14,
        alignItems: "center",
        justifyContent: "center",
        borderWidth: 1,
        borderColor: palette.border,
        backgroundColor: disabled ? hue.muted() : palette.bg,
        cursor: disabled ? "default" : "pointer",
        opacity: disabled ? 0.5 : 1,
        transitionProperty: "background-color",
        transitionDuration: "120ms",
        ...props.style,
      }}
      hoverStyle={disabled || variant !== "ghost" ? undefined : { backgroundColor: hue.accent() }}
    >
      <Text
        style={{
          fontSize: 11,
          fontWeight: variant === "primary" ? "600" : "400",
          color: disabled ? ink.muted() : palette.ink,
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
