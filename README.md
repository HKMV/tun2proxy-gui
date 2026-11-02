# tun2proxy GUI

[English](README.en.md) | **中文**

tun2proxy 的跨平台桌面图形前端，用 Rust 编写。界面基于
[GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui)（Zed 的 GPU 渲染
UI 框架）和 gpuikit（shadcn 风格的 GPUI 组件库），不依赖 WebView；配色、圆角、
控件尺度全部取自 gpuikit 自带的主题（默认 Gruvbox 深色）。

界面层级与交互参考了 [Paper-Dragon/tun2proxy-gui](https://github.com/Paper-Dragon/tun2proxy-gui)，
并按「多套代理配置 + 一键启停」重做了布局。

![仪表盘](assets/screenshots/dashboard-zh.png)

*中文界面（仪表盘）：左侧常驻运行状态与启停按钮，右侧是当前配置、运行参数与实时日志。*

## 功能

- **多套代理配置**：保存任意多套配置（家里 / 公司 / 临时节点），列表里一键切换，
  支持新建、复制、删除、改名；启动时使用当前选中的那套。
- **一键启停**：状态常驻左侧边栏（运行状态 + 启动/停止按钮），随时能看到是否在运行。
- **自动管理 tun2proxy**：首次运行点「下载并安装」，程序会从 GitHub Release
  下载对应平台的官方二进制、校验 SHA-256、解压到自己的数据目录；之后可随时
  「检查更新」并原地替换（不必重新发布本程序）。
- **完整参数覆盖**：上游代理（`socks5://` / `socks4://` / `http://`）、DNS 策略
  （直连 / 虚拟 IP / 走 TCP）、DNS 服务器、虚拟地址池、绕过网段、TUN 名称、
  MTU、TCP/UDP 超时、最大会话数、IPv6、自动配置路由、UDPGW、日志级别。
- **实时日志**：子进程的 stdout/stderr 直接显示在界面里，可一键清空。
- **代理连通性检测**：填完上游地址点一下「检测」，程序自己建立 TCP 连接并按
  协议走一次握手（SOCKS5 问候 / HTTP `CONNECT`），把「端口没在监听」「代理
  要求认证」「连上了但协议不对」直接写在那一行里——不用等启动失败再猜。
- **启动前自动预检**：点「启动」时先按同一套握手探一次上游代理，连不通就直接
  弹窗说明原因并**不启动**内核，而不是拉起一个注定失败的 tun2proxy 让用户自己
  看日志。
- **中英文界面**：设置里可选「跟随系统 / 中文 / English」，切换立即生效。
  代码里每条文案都是 `t("中文", "English")`，没有单独的翻译文件要维护。
- **实时流量折线图**：仪表盘首屏卡右侧显示最近一分钟的下行/上行速率。
- **系统托盘**：托盘图标常驻，提示文字跟着运行状态变；右键菜单可启动 / 停止 /
  显示窗口 / 退出，双击图标唤回窗口。
- **自动提权**：Windows 上检测到没有管理员权限时，询问后用 UAC 重新拉起自己；
  macOS/Linux 需要 root 时会给出提示（Linux 可开启「免 root」用用户命名空间运行）。

## 目录结构

```
src/
├─ main.rs         启动流程：权限 → DPI → 配置 → 引擎 → 窗口
├─ core/           与界面无关的内核（不依赖 GUI 框架，可单独测试）
│  ├─ mod.rs       模块说明与再导出
│  ├─ paths.rs     配置 / 下载 / 日志的存放位置
│  ├─ config.rs    多套代理配置的读写、校验、转成命令行参数
│  ├─ binary.rs    下载 / 校验 / 解压 tun2proxy 官方二进制
│  ├─ engine.rs    拉起子进程、收日志、维护状态与更新进度
│  ├─ elevate.rs   管理员 / root 权限检测与 Windows 自动提权
│  ├─ probe.rs     上游代理连通性检测（TCP + SOCKS5 / HTTP 握手）
│  ├─ traffic.rs   采样子进程的收发字节数（Windows IO 计数器 / /proc）
│  ├─ tray.rs      系统托盘图标（Windows：独立消息窗口 + Shell_NotifyIcon）
│  ├─ window.rs    主窗口查找与显隐（托盘唤回、最小化到托盘）
│  ├─ icon.rs      程序图标的多尺寸位图（窗口 / 托盘共用一份数据）
│  ├─ i18n.rs      中英文文案与语言设置
│  └─ platform.rs  进程级初始化（Windows DPI 感知）
└─ ui/             界面层（GPUI + gpuikit）
   ├─ state.rs     应用状态、消息枚举、数字输入草稿
   ├─ view.rs      根视图：左侧边栏 + 右侧内容区
   ├─ theme.rs     装载 gpuikit 主题（可换肤）与字号常量
   ├─ sidebar.rs   左侧边栏（标题、运行状态、导航、启动/停止）
   ├─ pages.rs     右侧内容区：细顶栏 + 各页的分组组合
   ├─ cards.rs     各页的分组与行
   ├─ widgets.rs   可复用的文字样式与小部件（首屏卡、分组、设置行、胶囊）
   ├─ chart.rs     实时流量折线图（gpui canvas）
   └─ update.rs    消息处理（唯一修改状态的地方）
assets/
├─ icon.ico       程序图标（多尺寸，编进 exe 资源）
└─ icon.png       图标原图（512×512，带透明通道）
build.rs         把 app.rc（图标 + 版本信息）编进 Windows exe
```

### 程序图标

图标的唯一来源是 exe 资源段：`app.rc` + `build.rs`（`embed-resource`）把
`assets/icon.ico` 编进 exe，资源管理器、任务栏、窗口标题栏都从这里取——
GPUI 不做运行时的图标加载，因此也没有「把 ico 落到数据目录」那条路径。

图标本身是多尺寸 ico（16/24/32/48/64/128/256），系统按需挑一档，任务栏小图标
不会糊。换图标只需替掉 `assets/icon.ico` 后重新 `cargo build`。

### 界面结构

```
┌──────────────┬──────────────────────────────────────┐
│ Tun2Proxy    │ 仪表盘            服务状态、当前配置……  │ ← 细顶栏（约 45dp）
│ [v0.0.1]     │──────────────────────────────────────│ ← 1dp 发丝线
│              │  ╭──────────────────────────────────╮│
│ ▸ 仪表盘      │  │ [未运行]              [  启动  ] ││
│ ▸ 代理配置    │  │ Tun2Proxy 未启动                  ││
│ ▸ 网络与绕过  │  │ 一句说明……                        ││
│ ▸ 程序与日志  │  ╰──────────────────────────────────╯│
│              │  当前配置                             │ ← 分组标题（卡片外）
│              │  ╭──────────────────────────────────╮│
│              │  │ [▤] 配置文件          [ 默认  ⌄] ││ ← 行
│              │  │     隧道用这套配置启动……          ││
│              │  ├──────────────────────────────────┤│ ← 左右缩进的发丝线
│              │  │ [⚙] 上游代理    socks5://…  [✎]  ││
│              │  ╰──────────────────────────────────╯│
│ ● 服务状态    │                                      │
│ [  启动  ]    │  可滚动的分组列表（溢出时才出滚动条）    │
└──────────────┴──────────────────────────────────────┘
```

视觉分三层：侧边栏 `surface`、内容区 `bg`、卡片 `surface` + 1dp 描边 + 8dp 圆角。
三档颜色都取自 gpuikit 的主题语义色，所以换一套主题（见下文「主题」）整窗一起变，
没有一处写死的十六进制。

设置项的版式：**分组标题在卡片外**（加粗小字），卡片里是一行一个设置项——左边图标、
中间「标题 + 灰说明」、右边控件或值，行与行之间夹一条左右缩进 12dp 的发丝线。
同一类设置收进一张卡，比「一卡一项」密得多，也少了满屏的框。

顶栏只有一行（页面名 + 右侧一句说明 + 底部发丝线），不再占两行高度；
仪表盘的首屏卡片（状态胶囊 + 大标题 + 主操作）放在内容流的第一位。
右侧只有分组列表是滚动区，顶栏始终可见。

配置详情页（代理配置 → 铅笔）的顶栏右侧换成「保存到磁盘」主按钮：字段多到
要滚动时，保存不能跟着内容跑出屏幕；返回按钮在标题左边，回到配置列表。

四个页面各管一组设置，切换页面不会丢失任何已填内容（状态都在 `App` 里）。
原来的「高级选项」页已拆掉：日志级别进「程序与日志」，UDP 转发（UDPGW）进
「网络与绕过」——一个只有三个开关的页面只多一次点击，不省事。

内核层只通过「重绘回调」通知界面刷新（`Engine::set_rebuild_hook`），因此把界面
换成别的框架时，`src/core` 可以原样复用。

## 构建

```bash
cargo build --release
```

产物在 `target/release/tun2proxy-gui`（Windows 为 `tun2proxy-gui.exe`）。

依赖说明：

- Windows：无需额外依赖。
- Linux：GPUI 用 Vulkan 渲染、Wayland / X11 窗口，需要对应的开发库与
  `pkg-config`；包名以 gpui 仓库的 Linux 说明为准。
- macOS：需要 Xcode 命令行工具。

### 发布

打 tag 即自动编译三平台产物并发布到 GitHub Release（见
`.github/workflows/release.yml`）：

```bash
# 先把 Cargo.toml 的 version 改好并提交，tag 必须与它一致（CI 会校验）
git tag v0.2.0 && git push origin v0.2.0
```

产物：`tun2proxy-gui-<tag>-<target>.zip`（Windows）、`.tar.gz`（Linux /
macOS，macOS 里是 `Tun2Proxy GUI.app`），外加一份 `SHA256SUMS.txt`。
也可以在 Actions 页面手动 Run workflow，填一个已存在的 tag 重跑发布。

## 网络与代理

下载/更新 tun2proxy 走系统 `curl`（其次 `wget`、PowerShell），因此**认标准的
`http_proxy` / `https_proxy` 环境变量**：

```bash
# Windows（cmd）
set http_proxy=http://127.0.0.1:7890
set https_proxy=http://127.0.0.1:7890
# Windows（PowerShell）
$env:http_proxy="http://127.0.0.1:7890"; $env:https_proxy="http://127.0.0.1:7890"
```

设好后从同一个终端启动本程序即可。下载失败时界面会显示 `curl` 的具体原因
（`28` 是超时、`403` 多半是代理拦了 GitHub），也可以手动下载 Release 包，
把里面的文件解压到数据目录的 `bin/` 下。

## 数据目录

| 平台 | 路径 |
|------|------|
| Windows | `%APPDATA%\tun2proxy-gui\` |
| macOS | `~/Library/Application Support/tun2proxy-gui/` |
| Linux | `~/.config/tun2proxy-gui/`（遵循 `$XDG_CONFIG_HOME`） |

目录内容：`config.json`（全部配置）、`bin/`（下载的 tun2proxy 与 wintun.dll 等）。
设置环境变量 `TUN2PROXY_GUI_HOME` 可以把整个目录搬到别处（便携版）。

`config.json` 形如：

```json
{
  "profiles": [
    { "name": "家庭 SOCKS5", "proxy_url": "socks5://127.0.0.1:1080" },
    { "name": "公司 HTTP", "proxy_url": "http://10.0.0.5:8080", "dns": "virtual" }
  ],
  "active": 0,
  "auto_check_update": true,
  "release_repo": "tun2proxy/tun2proxy"
}
```

缺字段会按默认值补齐，因此手写配置文件时只需要写关心的几项。

## 运行须知

- tun2proxy 需要创建虚拟网卡并改系统路由，**必须有管理员 / root 权限**。
- 本程序不把 tun2proxy 编进自身，也不链接它的库：隧道完全由官方二进制完成，
  这样 GUI 的发布节奏不必跟着 tun2proxy 走。
- 更新前请先停止服务（Windows 上无法替换正在运行的 exe，程序会自动拦截）。
- 点「启动」时会先探一次上游代理（和「检测」按钮同一套握手），连不通就不启动，
  弹窗里直接写原因。

## 托盘与关闭

- 关窗口时弹确认框，**默认按钮是「最小化到托盘」**（回车即可）：隧道不断，
  点托盘图标双击可恢复窗口。想真退出就点右上叉或按 `Esc`。
- 设置里关掉「关闭时询问」后，关窗口直接藏进托盘，不再问。
- 托盘右键菜单：显示窗口 / 启动 / 停止 / 退出；提示文字随状态变。
- 只支持 Windows：其它平台的托盘需要各自的系统 API，目前没有实现，
  「最小化到托盘」在那些平台上会自动退化为直接退出（不会把窗口藏到找不回来）。

## 语言

设置页可选「跟随系统 / 中文 / English」，切换后立即生效并落盘（`language`
字段）。文案写在代码里（`t("中文", "English")`），没有单独的翻译资源文件。

## 主题

配色、圆角、控件高度全部来自 gpuikit 的 `Themeable` 语义色与 `ControlScale`，
本程序不自己配色。默认是 Catppuccin Mocha（冷灰底 + 蓝色强调色），设环境变量
可以换：

```powershell
set TUN2PROXY_GUI_THEME=light    # Catppuccin Latte（浅色）
set TUN2PROXY_GUI_THEME=frappe   # Catppuccin Frappe
set TUN2PROXY_GUI_THEME=gruvbox  # gpuikit 的招牌 Gruvbox 深色
```

认不出来的名字回落到默认主题（不报错，免得环境变量写错就起不来）。
换主题时 Windows 原生标题栏的深浅也会跟着切（`core::platform::set_dark_titlebar`）。

为什么不用 gpuikit 的默认主题：Gruvbox 是暖褐底 + 亮绿强调色，那套色板适合终端
和编辑器，放在满屏中文小字的设置页上有两个问题——暖褐底色让文字发黄显脏，绿色
又同时占了侧栏选中、主按钮填充和图表三条线。`ui::theme::tune` 里做了三件事：

1. **重排 Catppuccin 的层次。** gpuikit 把 `surface` 映射成 mantle（比 base 更暗），
   照原样画，卡片会沉进背景里。改成背景 base < 卡片 surface0 < 按钮 surface1。
2. **输入框改成「比卡片暗一档 + 更亮的描边」。** gpuikit 默认让输入框和卡片同色，
   只靠 1dp 描边区分，在满屏卡片里那个方框几乎看不见。
3. **深色主题的描边相对提亮一档**，1dp 的小控件才立得住。

主按钮不用强调色填充：强调色留给「选中 / 焦点 / 数据」，整块铺在按钮上会一直把
视线拽回来。主按钮靠对比度立起来——深色主题是浅底深字，浅色主题自动反转成深底
浅字，不用为两套主题各写一份。

## 托盘

托盘图标按服务状态换色：**未运行是灰的，运行中是绿的**。托盘区域只有 16~32px，
靠形状分不出在不在跑，颜色是最快的信号（鼠标悬停的提示文字也带着状态）。

两档都从 `assets/icon.ico` 那一份算出来，不额外维护图片：ico 里的帧是 PNG，
运行时解码、把有彩色像素的色相换成绿（未运行时去饱和压暗）、再编码回去造
`HICON`（`core::icon::recolor_pixel`）。白色图案是无彩色像素，保持不动——否则
图案和底色会糊成一片。

「正在启动」也算「在跑」：用户已经按了启动，图标这时候就该亮，等真正 `Running`
再变色会慢一拍。着色失败时退回原图，颜色不对是小事，图标消失是大事。

## 开发

```bash
cargo check        # 类型检查
cargo test         # 配置管理的单元测试
cargo build        # 调试构建
```

设置 `TUN2PROXY_GUI_NO_ELEVATE=1` 可跳过提权，直接以当前权限启动。

### 调试时怎么截图

用 `PrintWindow(hwnd, hdc, PW_RENDERFULLCONTENT)` 向窗口要画面，**不要**用
`CopyFromScreen`：调试时窗口常常被别的程序盖着，屏幕抓图会抓到上面那个程序，
看起来像「界面没更新」，实际是截错了对象。

窗口矩形用 `GetWindowRect` 会包含 Win11 的不可见阴影边框（左右各约 11px、
底部约 10px），按这个内缩裁一下，图里才不会混进桌面像素。点击可以直接
`PostMessage` 一个 `WM_LBUTTONDOWN` / `WM_LBUTTONUP` 到窗口句柄（坐标是客户区
坐标），不要求窗口在前台。

### 布局上的三个坑

1. **可换行的文字要 `flex_1 + min_w_0`**：不写的话，行内文字会按「最宽不可断词」
   测量，中文会被压成一列竖排的字。见 `src/ui/widgets.rs` 的 `setting_row`。
2. **自绘控件要取 `theme.control(size)`**：按钮、图标按钮、徽标的高度 / 圆角 /
   字号都来自 gpuikit 的 `ControlScale`，自己画的东西也取同一组数，摆在一行才
   不会一大一小。见 `src/ui/widgets.rs` 的 `primary_button`。
3. **卡片用单层描边**：`bg(surface) + border_1(border) + rounded(0.5rem)`，和
   gpuikit 的 `Card` 一致。别用「底色外面套一圈描边」的双层盒子——深色主题下
   那会露出一圈亮边。

## 许可

MIT，全文见 [LICENSE](LICENSE)。
