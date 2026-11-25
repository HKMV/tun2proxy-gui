//! TSX 入口。
//!
//! 只有这一个文件放在仓库根：nsh 从 `nsh.json` 同级的入口解析 `tun2proxy/native`，
//! 入口放到 `src/` 下面就找不到 Rust 那一套了（实测报
//! `could not load module 'tun2proxy/native'`）。界面本身全在 `src/` 里：
//!
//! ```text
//! src/App.tsx            整窗骨架 + 导航
//! src/app/api.ts         后端接口（唯一 import tun2proxy/native 的地方）
//! src/app/theme.ts       颜色语义 + 数字格式化
//! src/app/components.tsx 卡片、胶囊、按钮
//! ```
//!
//! 启动：`nsh run App.tsx`（QuickJS，TSX 热更新）。

import { mount } from "nsh";
import { App } from "./src/App";

mount(App);
