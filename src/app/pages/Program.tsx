//! 程序与日志页：内核、窗口与界面、关于、运行日志。
//!
//! 这一页是「程序本身」而不是「隧道」：内核装没装、要不要更新、关窗怎么办、
//! 配置文件与日志落在哪个目录，以及正在产生的日志行。
//!
//! # 日志只画最后一段
//!
//! 内核留 2000 行、快照给 400 行。全画出来既没人看也会让每秒一帧的重建变慢，
//! 所以这里只画过滤后的最后 [`LOG_ROWS`] 行，并在标题上把「一共多少行」写清楚
//! ——完整日志在磁盘文件里，路径就在上面那张卡的「日志」一行。

import { Clipboard, signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "../api";
import { Button, Card, CardHead, Caption, Chip, Chips, ScrollArea, Setting } from "../components";
import { hideToTray } from "../desktop";
import { syncLanguage, t } from "../i18n";
import type { MessageKey } from "../i18n";
import { hue, ink, surface } from "../theme";

/** 日志区画多少行。 */
const LOG_ROWS = 200;

/** 日志过滤的四档（和后端的 `LogFilter` 一一对应）。 */
const FILTERS = [
  { id: "all", label: "program.filter.all" },
  { id: "output", label: "program.filter.output" },
  { id: "app", label: "program.filter.app" },
  { id: "split", label: "program.filter.split" },
] as const satisfies readonly { id: string; label: MessageKey }[];

type FilterId = (typeof FILTERS)[number]["id"];

export function ProgramPage(props: { snap: () => api.Snapshot }): JSX.Element {
  const view = signal<api.ProgramView>(api.programView());
  const filter = signal<FilterId>("all");
  /** 日志是否跟着最新一行走。 */
  const follow = signal(true);
  /** 「请再滚到底一次」的计数器（见 `ScrollArea` 那处的注释）。 */
  const jump = signal(0);
  // 换配色 / 明暗 / 关窗行为之后要重读一次：这几项都落在同一个配置文件里。
  const reload = () => view.set(api.programView());

  const kept = () => props.snap().logs.filter((line) => keeps(filter(), line.kind));
  const shown = () => kept().slice(-LOG_ROWS);

  /**
   * 复制日志：把**当前这一档**整段放进剪贴板（不是屏幕上那 200 行）。
   *
   * 报问题时把日志粘出去是这一页最常用的动作，只给个路径让用户自己去翻文件
   * 等于没给。旧界面有这颗按钮，搬过来时漏了。
   *
   * 复制成功后在按钮上回一句「已复制 N 行」：这个动作没有别的可见结果，不回报
   * 一句用户不知道到底成没成。不用定时器让它自己消失——下一次复制或换档时复位。
   */
  const copied = signal<number | null>(null);
  const copyError = signal<string | null>(null);
  const copy = () => {
    const lines = kept();
    if (lines.length === 0) return;
    const body = lines.map((line) => `${kindTag(line.kind)}  ${line.text}`).join("\n");
    Clipboard.writeText(body).then(
      () => {
        copied.set(lines.length);
        copyError.set(null);
      },
      (err: unknown) => {
        copied.set(null);
        copyError.set(err instanceof Error ? err.message : String(err));
      },
    );
  };

  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      {/* 内核 */}
      <Card>
        <CardHead
          title={t("program.coreSection")}
          hint={coreSummary(props.snap())}
          right={
            <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
              {/* 隧道在跑时按钮是灰的，原因写在这里：不写的话用户只会觉得按钮坏了。 */}
              {props.snap().active ? (
                <Caption>{t("program.coreLocked")}</Caption>
              ) : null}
              <Button
                label={t("program.checkUpdates")}
                disabled={props.snap().update.busy}
                onPress={() => api.checkUpdate()}
              />
              <Button
                label={
                  props.snap().installed ? t("program.reinstall") : t("program.download")
                }
                variant="primary"
                disabled={props.snap().update.busy || props.snap().active}
                onPress={() => api.installUpdate()}
              />
            </View>
          }
        />
        {/* 下载进度：有总长就是走的进度条，没有就画一条在呼吸的，别让人以为卡住了。 */}
        {props.snap().update.busy ? (
          <View style={{ flexDirection: "column", gap: 8 }}>
            <View
              style={{
                height: 6,
                borderRadius: 3,
                overflow: "hidden",
                backgroundColor: hue.muted(),
              }}
            >
              <View
                style={{
                  height: "100%",
                  borderRadius: 3,
                  backgroundColor: hue.primary(),
                  width:
                    props.snap().update.progress === null
                      ? "35%"
                      : `${Math.round((props.snap().update.progress ?? 0) * 100)}%`,
                  animationName: props.snap().update.progress === null ? "pulse" : "none",
                  animationDuration: "1200ms",
                  animationIterationCount: "infinite",
                }}
              />
            </View>
            <Caption>{props.snap().update.text ?? t("program.preparing")}</Caption>
          </View>
        ) : null}
        <View style={{ flexDirection: "column", gap: 4 }}>
          <PathRow label={t("program.binary")} value={view().binDir} />
          <PathRow label={t("program.logs")} value={view().logsDir} />
        </View>
      </Card>

      {/* 窗口与界面 */}
      <Card>
        <CardHead
          title={t("program.window")}
          hint={t("program.windowHint")}
        />
        <Setting
          label={t("program.closeButton")}
          hint={t("program.closeHint")}
        >
          <Chips>
            {view().closeOptions.map((option) => (
              <Chip
                label={option.label}
                chosen={view().closeAction === option.id}
                onPress={() => {
                  api.setCloseAction(option.id);
                  reload();
                }}
              />
            ))}
          </Chips>
        </Setting>
        <Setting
          label={t("program.language")}
          hint={t("program.languageHint")}
        >
          <Chips>
            {view().languageOptions.map((option) => (
              <Chip
                label={option.label}
                chosen={view().language === option.id}
                onPress={() => {
                  api.setLanguage(option.id);
                  // 语言一变，界面骨架的文案（`t()`）要立刻跟着换，不等下一秒的轮询。
                  syncLanguage(api.language());
                  reload();
                }}
              />
            ))}
          </Chips>
        </Setting>
        <Setting
          label={t("program.tray")}
          hint={t("program.trayHint")}
        >
          <Button label={t("program.minimiseToTray")} onPress={() => hideToTray()} />
        </Setting>
      </Card>

      {/* 关于 */}
      <Card>
        <CardHead
          title={t("program.about")}
          hint={t("program.aboutHint")}
        />
        <Setting
          label={t("program.title")}
          hint={t("program.aboutBlurb")}
        >
          <Text style={{ fontSize: 12, fontWeight: "600", color: ink.primary() }}>
            {`Tun2Proxy GUI ${view().version}`}
          </Text>
        </Setting>
        <View style={{ flexDirection: "column", gap: 4 }}>
          <PathRow label={t("program.about.config")} value={view().configPath} />
          <PathRow label={t("program.about.data")} value={view().dataDir} />
        </View>
      </Card>

      {/* 运行日志 */}
      <Card>
        <CardHead
          title={t("program.runLog")}
          hint={
            filter() === "all"
              ? t("program.logCount", { total: props.snap().logs.length, rows: LOG_ROWS })
              : t("program.logCountFiltered", { kept: kept().length, total: props.snap().logs.length, rows: LOG_ROWS })
          }
          right={
            <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
              <Button
                label={t("program.jumpLatest")}
                onPress={() => {
                  follow.set(true);
                  jump.set(jump() + 1);
                }}
              />
              <Chip
                label={t("program.follow")}
                chosen={follow()}
                onPress={() => follow.set(!follow())}
              />
              <Button label={t("common.clear")} onPress={() => api.clearLogs()} />
            </View>
          }
        />
        {/* 日志区：底色用「页面底色」而不是「浅色底」，否则浅色主题下和白卡片
            几乎同色，一大块空地看着像没画完。内容包一层 ScrollArea 并自动贴底
            —— 200 行摞起来远高于 300，不滚的话看到的反而是最旧的那一截。 */}
        <View
          style={{
            height: 300,
            borderRadius: 10,
            backgroundColor: surface.inset(),
            padding: 8,
          }}
        >
          {/* 跟随：开着就一直贴着底（新日志自己滚出来）；关掉就冻在当前这一屏，
              往上翻旧日志时不会被新行拽回去。`jump` 只是个「请再滚一次」的计数器
              ——`stickToBottom` 只在返回值变化时才滚，光把 follow 打开不会触发。 */}
          <ScrollArea
            stickToBottom={() => (follow() ? props.snap().logs.length : 0) + jump()}
          >
            <View style={{ flexDirection: "column", gap: 2 }}>
              {shown().map((line) => (
                <View style={{ flexDirection: "row", alignItems: "flex-start", gap: 8 }}>
                  <Text
                    style={{
                      width: 44,
                      fontSize: 10,
                      color: kindColor(line.kind),
                      fontVariant: ["tabular-nums"],
                    }}
                  >
                    {kindTag(line.kind)}
                  </Text>
                  <Text style={{ flex: 1, fontSize: 10, color: ink.card() }}>{line.text}</Text>
                </View>
              ))}
              {shown().length === 0 ? (
                <Text style={{ fontSize: 10, color: ink.muted() }}>
                  {props.snap().logs.length === 0
                    ? t("program.logEmptyHint")
                    : t("program.logEmpty")}
                </Text>
              ) : null}
            </View>
          </ScrollArea>
        </View>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Chips>
            {FILTERS.map((item) => (
              <Chip
                label={t(item.label)}
                chosen={filter() === item.id}
                onPress={() => {
                  filter.set(item.id);
                  // 换档之后行数变了，上一次的「已复制 N 行」不再对得上。
                  copied.set(null);
                }}
              />
            ))}
          </Chips>
          <Button
            label={copied() === null ? t("common.copy") : t("program.copiedLines", { count: copied() ?? 0 })}
            onPress={copy}
          />
          {copyError() ? (
            <Text style={{ fontSize: 10, color: hue.danger() }}>
              {t("program.error.copyFailed", { msg: copyError() ?? "" })}
            </Text>
          ) : (
            <Caption>
              {t("program.logPathHint")}
            </Caption>
          )}
        </View>
      </Card>
    </View>
  );
}

/** 路径的一行：左边标签、右边等宽路径。 */
function PathRow(props: { label: string; value: string }): JSX.Element {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
      <Text style={{ width: 52, fontSize: 10, color: ink.muted() }}>{props.label}</Text>
      <Text style={{ flex: 1, fontSize: 10, color: ink.card() }}>
        {props.value}
      </Text>
    </View>
  );
}

/** 内核那一行的说明：版本 + 更新结论。 */
function coreSummary(snap: api.Snapshot): string {
  const version = snap.version ? `tun2proxy ${snap.version}` : t("program.notInstalled");
  const update = snap.update.text ? ` · ${snap.update.text}` : "";
  return `${version}${update}`;
}

/** 一条日志行是否属于这一档。 */
function keeps(filter: FilterId, kind: string): boolean {
  if (filter === "all") return true;
  if (filter === "output") return kind === "output";
  if (filter === "app") return kind === "app";
  return kind === "split";
}

/** 日志行前面的短标签。 */
function kindTag(kind: string): string {
  if (kind === "output") return t("program.about.core");
  if (kind === "app") return t("program.about.app");
  if (kind === "split") return t("program.about.split");
  return t("program.about.other");
}

/** 标签的颜色。 */
function kindColor(kind: string): string {
  if (kind === "output") return hue.direct();
  if (kind === "app") return hue.primary();
  if (kind === "split") return hue.proxy();
  return ink.muted();
}
