//! 布局探针：不开窗口，直接把视图按指定尺寸布局 / 绘制一遍。
//!
//! 「窗口拉宽拉窄会不会塌」「滚动条会不会出现」这类问题靠肉眼截图很难确认，
//! 但布局引擎本身是纯函数：给它不同的窗口尺寸，就能把这两件事变成断言。
//! 这里只做只读检查，不改任何界面代码。

use zsui::{Dpi, Rect, View, ViewLayoutCx, ViewNode, ViewPaintCx, WidgetId};

use crate::core::{Config, Engine};
use crate::ui::pages::content_scroll_id;
use crate::ui::state::{App, Msg, Page};
use crate::ui::widgets::LOG_SCROLL_ID;

/// 一个不依赖真实安装状态的 App。
fn test_app() -> App {
    let engine = Engine::new();
    let shared = engine.shared();
    App::new(Config::default(), engine, shared)
}

/// 按给定逻辑尺寸布局整棵视图。
fn layout_at(app: &App, width: f32, height: f32) -> ViewNode<Msg> {
    let mut node = crate::ui::view(app);
    let mut cx = ViewLayoutCx::new(
        Rect {
            x: 0,
            y: 0,
            width: width as i32,
            height: height as i32,
        },
        Dpi::standard(),
    );
    node.layout(&mut cx);
    node
}

/// 布局 + 绘制一遍（滚动条几何只在绘制阶段算出来）。
fn paint_at(app: &App, width: f32, height: f32) -> ViewNode<Msg> {
    let node = layout_at(app, width, height);
    let mut cx = ViewPaintCx::new(Dpi::standard());
    node.paint(&mut cx);
    node
}

/// 深度优先找指定 id 的节点。
fn find(node: &ViewNode<Msg>, id: WidgetId) -> Option<&ViewNode<Msg>> {
    if node.id == Some(id) {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

fn width_of(node: &ViewNode<Msg>, page: Page) -> i32 {
    find(node, content_scroll_id(page))
        .and_then(|n| n.bounds())
        .map(|b| b.width)
        .expect("内容区滚动节点应该存在并且被布局")
}

#[test]
fn content_area_is_bounded_by_the_window_height() {
    let app = test_app();
    let node = layout_at(&app, 900.0, 650.0);
    let bounds = find(&node, content_scroll_id(app.page))
        .and_then(|n| n.bounds())
        .expect("内容区滚动节点应该存在并且被布局");
    assert!(
        bounds.height < 650,
        "内容区高度必须被窗口约束，否则永远不会滚动：{bounds:?}"
    );
}

#[test]
fn content_scrollbar_appears_when_cards_overflow() {
    let app = test_app();
    let node = paint_at(&app, 900.0, 480.0);
    assert!(
        node.widget_scrollbar_layout(content_scroll_id(app.page))
            .is_some(),
        "窗口变矮、卡片溢出时必须画出滚动条"
    );
}

#[test]
fn cards_follow_the_window_width() {
    let app = test_app();
    let narrow = width_of(&layout_at(&app, 620.0, 650.0), app.page);
    let wide = width_of(&layout_at(&app, 1200.0, 650.0), app.page);
    assert!(
        wide > narrow + 400,
        "窗口变宽时内容区必须跟着变宽：narrow={narrow} wide={wide}"
    );
}

#[test]
fn log_pane_scrolls_on_its_own() {
    let mut app = test_app();
    app.page = Page::Program;
    {
        let mut shared = app.shared.lock().unwrap();
        for index in 0..300 {
            shared.logs.push_back(format!("line {index}"));
        }
    }
    let node = paint_at(&app, 900.0, 650.0);
    assert!(
        node.widget_scrollbar_layout(WidgetId::new(LOG_SCROLL_ID))
            .is_some(),
        "日志超出固定高度时应该出现自己的滚动条"
    );
}

/// 最小窗口下任何一页都不允许横向溢出。
///
/// ZSUI 不会把控件压到平台度量以下，装不下就裁剪——所以「窗口最小宽度」必须
/// 大于「最宽那一页的内在最小宽度」。探针里的文字度量是估算值，比真实
/// DirectWrite 量出来的更宽，所以这个断言过了，真实窗口一定不会切边。
#[test]
fn no_page_overflows_the_minimum_window() {
    for page in [
        Page::Dashboard,
        Page::Profiles,
        Page::Network,
        Page::Program,
    ] {
        let mut app = test_app();
        app.page = page;
        let node = layout_at(&app, crate::ui::MIN_WINDOW_WIDTH, 700.0);
        let content = find(&node, content_scroll_id(page))
            .and_then(|n| n.bounds())
            .expect("内容区应该被布局");
        assert!(
            content.x + content.width <= crate::ui::MIN_WINDOW_WIDTH as i32,
            "{page:?} 页在最小窗口宽度下横向溢出了：内容区 {content:?}"
        );
    }
}

/// 卡片必须填满内容区的可用宽度（不能缩在左边）。
#[test]
fn cards_fill_the_available_width() {
    let mut app = test_app();
    app.page = Page::Profiles;
    let node = layout_at(&app, 1200.0, 700.0);
    let content = find(&node, content_scroll_id(app.page))
        .and_then(|n| n.bounds())
        .expect("内容区应该被布局");
    // 内容区里的第一个卡片容器。
    let scroll = find(&node, content_scroll_id(app.page)).unwrap();
    let card = scroll.children.first().and_then(|n| n.bounds()).unwrap();
    assert!(
        card.width >= content.width - 64,
        "卡片应该跟着内容区变宽：card={card:?} content={content:?}"
    );
}
/// 任何节点都不允许超出窗口右边界。
///
/// 上一条测的是「内容区本身」不越界，这一条管「内容区里的东西」：
/// ZSUI 对控件的硬性最小宽度不设商量，父级窄了就直接裁掉——
/// 表现在界面上就是按钮被切掉半个、输入框右半边消失。这种事故在截图里
/// 很容易被忽略（看起来像「贴边」），但用户点不到被切掉的控件。
#[test]
fn no_node_exceeds_the_window_width() {
    for width in [crate::ui::MIN_WINDOW_WIDTH, 1200.0] {
        for page in [
            Page::Dashboard,
            Page::Profiles,
            Page::Network,
            Page::Program,
        ] {
            let mut app = test_app();
            app.page = page;
            let node = layout_at(&app, width, 700.0);
            let mut worst: Option<(i32, String)> = None;
            collect_right_overflow(&node, width as i32, &mut worst);
            assert!(
                worst.is_none(),
                "窗口宽 {width} 时 {page:?} 页有控件被裁切：{worst:?}"
            );
        }
    }
}

/// 递归找出「右边界超出窗口」的节点，记录超出最多的那个。
fn collect_right_overflow<M>(node: &ViewNode<M>, limit: i32, worst: &mut Option<(i32, String)>) {
    if let Some(bounds) = node.bounds() {
        let right = bounds.x + bounds.width;
        if right > limit {
            let over = right - limit;
            // `ViewNodeKind` 没有 `Debug`（它带着消息类型），只报 id 与判别值。
            let name = match node.id {
                Some(id) => format!("id {id:?} 右边界 {right}"),
                None => format!("{:?} 右边界 {right}", std::mem::discriminant(&node.kind)),
            };
            if worst.as_ref().map(|(w, _)| over > *w).unwrap_or(true) {
                *worst = Some((over, name));
            }
        }
    }
    for child in &node.children {
        collect_right_overflow(child, limit, worst);
    }
}

/// 侧边栏必须保持 160（展开）/ 48（收起）。
///
/// 它被任何宽控件撑开，导航项的白底就会比文字宽出一大截——收起按钮曾经因此
/// 被搬出侧边栏。现在它单独占一行，不再参与宽度竞争，这条断言把这件事钉住。
#[test]
fn sidebar_keeps_its_width() {
    use crate::ui::sidebar::{RAIL_WIDTH, SIDEBAR_WIDTH};

    for collapsed in [false, true] {
        let mut app = test_app();
        app.sidebar_collapsed = collapsed;
        let node = layout_at(&app, crate::ui::MIN_WINDOW_WIDTH, 700.0);
        // 根 = row([侧边栏, 内容区])；侧边栏 = row([面板, 发丝线])。
        let pane = node.children[0].children[0]
            .bounds()
            .expect("侧边栏应该被布局");
        let expected = if collapsed {
            RAIL_WIDTH as i32
        } else {
            SIDEBAR_WIDTH as i32
        };
        assert_eq!(
            pane.width, expected,
            "collapsed={collapsed} 时侧边栏宽度是 {}，应为 {expected}",
            pane.width
        );
    }
}
