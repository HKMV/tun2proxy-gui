# gpuikit 0.9.0（本地副本 + 一处补丁）

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

## 升级时怎么做

1. 把新版本源码拷到本目录（`src/`、`assets/`、`Cargo.toml`、两个 LICENSE）；
2. 按上面 7 条重新打一遍（`git diff` 对着看最快）；
3. 如果新版上游已经给了 `Select::popup_width`，直接删掉本目录和
   `Cargo.toml` 里的 `[patch.crates-io]` 段，把依赖换回 crates.io。
