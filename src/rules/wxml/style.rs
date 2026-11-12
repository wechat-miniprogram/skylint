//! 样式事实：从级联结果推出本工具反复要问的几个问题。
//!
//! 只放「多个规则都会问」的判定。单条规则自己的判据留在那条规则里。

use crate::input::wxml::WxmlElement;
use crate::matcher::Tree;
use crate::project::Project;

use super::cascade::resolved_property;

/// 元素是否脱离文档流（`position: absolute | fixed`）。
pub(crate) fn is_out_of_flow(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "position").is_some_and(|resolved| {
        matches!(
            resolved.value.trim().to_ascii_lowercase().as_str(),
            "fixed" | "absolute"
        )
    })
}

/// 元素的 `width` 是否本来就由内容决定，`fit-content` 被丢弃也不改变渲染。
///
/// 两种情况：自身脱离文档流（`position: absolute | fixed`），或父级是行向 flex 容器
/// 且自己不参与拉伸——`flex-basis: auto` 下主轴尺寸就是内容尺寸，与 `fit-content`
/// 同解。写了 `flex` / `flex-grow` 就不算：那会让元素按剩余空间拉伸。
///
/// 方向缺省按引擎的 `ApplyInitial` 走：普通模式下是 `column`
/// （`skyline/css/css_properties/flex_direction.cc:39-48`），列向容器里 `width` 是
/// 交叉轴、`auto` 会被拉伸；开了 `defaultDisplayBlock` 才回到 `row`——与
/// `scroll_view_height::flex_axis` 同一口径。那个开关同时让没写 `display` 的父级
/// 变成 `block`（`flex_container` 返回 false），`width: auto` 撑满父级，同样不按
/// 内容收缩。
pub(crate) fn width_follows_content(
    project: &Project,
    tree: &Tree<'_>,
    element: &WxmlElement,
) -> bool {
    if is_out_of_flow(project, tree, element) {
        return true;
    }
    let Some(parent) = tree.parent_of(element) else {
        return false;
    };
    if !flex_container(project, tree, parent) {
        return false;
    }
    let default_block = project.scope_of(tree.path()).map_or_else(
        || project.app_display_block(),
        |scope| project.renders_as_block(&scope),
    );
    let row = resolved_property(project, tree, parent, "flex-direction").map_or(
        default_block,
        |resolved| {
            matches!(
                resolved.value.trim().to_ascii_lowercase().as_str(),
                "row" | "row-reverse"
            )
        },
    );
    row && ["flex", "flex-grow"]
        .iter()
        .all(|name| resolved_property(project, tree, element, name).is_none())
}

/// 元素的 `display` 是否算成 flex 容器（取级联最后一个近似值）。
///
/// 与 [`display_values`] 的区别：这里要的是「最终算出哪个值」，同一元素命中
/// 多份互相矛盾的 `display` 时按样式表顺序取最后一个，让各规则口径一致。
pub(crate) fn flex_container(project: &Project, tree: &Tree<'_>, element: &WxmlElement) -> bool {
    resolved_property(project, tree, element, "display")
        .is_some_and(|resolved| is_flex_value(&resolved.value))
}

/// `display: flex` / `inline-flex`：文本与元素在 WebView 与 Skyline 下都是 flex item。
pub(crate) fn is_flex_value(value: &str) -> bool {
    let value = value.trim();
    value.eq_ignore_ascii_case("flex") || value.eq_ignore_ascii_case("inline-flex")
}

#[cfg(test)]
mod tests {
    use super::width_follows_content;
    use crate::matcher::Tree;
    use crate::rules::testutil::Fixture;
    use crate::rules::wxml::load_doc;

    /// `width_follows_content` 看的是 fixture 里 `class="x"` 的那个元素。
    fn follows_content_in(fixture: Fixture) -> bool {
        let project = fixture.project();
        let path = project.pages[0].files.wxml.clone().unwrap();
        let doc = load_doc(&project, &path).unwrap();
        let tree = Tree::new(&doc);
        let element = doc
            .elements()
            .find(|element| {
                element.attributes.iter().any(|attribute| {
                    attribute.name == "class" && attribute.value.as_deref() == Some("x")
                })
            })
            .expect("fixture 里没有 class=x 的元素");
        width_follows_content(&project, &tree, element)
    }

    fn follows_content(name: &str, wxml: &str, wxss: &str) -> bool {
        follows_content_in(Fixture::page(name, wxml, wxss))
    }

    fn default_block_fixture(name: &str, wxml: &str, wxss: &str) -> Fixture {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        fixture
    }

    /// Skyline 的 `flex-direction` 初始值是 `column`（`flex_direction.cc:39-48`）：
    /// `display: flex` 不带方向时 `width` 落在交叉轴、`auto` 会被拉伸，与
    /// `fit-content` 不同解，不能豁免。
    #[test]
    fn width_follows_content_defaults_to_column() {
        assert!(
            !follows_content(
                "wfc-default-column",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex }\n.x { width: fit-content }",
            ),
            "缺省方向是 column，应判为不按内容收缩"
        );
        assert!(
            !follows_content(
                "wfc-explicit-column",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex; flex-direction: column }\n.x { width: fit-content }",
            ),
            "显式 column 同样在交叉轴"
        );
        assert!(
            follows_content(
                "wfc-explicit-row",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex; flex-direction: row }\n.x { width: fit-content }",
            ),
            "行向主轴上的 auto 才等于内容宽"
        );
    }

    /// `defaultDisplayBlock: true` 时没写 `display` 的父级是 block，不是 flex：
    /// `width: auto` 撑满父级，同样不按内容收缩。
    #[test]
    fn width_follows_content_sees_a_block_parent_under_default_display_block() {
        assert!(
            !follows_content_in(default_block_fixture(
                "wfc-default-block",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { }\n.x { width: fit-content }",
            )),
            "block 父级下 width: auto 是拉伸"
        );
    }

    /// 同一个开关把缺省 `flex-direction` 拉回 row（`FlexDirection::ApplyInitial`）：
    /// 父级显式 `display: flex` 时仍是行向主轴，按内容收缩。
    #[test]
    fn width_follows_content_defaults_to_row_under_default_display_block() {
        assert!(
            follows_content_in(default_block_fixture(
                "wfc-default-block-row",
                r#"<view class="p"><view class="x"/></view>"#,
                ".p { display: flex }\n.x { width: fit-content }",
            )),
            "defaultDisplayBlock 下缺省方向是 row"
        );
    }
}
