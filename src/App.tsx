//! 界面入口（TSX 这一套）。
//!
//! ```text
//! App.tsx            整窗骨架：顶栏 + 侧栏 + 内容区
//! app/api.ts         后端接口（唯一 import tun2proxy/native 的地方）
//! app/theme.ts       颜色语义 + 数字格式化
//! app/components.tsx 卡片、胶囊、按钮这些小块
//! ```
//!
//! 页面全部只「读快照 + 调动作」：数据每秒从 `api.snapshot()` 拿一份，渲染完
//! 就扔。界面不缓存业务状态，所以窗口重建、页面切换都不会影响内核。

import { effect, setColorScheme, setTheme, signal, Text, View } from "nsh";
import type { JSX } from "nsh";

import * as api from "./app/api";
import { Button, Card, CardHead, Caption, Dot, Metric, ScrollArea } from "./app/components";
import { bindWindow, hideToTray, installTray, interceptClose, quit } from "./app/desktop";
import { NetworkPage } from "./app/pages/Network";
import { ProfilesPage } from "./app/pages/Profiles";
import { ProgramPage } from "./app/pages/Program";
import { SplitPage } from "./app/pages/Split";
import { bumpTheme, hue, ink, rateText, surface } from "./app/theme";

/** 导航项。 */
const PAGES = [
  { id: "dashboard", label: "仪表盘", hint: "隧道状态、实时流量" },
  { id: "profiles", label: "代理配置", hint: "多套上游代理" },
  { id: "split", label: "分流规则", hint: "国内直连、清单与出口" },
  { id: "network", label: "网络与绕过", hint: "TUN 网卡、UDP 与绕行" },
  { id: "program", label: "程序与日志", hint: "内核版本、日志与更新" },
] as const;

type PageId = (typeof PAGES)[number]["id"];

/** 最近两分钟：每秒一个点。 */
const HISTORY_POINTS = 120;

/** 可选的配色。色块不写死色值——从各套 JSON 的 `primary` 里读，改配色不用改这里。 */
const PALETTES = [
  { id: "cyber", label: "赛博青" },
  { id: "matrix", label: "矩阵绿" },
  { id: "ember", label: "熔岩橙" },
  { id: "aurora", label: "极光紫" },
] as const;

/** 明暗三档。`auto` 交给系统。 */
const THEMES = [
  { id: "light", label: "浅色" },
  { id: "auto", label: "跟随" },
  { id: "dark", label: "深色" },
] as const;

/**
 * 启动时的一次性数据：配置里的配色 / 明暗 / 版本。
 *
 * 配色与明暗必须**在第一次渲染之前**套上，否则第一帧是 nsh 的默认黑白，随后
 * 再闪一下。所以放在模块顶层、`mount` 之前。
 */
const BOOT = api.boot();

/** 一套配色的主色（色块用）：读它 JSON 里的 `primary`。结果缓存——色块每秒都画，别每次都解析。 */
const SWATCHES = new Map<string, string>();
function swatchOf(palette: string): string {
  const cached = SWATCHES.get(palette);
  if (cached) return cached;
  let color = "#888888";
  try {
    const theme = JSON.parse(api.paletteJson(palette)) as {
      cssVars?: { dark?: Record<string, string>; light?: Record<string, string> };
    };
    color = theme.cssVars?.dark?.primary ?? theme.cssVars?.light?.primary ?? color;
  } catch {
    // 配色文件坏了：用中性灰，至少色块还在。
  }
  SWATCHES.set(palette, color);
  return color;
}

/** 配置里的明暗 → nsh 的外观光。 */
function schemeOf(setting: string): "light" | "dark" | "system" {
  return setting === "light" || setting === "dark" ? setting : "system";
}

// 第一帧之前就把配色和明暗定下来，免得先闪一下默认配色。
const BOOT_THEME = JSON.parse(BOOT.themeJson) as Parameters<typeof setTheme>[0];
setTheme(BOOT_THEME);
setColorScheme(schemeOf(BOOT.theme));

// 窗口起来之后还要再套一次：nsh 建窗时会按 `nsh.json` 里的 `theme` 再装一遍，
// 把这里盖掉；而 `setTheme` 对「和当前一样」的值直接跳过重绘，于是界面停在
// `nsh.json` 那套上，配置里存的配色根本看不见（点一个别的配色才盖得回来）。
// 先清空再装，保证这一次一定算「变了」。只在第一次布局时补一次。
let bootThemeSettled = false;
function settleBootTheme(): void {
  if (bootThemeSettled) return;
  bootThemeSettled = true;
  setTheme(null);
  setTheme(BOOT_THEME);
  // 组件体里算出来的颜色是按「那一刻」的主题算的，换完得通知它们重算一遍：
  // 第一帧就挂上去、之后 props 又没变的元素（状态卡那颗主按钮）否则会一直
  // 停在旧配色的青色上。
  bumpTheme();
}

/** 整窗：顶栏 + 侧栏 + 内容区。 */
export function App(): JSX.Element {
  // 当前页 + 这一帧的数据。data 每秒刷新一次；界面不存别的业务状态。
  const page = signal<PageId>("dashboard");
  const snap = signal<api.Snapshot>(api.snapshot());
  const notice = signal<string | null>(null);
  const palette = signal<string>(BOOT.palette);
  const themeSetting = signal<string>(BOOT.theme);
  /** 关窗确认浮层开着没有（关窗行为配成「每次询问」时用）。 */
  const askClose = signal(false);

  // 换配色：先把新 JSON 套上去（颜色立刻变），再让后端存盘。
  const choosePalette = (id: string) => {
    const json = api.paletteJson(id);
    if (json && json !== "{}") {
      setTheme(JSON.parse(json) as Parameters<typeof setTheme>[0]);
    }
    palette.set(id);
    bumpTheme();
    api.setPalette(id);
  };

  // 换明暗：外观由 nsh 的平台层决定（`system` 交给系统），配置同样要存盘。
  const chooseTheme = (id: string) => {
    setColorScheme(schemeOf(id));
    themeSetting.set(id);
    bumpTheme();
    api.setTheme(id);
  };

  effect(() => {
    const id = setInterval(() => {
      snap.set(api.snapshot());
    }, 1000);
    return () => clearInterval(id);
  });

  /**
   * 点完「启动」到内核真的开始启动之间还有一段：内核先在后台探上游代理
   * （最多 3 秒），这段里状态机还是「未启动」，蒙版不会自己出来——用户点了
   * 一两秒没反应，就会以为按钮坏了，然后反复点。
   *
   * 所以这里自己记一个「正在预检」：点下去立刻上蒙版，等状态或结论一变就收。
   */
  const starting = signal(false);
  let clickedAt = 0;
  let startingTimer: number | null = null;

  const stopStarting = () => {
    starting.set(false);
    if (startingTimer !== null) {
      clearTimeout(startingTimer);
      startingTimer = null;
    }
  };

  const toggle = () => {
    const state = snap();
    if (state.busy || starting()) return;
    // 桥接调用抛出来的东西没人接：按钮照旧没反应，用户只能以为它坏了。
    // 统一兜住，把原因摆到状态卡那一行上。
    try {
      if (state.active) {
        api.stop();
        return;
      }
      clickedAt = Date.now();
      starting.set(true);
      // 兜底：预检自己出不来结论时，最多盖 8 秒。
      startingTimer = setTimeout(stopStarting, 8000);
      const result = api.start();
      if (!result.ok) {
        stopStarting();
        notice.set(result.error ?? "启动失败");
      } else {
        notice.set(null);
      }
    } catch (err) {
      stopStarting();
      notice.set(`启动/停止时出错：${err instanceof Error ? err.message : String(err)}`);
    }
  };

  // 预检有结论就收掉蒙版：真的开始启动了（active/busy），或者被挡下来
  // （blocked 有话说）。blocked 要等一拍再看——上一轮的失败原因还挂在快照上，
  // 点下去内核清掉它要等下一次刷新，早了会把这一轮的蒙版当场收掉。
  effect(() => {
    if (!starting()) return;
    const s = snap();
    const settled =
      s.active || s.busy || (s.blocked !== null && Date.now() - clickedAt > 1200);
    if (settled) stopStarting();
  });

  // 托盘与关窗：这两个是「窗口」的事，和页面无关，所以挂在这里一次。
  //
  // 关窗行为每次现读配置：程序页里改完立刻生效，不用再维护一份副本。
  bindWindow();
  installTray({ snap, onToggle: toggle });
  interceptClose({
    action: () => api.programView().closeAction,
    onAsk: () => askClose.set(true),
  });

  /**
   * 没有管理员权限时的那句话。
   *
   * 发布版在开窗口之前就 UAC 重启过自己（`main.rs` 的 `ensure_elevated`），走不到
   * 这里；`nsh run` 的开发宿主和 `nsh package` 打的包没有那一步，进程就是普通
   * 权限——这时候点「启动」只会拿到一句内核报错，用户根本猜不到是权限问题。
   */
  const privilege = signal(api.privilegeHint());
  const privilegeError = signal<string | null>(null);

  return (
    <View
      onLayout={settleBootTheme}
      style={{
        flex: 1,
        flexDirection: "column",
        backgroundColor: surface.background(),
      }}
    >
      <TopBar
        snap={snap}
        palette={palette}
        themeSetting={themeSetting}
        onPalette={choosePalette}
        onTheme={chooseTheme}
      />
      {/* 权限条：只在真的没有权限时出现，横跨整窗——任何一页点「启动」都该
          看到同一句后果，而不是各页各说一套。 */}
      {privilege() ? (
        <View
          style={{
            flexDirection: "row",
            alignItems: "center",
            gap: 10,
            paddingLeft: 16,
            paddingRight: 16,
            paddingTop: 8,
            paddingBottom: 8,
            backgroundColor: surface.inset(),
            borderBottomWidth: 1,
            borderColor: surface.border(),
          }}
        >
          <Dot color={hue.warn()} size={6} />
          <Text style={{ flex: 1, fontSize: 11, color: ink.primary() }}>{privilege()}</Text>
          {privilegeError() ? (
            <Text style={{ fontSize: 11, color: hue.danger() }}>{privilegeError()}</Text>
          ) : null}
          <Button
            label="以管理员身份重启"
            onPress={() => {
              try {
                const result = api.relaunchElevated();
                if (result.ok) {
                  // 新进程已经起来了，本进程必须让位：留着就是两个窗口、两份托盘。
                  quit();
                  return;
                }
                privilegeError.set(result.error ?? "提权失败");
              } catch (err) {
                privilegeError.set(err instanceof Error ? err.message : String(err));
              }
            }}
          />
        </View>
      ) : null}
      <View style={{ flex: 1, flexDirection: "row" }}>
        <SideBar
          page={page}
          snap={snap}
          starting={starting}
          onToggle={toggle}
          onSelect={(id) => {
            notice.set(null);
            page.set(id);
          }}
        />
        <ScrollArea resetOn={page} style={{ flex: 1, padding: 24 }}>
          {/* 上限 1200 并居中：窗口一拉大，宽屏下卡片能到 1300+，状态卡的数字
              被推到右侧边缘、离标题老远，中间空出一大片。窄窗不受影响。 */}
          <View
            style={{
              flexDirection: "column",
              gap: 16,
              width: "100%",
              maxWidth: 1200,
              alignSelf: "center",
            }}
          >
            <PageHeader page={page} snap={snap} />
            {/* 启动被本地校验挡回来时的一句话。放在这里而不是仪表盘里：
                侧栏那颗按钮任何页都能点，坐在程序页点完却什么都看不到，
                只能以为它没反应。点一下这行可以消掉。 */}
            {notice() ? (
              <View
                onPress={() => notice.set(null)}
                style={{ flexDirection: "row", alignItems: "flex-start", gap: 8, cursor: "pointer" }}
              >
                <Text style={{ fontSize: 11, color: hue.danger() }}>启动失败：{notice()}</Text>
                <Text style={{ fontSize: 11, color: ink.muted() }}>（点一下消掉）</Text>
              </View>
            ) : null}
            {
              page() === "dashboard" ? (
                <Dashboard snap={snap} notice={notice} starting={starting} onToggle={toggle} />
              ) : page() === "profiles" ? (
                <ProfilesPage snap={snap} />
              ) : page() === "split" ? (
                <SplitPage snap={snap} />
              ) : page() === "network" ? (
                <NetworkPage snap={snap} />
              ) : (
                <ProgramPage snap={snap} />
              )
            }
          </View>
        </ScrollArea>
      </View>
      {/* 启停蒙版：这几秒内核在干活，整窗盖住，别让人反复点。 */}
      {snap().busy || starting() ? (
        <BusyMask
          snap={snap}
          label={starting() ? "正在检查上游代理…" : undefined}
          hint={
            starting()
              ? "连通性检测最多 3 秒；不通会取消启动并说明原因。"
              : undefined
          }
        />
      ) : null}
      {/* 关窗确认：配成「每次询问」时才出现。整窗蒙一层，问清楚再走。 */}
      {askClose() ? (
        <View
          style={{
            position: "absolute",
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            alignItems: "center",
            justifyContent: "center",
            backgroundColor: "rgba(0, 0, 0, 0.55)",
          }}
        >
          <Card style={{ width: 400 }}>
            <CardHead
              title="关闭窗口？"
              hint="tun2proxy 是常驻服务：退出会把正在跑的隧道一起带走，网会断。收进托盘可以继续在后台跑。"
            />
            <View style={{ flexDirection: "row", justifyContent: "flex-end", gap: 8 }}>
              <Button
                label="继续后台运行"
                onPress={() => {
                  askClose.set(false);
                  hideToTray();
                }}
              />
              <Button
                label="退出程序"
                variant="danger"
                onPress={() => {
                  askClose.set(false);
                  quit();
                }}
              />
            </View>
          </Card>
        </View>
      ) : null}
    </View>
  );
}

/**
 * 启停蒙版：整窗盖住，中间一个转圈的环 + 「在等哪一步」。
 *
 * 启动要走「探上游 → 清残留网卡 → 起分流器 → 拉子进程」，停止要等子进程退出
 * 和网卡回收，都是几秒级的。只把按钮变灰的话，用户会以为程序没反应，然后反复
 * 点——内核那两次调用都不是幂等的。
 *
 * 盖的是**界面**：系统标题栏在外面，蒙版期间照样能点叉关窗（那是窗口的事）。
 */
function BusyMask(props: {
  snap: () => api.Snapshot;
  /** 覆盖后端给的文案（预检那一段后端还没有状态）。 */
  label?: string;
  hint?: string;
}): JSX.Element {
  // 文案由后端给（`busyLabel`）：状态机在那边，界面不复述一套。
  const label = () => props.label || props.snap().busyLabel || "请稍等…";
  const hint = () =>
    props.hint ||
    (props.snap().status === "stopping"
      ? "等子进程退出、网卡回收，这几秒不要再点一次。"
      : "探上游代理、清理残留网卡与进程，这几秒不要再点一次。");
  return (
    <View
      style={{
        position: "absolute",
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: "rgba(0, 0, 0, 0.55)",
      }}
    >
      <View
        style={{
          width: 420,
          padding: 24,
          borderRadius: 16,
          alignItems: "center",
          gap: 16,
          backgroundColor: surface.card(),
        }}
      >
        {/* 两段主色 + 两段淡色的环，靠 `spin` 转起来才看得出在动。 */}
        <View
          style={{
            width: 34,
            height: 34,
            borderRadius: 17,
            borderWidth: 3,
            borderTopColor: hue.primary(),
            borderRightColor: hue.primary(),
            borderBottomColor: hue.muted(),
            borderLeftColor: hue.muted(),
            animationName: "spin",
            animationDuration: "800ms",
            animationTimingFunction: "linear",
            animationIterationCount: "infinite",
            animationFrameRate: 30,
          }}
        />
        <Text style={{ fontSize: 14, fontWeight: "600", color: ink.primary() }}>{label()}</Text>
        <Text style={{ fontSize: 11, color: ink.muted() }}>{hint()}</Text>
      </View>
    </View>
  );
}

/**
 * 顶栏：品牌 + 状态 + 配色 / 明暗切换 + 版本。
 *
 * 左边那块（品牌，占满剩余宽度）带 `windowDrag`：抓住顶栏空白处就能拖窗口，
 * 右边的按钮不带，所以点它们不会顺手把窗口拖走。
 */
function TopBar(props: {
  snap: () => api.Snapshot;
  palette: () => string;
  themeSetting: () => string;
  onPalette: (id: string) => void;
  onTheme: (id: string) => void;
}): JSX.Element {
  return (
    <View
      style={{
        height: 52,
        flexDirection: "row",
        alignItems: "center",
        gap: 12,
        paddingLeft: 16,
        paddingRight: 12,
        backgroundColor: surface.sidebar(),
        borderBottomWidth: 1,
        borderColor: surface.sidebarBorder(),
      }}
    >
      <View
        desktop={{ windowDrag: true }}
        style={{ flex: 1, height: "100%", flexDirection: "row", alignItems: "center", gap: 12 }}
      >
        <View
          style={{
            width: 26,
            height: 26,
            borderRadius: 8,
            alignItems: "center",
            justifyContent: "center",
            backgroundImage: `linear-gradient(135deg, ${hue.direct()}, ${hue.proxy()})`,
          }}
        >
          <Text style={{ fontSize: 13, fontWeight: "700", color: hue.primaryInk() }}>
            T
          </Text>
        </View>
        <View style={{ flexDirection: "column", gap: 2 }}>
          <Text style={{ fontSize: 13, fontWeight: "600", color: ink.primary() }}>Tun2Proxy</Text>
          <Text style={{ fontSize: 10, color: ink.muted() }}>隧道控制台</Text>
        </View>
      </View>
      <StatusPill snap={props.snap} />
      <PaletteDots current={props.palette()} onChoose={props.onPalette} />
      <ThemeChips current={props.themeSetting()} onChoose={props.onTheme} />
      <View
        style={{
          height: 24,
          paddingLeft: 8,
          paddingRight: 8,
          borderRadius: 12,
          alignItems: "center",
          justifyContent: "center",
          backgroundColor: hue.accent(),
        }}
      >
        <Text style={{ fontSize: 10, color: ink.muted(), fontVariant: ["tabular-nums"] }}>
          {`v${BOOT.version}`}
        </Text>
      </View>
    </View>
  );
}

/** 四个配色小圆点：各套自己的主色，选中的加一圈描边。 */
function PaletteDots(props: {
  current: string;
  onChoose: (id: string) => void;
}): JSX.Element {
  return (
    <View style={{ flexDirection: "row", alignItems: "center", gap: 2 }}>
      {PALETTES.map((item) => {
        const chosen = props.current === item.id;
        // 24×24 的透明方块才是可点区域：圆点本身只有 16，直接点它太考验手速。
        return (
          <View
            onPress={chosen ? undefined : () => props.onChoose(item.id)}
            style={{
              width: 24,
              height: 24,
              borderRadius: 12,
              alignItems: "center",
              justifyContent: "center",
              cursor: chosen ? "default" : "pointer",
            }}
            hoverStyle={chosen ? undefined : { backgroundColor: hue.accent() }}
          >
            <View
              style={{
                width: chosen ? 16 : 12,
                height: chosen ? 16 : 12,
                borderRadius: 8,
                backgroundColor: swatchOf(item.id),
                borderWidth: chosen ? 2 : 0,
                borderColor: ink.primary(),
              }}
            />
          </View>
        );
      })}
    </View>
  );
}

/** 明暗三档。 */
function ThemeChips(props: { current: string; onChoose: (id: string) => void }): JSX.Element {
  return (
    <View
      style={{
        flexDirection: "row",
        alignItems: "center",
        gap: 2,
        padding: 2,
        height: 26,
        borderRadius: 13,
        backgroundColor: hue.accent(),
      }}
    >
      {THEMES.map((item) => {
        const chosen = props.current === item.id;
        return (
          <View
            onPress={chosen ? undefined : () => props.onChoose(item.id)}
            style={{
              height: 22,
              paddingLeft: 10,
              paddingRight: 10,
              borderRadius: 11,
              alignItems: "center",
              justifyContent: "center",
              backgroundColor: chosen ? hue.primary() : "transparent",
              cursor: chosen ? "default" : "pointer",
              transitionProperty: "background-color",
              transitionDuration: "140ms",
            }}
            hoverStyle={chosen ? undefined : { backgroundColor: surface.card() }}
          >
            <Text
              style={{
                fontSize: 10,
                fontWeight: chosen ? "600" : "400",
                color: chosen ? hue.primaryInk() : ink.muted(),
              }}
            >
              {item.label}
            </Text>
          </View>
        );
      })}
    </View>
  );
}

/** 状态胶囊：一个灯 + 一句话。 */
function StatusPill(props: { snap: () => api.Snapshot }): JSX.Element {
  const color = () =>
    props.snap().status === "failed"
      ? hue.danger()
      : props.snap().active
        ? hue.ok()
        : hue.warn();
  return (
    <View
      style={{
        height: 26,
        paddingLeft: 10,
        paddingRight: 12,
        borderRadius: 13,
        flexDirection: "row",
        alignItems: "center",
        gap: 8,
        backgroundColor: hue.accent(),
      }}
    >
      <Dot color={color()} pulse={!props.snap().active} />
      <Text style={{ fontSize: 11, color: ink.primary() }}>{props.snap().statusLabel}</Text>
    </View>
  );
}

/** 侧栏：五个导航项 + 底部那颗启停按钮。 */
function SideBar(props: {
  page: () => PageId;
  snap: () => api.Snapshot;
  starting: () => boolean;
  onToggle: () => void;
  onSelect: (id: PageId) => void;
}): JSX.Element {
  return (
    <View
      style={{
        width: 176,
        padding: 12,
        gap: 4,
        backgroundColor: surface.sidebar(),
        borderRightWidth: 1,
        borderColor: surface.sidebarBorder(),
        justifyContent: "space-between",
      }}
    >
      <View style={{ flexDirection: "column", gap: 2 }}>
        <Text
          style={{
            fontSize: 10,
            color: ink.muted(),
            paddingLeft: 12,
            paddingTop: 8,
            paddingBottom: 8,
          }}
        >
          导航
        </Text>
        {PAGES.map((item) => (
          <NavItem
            label={item.label}
            hint={item.hint}
            chosen={props.page() === item.id}
            onPress={() => props.onSelect(item.id)}
          />
        ))}
      </View>
      <ToggleButton snap={props.snap} starting={props.starting} onPress={props.onToggle} />
    </View>
  );
}

/** 一个导航项。 */
function NavItem(props: {
  label: string;
  hint: string;
  chosen: boolean;
  onPress: () => void;
}): JSX.Element {
  return (
    <View
      onPress={props.onPress}
      style={{
        paddingLeft: 12,
        paddingRight: 12,
        paddingTop: 8,
        paddingBottom: 8,
        borderRadius: 8,
        flexDirection: "column",
        gap: 2,
        cursor: "pointer",
        backgroundColor: props.chosen ? hue.accent() : "transparent",
        transitionProperty: "background-color",
        transitionDuration: "120ms",
      }}
      hoverStyle={props.chosen ? undefined : { backgroundColor: hue.accent() }}
    >
      <Text
        style={{
          fontSize: 12,
          fontWeight: props.chosen ? "600" : "400",
          color: props.chosen ? ink.primary() : ink.muted(),
        }}
      >
        {props.label}
      </Text>
    </View>
  );
}

/**
 * 侧栏底部那颗按钮：未启动时「启动」，运行中「停止」。
 *
 * 底色和文字都长在 children 分支里，不写在 `style` 上：nsh 的组件体只跑一次，
 * 而 `style` 是个字面对象，在里面读信号拿到的永远是首帧那一次（按钮会一直停在
 * 「启动」、一直是主色）——只有放在 JSX 子节点位置的表达式才跟着信号走。
 *
 * 忙碌期间由蒙版挡着，这里再按一次也只当没点：内核那两次调用不是幂等的。
 */
function ToggleButton(props: {
  snap: () => api.Snapshot;
  starting: () => boolean;
  onPress: () => void;
}): JSX.Element {
  const running = () => props.snap().active;
  // 预检那几秒内核还没进「启动中」，光看 busy 挡不住第二次点击。
  const blocked = () => props.snap().busy || props.starting();
  // 文案走 children（跟着信号走），不写进 style：nsh 的 style 字面量只在挂载/换主题时读。
  const face = (label: string, background: string) => (
    <View
      style={{
        flex: 1,
        borderRadius: 18,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: background,
        transitionProperty: "background-color",
        transitionDuration: "160ms",
      }}
    >
      <Text style={{ fontSize: 12, fontWeight: "600", color: hue.primaryInk() }}>{label}</Text>
    </View>
  );
  return (
    <View
      onPress={() => {
        if (!blocked()) props.onPress();
      }}
      style={{ height: 36, borderRadius: 18, cursor: "pointer" }}
    >
      {running()
        ? face(props.snap().busy ? "停止中…" : "停止", hue.danger())
        : face(blocked() ? "启动中…" : "启动", hue.primary())}
    </View>
  );
}

/** 页面标题 + 右侧一句实时读数。 */
function PageHeader(props: { page: () => PageId; snap: () => api.Snapshot }): JSX.Element {
  const meta = () => PAGES.find((item) => item.id === props.page()) ?? PAGES[0];
  const version = () => {
    const snap = props.snap();
    return `${snap.statusLabel} · ${snap.version ? `tun2proxy ${snap.version}` : "未安装"}`;
  };
  return (
    <View style={{ flexDirection: "row", alignItems: "flex-end", gap: 16 }}>
      <View style={{ flex: 1, flexDirection: "column", gap: 4 }}>
        <Text style={{ fontSize: 22, fontWeight: "600", color: ink.primary() }}>{meta().label}</Text>
        <Text style={{ fontSize: 12, color: ink.muted() }}>{meta().hint}</Text>
      </View>
      <Text style={{ fontSize: 11, color: ink.muted(), fontVariant: ["tabular-nums"] }}>
        {version()}
      </Text>
    </View>
  );
}

/** 仪表盘。 */
function Dashboard(props: {
  snap: () => api.Snapshot;
  notice: () => string | null;
  starting: () => boolean;
  onToggle: () => void;
}): JSX.Element {
  return (
    <View style={{ flexDirection: "column", gap: 16 }}>
      <StatusCard
        snap={props.snap}
        notice={props.notice}
        starting={props.starting}
        onToggle={props.onToggle}
      />
      <TrafficCard snap={props.snap} />
    </View>
  );
}

/** 状态卡：状态 + 四个速率 + 启停。 */
function StatusCard(props: {
  snap: () => api.Snapshot;
  notice: () => string | null;
  starting: () => boolean;
  onToggle: () => void;
}): JSX.Element {
  const snap = () => props.snap();
  const color = () =>
    snap().status === "failed" ? hue.danger() : snap().active ? hue.ok() : hue.warn();
  return (
    <Card>
      <View style={{ flexDirection: "row", alignItems: "center", gap: 16 }}>
        <View style={{ flex: 1, flexDirection: "column", gap: 6 }}>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <Dot color={color()} size={10} pulse={!snap().active} />
            <Text style={{ fontSize: 10, fontWeight: "600", color: ink.muted() }}>隧道状态</Text>
          </View>
          <Text style={{ fontSize: 24, fontWeight: "600", color: ink.primary() }}>
            {snap().statusLabel}
          </Text>
          <Text style={{ fontSize: 11, color: ink.muted() }}>
            {props.notice() ?? snap().detail ?? snap().blocked ?? "启动后这里显示直连与代理的实时速率。"}
          </Text>
        </View>
        <Metric label="直连 ↓" value={rateText(snap().direct.down)} color={hue.direct()} />
        <Metric label="直连 ↑" value={rateText(snap().direct.up)} color={hue.direct()} />
        <Metric label="代理 ↓" value={rateText(snap().proxy.down)} color={hue.proxy()} />
        <Metric label="代理 ↑" value={rateText(snap().proxy.up)} color={hue.proxy()} />
        <Button
          label={snap().active ? "停止" : "启动"}
          variant="primary"
          disabled={snap().busy || props.starting()}
          onPress={props.onToggle}
          style={{ height: 38, paddingLeft: 24, paddingRight: 24, borderRadius: 19 }}
        />
      </View>
    </Card>
  );
}

/** 流量卡：两分钟的堆叠柱状图 + 分流计数。 */
function TrafficCard(props: { snap: () => api.Snapshot }): JSX.Element {
  const points = () => props.snap().history.slice(-HISTORY_POINTS);
  const peak = () =>
    Math.max(1, ...points().map((point) => point.direct.down + point.proxy.down));
  // 一根柱子都没有时整个图区是空白，看起来像「坏了」——补一句说明。
  const idle = () => points().every((point) => point.direct.down + point.proxy.down === 0);
  return (
    <Card>
      <CardHead
        title="实时流量"
        hint="直连在下、代理在上，最近两分钟"
        right={
          <View style={{ flexDirection: "row", gap: 16 }}>
            <Metric label="直连" value={rateText(props.snap().direct.down)} color={hue.direct()} />
            <Metric label="代理" value={rateText(props.snap().proxy.down)} color={hue.proxy()} />
          </View>
        }
      />
      <View
        style={{
          height: 120,
          flexDirection: "row",
          alignItems: "flex-end",
          gap: 1,
        }}
      >
        {points().map((point) => {
          const total = point.direct.down + point.proxy.down;
          const height = Math.max(total > 0 ? 1 : 0, (total / peak()) * 120);
          const directPart = total > 0 ? (point.direct.down / total) * height : 0;
          return (
            <View
              style={{
                flex: 1,
                height,
                flexDirection: "column",
                justifyContent: "flex-end",
                borderRadius: 1,
                overflow: "hidden",
              }}
            >
              <View style={{ height: height - directPart, backgroundColor: hue.proxy() }} />
              <View style={{ height: directPart, backgroundColor: hue.direct() }} />
            </View>
          );
        })}
        {idle() ? (
          <View
            style={{
              position: "absolute",
              left: 0,
              right: 0,
              top: 0,
              bottom: 0,
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            <Text style={{ fontSize: 11, color: ink.muted() }}>
              还没有流量：启动隧道后这里每秒画一根柱子
            </Text>
          </View>
        ) : null}
      </View>
      <View style={{ flexDirection: "row", gap: 24 }}>
        <Metric label="分流到直连" value={`${props.snap().split.direct}`} color={hue.direct()} />
        <Metric label="分流到代理" value={`${props.snap().split.proxy}`} color={hue.proxy()} />
        <Metric label="判定失败" value={`${props.snap().split.failed}`} color={hue.danger()} />
        <Caption>{props.snap().split.running ? "分流器运行中" : "分流器未运行"}</Caption>
      </View>
    </Card>
  );
}
