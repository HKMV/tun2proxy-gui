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

import { signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "../api";
import { Button, Card, CardHead, Caption, Chip, Chips, ScrollArea, Setting } from "../components";
import { hideToTray } from "../desktop";
import { hue, ink, surface } from "../theme";

/** 日志区画多少行。 */
const LOG_ROWS = 200;

/** 日志过滤的四档（和后端的 `LogFilter` 一一对应）。 */
const FILTERS = [
  { id: "all", label: "全部" },
  { id: "output", label: "内核输出" },
  { id: "app", label: "本程序" },
  { id: "split", label: "分流判定" },
] as const;

type FilterId = (typeof FILTERS)[number]["id"];

export function ProgramPage(props: { snap: () => api.Snapshot }): JSX.Element {
  const view = signal<api.ProgramView>(api.programView());
  const filter = signal<FilterId>("all");
  // 换配色 / 明暗 / 关窗行为之后要重读一次：这几项都落在同一个配置文件里。
  const reload = () => view.set(api.programView());

  const kept = () => props.snap().logs.filter((line) => keeps(filter(), line.kind));
  const shown = () => kept().slice(-LOG_ROWS);

  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      {/* 内核 */}
      <Card>
        <CardHead
          title="tun2proxy 内核"
          hint={coreSummary(props.snap())}
          right={
            <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
              {/* 隧道在跑时按钮是灰的，原因写在这里：不写的话用户只会觉得按钮坏了。 */}
              {props.snap().active ? (
                <Caption>隧道运行中，先停止才能换内核</Caption>
              ) : null}
              <Button
                label="检查更新"
                disabled={props.snap().update.busy}
                onPress={() => api.checkUpdate()}
              />
              <Button
                label={props.snap().installed ? "重新安装" : "下载并安装"}
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
            <Caption>{props.snap().update.text ?? "正在准备…"}</Caption>
          </View>
        ) : null}
        <View style={{ flexDirection: "column", gap: 4 }}>
          <PathRow label="二进制" value={view().binDir} />
          <PathRow label="日志" value={view().logsDir} />
        </View>
      </Card>

      {/* 窗口与界面 */}
      <Card>
        <CardHead
          title="窗口与界面"
          hint="点关闭按钮时怎么处理，以及界面用哪种语言"
        />
        <Setting
          label="关闭按钮"
          hint="tun2proxy 是常驻服务，误关窗口会把正在跑的隧道一起带走"
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
        <Setting label="界面语言" hint="「跟随系统」按系统语言决定；改完立刻生效">
          <Chips>
            {view().languageOptions.map((option) => (
              <Chip
                label={option.label}
                chosen={view().language === option.id}
                onPress={() => {
                  api.setLanguage(option.id);
                  reload();
                }}
              />
            ))}
          </Chips>
        </Setting>
        <Setting
          label="托盘"
          hint="隧道在后台跑着的时候，可以把窗口收进托盘，图标点一下就能叫回来"
        >
          <Button label="最小化到托盘" onPress={() => hideToTray()} />
        </Setting>
      </Card>

      {/* 关于 */}
      <Card>
        <CardHead title="关于" hint="这个前端本身的信息" />
        <Setting label="程序" hint="官方 tun2proxy 二进制的图形前端，Rust 后端 + nsh 界面">
          <Text style={{ fontSize: 12, fontWeight: "600", color: ink.primary() }}>
            {`Tun2Proxy GUI ${view().version}`}
          </Text>
        </Setting>
        <View style={{ flexDirection: "column", gap: 4 }}>
          <PathRow label="配置" value={view().configPath} />
          <PathRow label="数据" value={view().dataDir} />
        </View>
      </Card>

      {/* 运行日志 */}
      <Card>
        <CardHead
          title="运行日志"
          hint={
            filter() === "all"
              ? `共 ${props.snap().logs.length} 行，只列最后 ${LOG_ROWS} 行`
              : `${kept().length} / ${props.snap().logs.length} 行，只列最后 ${LOG_ROWS} 行`
          }
          right={<Button label="清空" onPress={() => api.clearLogs()} />}
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
          <ScrollArea stickToBottom={() => props.snap().logs.length}>
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
                    ? "还没有日志：启动隧道之后这里会有内核输出"
                    : "这一档没有日志行，换一档看看"}
                </Text>
              ) : null}
            </View>
          </ScrollArea>
        </View>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
          <Chips>
            {FILTERS.map((item) => (
              <Chip
                label={item.label}
                chosen={filter() === item.id}
                onPress={() => filter.set(item.id)}
              />
            ))}
          </Chips>
          <Caption>完整日志在磁盘文件里，路径见上面的「日志」一行。</Caption>
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
  const version = snap.version ? `tun2proxy ${snap.version}` : "还没安装 tun2proxy";
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
  if (kind === "output") return "内核";
  if (kind === "app") return "程序";
  if (kind === "split") return "分流";
  return "其他";
}

/** 标签的颜色。 */
function kindColor(kind: string): string {
  if (kind === "output") return hue.direct();
  if (kind === "app") return hue.primary();
  if (kind === "split") return hue.proxy();
  return ink.muted();
}
