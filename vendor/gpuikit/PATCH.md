# gpuikit 0.9.0（本地副本 + 三处补丁）

这里是 `gpuikit` 0.9.0 的源码副本，来自 crates.io，通过 `Cargo.toml` 末尾的
`[patch.crates-io]` 指向本目录。除了下面列的补丁，其余文件与上游完全一致；
`LICENSE-MIT` / `LICENSE-APACHE` 是上游原文，随源码一起保留。

## 为什么打补丁

`Select` 的下拉弹层宽度由内容决定（`listbox.rs` 里只有 `min_w(120px)`），
触发框多宽它都不管，应用侧也没有任何旋钮：

- `Listbox` 是 `pub(crate)`，应用里拼不出一个同宽的替代品；
- `SelectState::render` 用 `deferred(anchored())` 直接把弹层挂上去，宽度
  一路传不进去。

结果是 220dp 的下拉框弹出一个 ~142dp 的列表，两者不像同一个控件。上游
0.9.0 是最新版，没有更高的版本可用。

## 补丁内容（相对上游 0.9.0）

`src/elements/select.rs`

1. `Select` / `SelectState` 各加一个 `popup_width: Option<Pixels>` 字段；
2. 新增构建方法 `Select::popup_width(width)`；
3. `toggle_listbox` 把 `self.popup_width` 透传给 `Listbox::build`。

`src/elements/listbox.rs`

4. `Listbox` 加 `width: Option<Pixels>` 字段，`build` 多一个同名参数；
5. `render` 里 `.when_some(self.width, |this, width| this.w(width))`，弹层按
   这个宽度画（行本来就会沿交叉轴拉伸，所以整行都是点击区）；
6. 行标签外面套一层 `div().min_w_0().truncate()`：弹层宽度定死之后，一个
   过长的选项名只能被截断，不能把弹层重新撑宽。

`src/elements/combobox.rs`

7. `Listbox::build` 的调用点补一个 `None` —— 组合框的弹层是过滤结果列表，
   本来就该按内容排版。

## 第二处补丁：`Dialog` 的「取消」与「随手关掉」分开

确认弹窗里，Esc、点弹层外面、标题栏 ✕ 三条路都走 `dismiss`，而上游的
`dismiss` 对确认弹窗一律当成「取消」。本应用「关闭后最小化？」那个弹窗的
取消答案是**退出程序**，于是一次误点就把整个程序关掉。

`src/elements/dialog.rs`

8. `Confirmation` 加 `on_dismiss` 字段（默认 `None`，行为与上游一致）；
9. 新增构建方法 `Dialog::on_dismiss(handler)`；
10. `dismiss` 改为：非确认弹窗照旧关闭；确认弹窗**有** `on_dismiss` 时只
    关闭并调用它（不发 `DialogCancelled`），没有时仍然走 `cancel`；
11. 标题栏 ✕ 的 `icon_button` 补 `.control_size(size)` —— 页脚按钮会跟随
    `ControlSize`，标题栏那颗原来是写死的 Medium，32dp 的界面上像缩水一圈。

## 第三处补丁：多行输入框的滚动量对外开放

gpui 的 `text_area` 只滚不画条，gpuikit 的 `Textarea` 也没有留画条的位置，
应用侧只能自己叠一条。但 `InputState` 把三个数都藏成了 `pub(crate)`：
可见高度、内容高度、偏移只能读不能写，画不出滑块、也拖不动。

`src/input/state.rs`

12. `content_height()` —— 公开 `total_content_height()`；
13. `visible_height()` —— 公开上次布局的 `available_height`；
14. `set_scroll_offset(offset, cx)` —— 夹到 `[0, 内容高 - 可见高]` 后写入并
    `notify`；滚轮与「跟随光标」那两处仍直接写字段（它们自己已经夹过了）。

应用侧的使用见 `src/ui/widgets.rs` 的 `input_scroll_bar`（绕过列表）。

## 升级时怎么做

1. 把新版本源码拷到本目录（`src/`、`assets/`、`Cargo.toml`、两个 LICENSE）；
2. 按上面 14 条重新打一遍（`git diff` 对着看最快）；
3. 如果新版上游已经给了 `Select::popup_width`、`Dialog::on_dismiss` 和这三个
   滚动量接口，直接删掉本目录和 `Cargo.toml` 里的 `[patch.crates-io]` 段，
   把依赖换回 crates.io。
