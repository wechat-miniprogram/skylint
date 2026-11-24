//! `skyline/native-layout-ignored`：没有 CSS 盒的内置组件上，布局类声明全部不生效。
//!
//! 与 `native-padding` 同一条机制（共享 [`capability`] 能力表）：这些组件在 Skyline 里由原生
//! widget 排布，`InflateNaked()` 连布局节点都不建，
//! 所以 `width` / `height` / `margin` / `display` / `flex-*` / `position` / offsets /
//! `align-content` / `order` 都不会进布局。
//! list/grid 四件套还会在 style builder 的白名单里被挡掉，只剩背景色；sticky / nested-scroll 系列连
//! computed style 都不建。
//!
//! `padding` / `border-width` / `gap` 不在这里报：它们的后果与改法都不同，
//! 由 `native-padding` 单独负责。
//!
//! WebView 里这些组件的内部是普通 DOM，布局属性照常生效，属于渲染器差异。

use crate::project::Project;
use crate::rules::wxml::tags::{Capability, CssBox, capability};
use crate::rules::wxml::{Reach, element_diagnostic, for_each_element};
use crate::rules::wxss::natives;
use crate::rules::{Rule, RuleMeta, VisualCase, VisualExpect};
use crate::{Diagnostic, Severity};

pub struct NativeLayoutIgnored;

impl NativeLayoutIgnored {
    pub const ID: &'static str = "skyline/native-layout-ignored";

    /// 会进布局的声明。`padding` / `border-width` / `gap` 已经由 `native-padding` 负责，
    /// 这里不重复报。
    ///
    /// offsets / `align-content` / `order` 与上面几项同源：`CssBox::None` 连布局节点都不建，
    /// 它们不会进布局。没有单独的视觉用例，依据是能力表里 `None` 的判据。
    const PROPERTIES: [&'static str; 29] = [
        "width",
        "height",
        "min-width",
        "min-height",
        "max-width",
        "max-height",
        "margin",
        "margin-top",
        "margin-right",
        "margin-bottom",
        "margin-left",
        "display",
        "position",
        "top",
        "right",
        "bottom",
        "left",
        "flex",
        "flex-direction",
        "flex-wrap",
        "flex-grow",
        "flex-shrink",
        "flex-basis",
        "justify-content",
        "align-items",
        "align-self",
        "align-content",
        "order",
        "overflow",
    ];

    fn message(capability: &Capability, properties: &[&str]) -> String {
        let list = properties
            .iter()
            .map(|property| format!("`{property}`"))
            .collect::<Vec<_>>()
            .join("、");
        format!(
            "`{}` 在 Skyline 下没有 CSS 盒（由原生 widget 排布）：{list} 不会进布局，WebView 下生效",
            capability.tag
        )
    }

    fn help(capability: &Capability) -> &'static str {
        match capability.css_box {
            // 背景色是唯一还能画出来的样式。
            CssBox::None if capability.paints_background => {
                "这层只负责滚动/布局容器：尺寸与间距交给组件自己的属性，或在里面包一层普通 `view` 承担布局；这个组件上只有 `background-color` 还会绘制"
            }
            CssBox::None => {
                "这个组件不构建样式，所有 CSS 都不生效：布局与外观都放到内层普通 `view` 上"
            }
            CssBox::Full | CssBox::BorderBoxTail => unreachable!("只有 CssBox::None 进这条规则"),
        }
    }
}

/// 视觉用例：`sticky-section` 上的尺寸与间距在 WebView 生效，Skyline 因为没有 CSS 盒而忽略。
///
/// 两端的 `scroll-view type="custom"` 只用来满足该组件的使用条件（必须放在 custom 模式的
/// scroll-view 里）并给子树一个可见高度：Skyline 下 scroll-view 没有高度时子树不绘制。
/// `.sect` 的 `display`、300×120、`margin-top` 与子 `.row` 的高度才是要比的点。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap">
  <view class="head">上方的标题</view>
  <scroll-view type="custom" class="scroll">
    <sticky-section class="sect">
      <view class="row" id="target">列表项</view>
    </sticky-section>
  </scroll-view>
</view>"#,
    wxss: ".wrap { background: #fff; }\n.head { height: 30px; background: #4a4a68; color: #fff; }\n.scroll { height: 200px; }\n.sect { display: block; width: 300px; height: 120px; margin-top: 16px; background: #d9f2d9; }\n.row { height: 40px; background: #9ad0f5; }\n",
    js: None,
    expect: VisualExpect::Differ,
    note: "`sticky-section` 在 Skyline 下没有 CSS 盒：声明的 300×120、`margin-top` 与 `display` 都不进布局，子节点 `.row` 作为独立布局根紧贴标题渲染；WebView 按普通 DOM 排出 300×120 的盒子。该组件连 `background-color` 也不画（`#d9f2d9` 在 Skyline 侧面积 0、WebView 侧 96000 device px），这是并集差异里最大的一块",
    target: "#target",
}];

impl Rule for NativeLayoutIgnored {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "没有 CSS 盒的内置组件上，布局类声明在 Skyline 下不生效",
        }
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();

        for_each_element(project, self.target(), |site| {
            if site.reach != Reach::Skyline {
                return;
            }
            let (doc, tree, element) = (site.doc, site.tree, site.element);
            let reach = site.reach;
            let Some(capability) = capability(&element.tag) else {
                return;
            };
            if !capability.css_box.ignores_layout() {
                return;
            }
            // 判不了的选择器可能改写结果：只要出现就不报（宁可漏报）。
            let Some(declared) =
                natives::declared(project, tree, element, &Self::PROPERTIES, effective)
            else {
                return;
            };
            if declared.is_empty() {
                return;
            }
            out.push(
                element_diagnostic(
                    Self::ID,
                    doc,
                    element,
                    doc.path(),
                    Severity::Warning,
                    Self::message(capability, &declared),
                )
                .help(reach.help(Self::help(capability))),
            );
        });
        out
    }
}

/// 这条声明是否真的想改变布局。
///
/// 判不了的（`{{…}}`、`var(…)`）与各属性的初始值都算「没写」，宁可漏报。
fn effective(property: &str, value: &str) -> bool {
    let value = value
        .trim()
        .trim_end_matches("!important")
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() || value.contains("var(") || value.contains("{{") {
        return false;
    }
    let zero = |value: &str| {
        value.split_whitespace().all(|token| {
            token.trim_end_matches(|c: char| c.is_ascii_alphabetic() || c == '%') == "0"
        })
    };
    !match property {
        "width" | "height" | "flex-basis" => value == "auto",
        "min-width" | "min-height" => value == "auto" || zero(&value),
        "max-width" | "max-height" => value == "none",
        "margin" | "margin-top" | "margin-right" | "margin-bottom" | "margin-left" => zero(&value),
        "top" | "right" | "bottom" | "left" => value == "auto",
        "position" => value == "static",
        "flex-direction" => value == "row",
        "flex-wrap" => value == "nowrap",
        "flex-grow" => value == "0",
        "flex-shrink" => value == "1",
        "align-items" => matches!(value.as_str(), "normal" | "stretch"),
        "align-self" => matches!(value.as_str(), "auto" | "normal" | "stretch"),
        "align-content" => matches!(value.as_str(), "normal" | "stretch"),
        "order" => value == "0",
        "overflow" => value == "visible",
        // `display` / `flex` / `justify-content`：初始值取决于 defaultDisplayBlock 与
        // 父级类型，静态判不准，写了就报。
        //
        // 待确认、先不报：`float` / `box-sizing` / `inset` 的盒内行为没有实测；
        // `display: none` 是否仍由 widget 树生效也未确认，现在按「写了就报」保持现状。
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::Diagnostic;
    use crate::rules::Rule;
    use crate::rules::testutil::wxss::{self, Fixture};

    use super::NativeLayoutIgnored;

    const SKYLINE_APP: &str = r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#;

    fn check(name: &str, wxml: &str, wxss: &str) -> Vec<Diagnostic> {
        wxss::check(&NativeLayoutIgnored, name, wxml, wxss)
    }

    #[test]
    fn visual_case_sticky_section_layout_is_reported() {
        let diagnostics = check(
            "layout-visual",
            r#"<view class="wrap"><view class="head">标题</view><scroll-view type="custom"><sticky-section class="sect"><view class="row">列表项</view></sticky-section></scroll-view></view>"#,
            ".head { height: 30px; }\n.sect { display: block; width: 300px; height: 120px; margin-top: 16px; }\n.row { height: 40px; }",
        );
        assert!(!diagnostics.is_empty(), "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`sticky-section`"));
    }

    #[test]
    fn layout_properties_on_list_view_are_reported() {
        let diagnostics = check(
            "layout-list-view",
            r#"<list-view class="list"><view class="row"/></list-view>"#,
            r#".list { display: flex; margin: 8px; background-color: #fff; }"#,
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`list-view`"));
        assert!(diagnostics[0].message.contains("`display`"));
        assert!(diagnostics[0].message.contains("`margin`"));
    }

    #[test]
    fn only_no_css_box_components_are_reported() {
        // scroll-view 有 yoga 节点，width/height 是有效的；swiper-item 的子节点更是普通布局。
        let diagnostics = check(
            "layout-scroll-view",
            r#"<scroll-view class="list"><view class="row"/></scroll-view><swiper><swiper-item class="page"/></swiper>"#,
            r#".list { width: 100%; height: 300px; } .page { display: flex; }"#,
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn defaults_and_dynamic_values_are_ignored() {
        let diagnostics = check(
            "layout-defaults",
            r#"<sticky-section class="s"><view class="row"/></sticky-section>"#,
            r#".s { width: auto; margin: 0; display: none; max-width: var(--w); }"#,
        );
        // `display: none` 不是初始值，会报；`width: auto` / `margin: 0` / `var()` 不报。
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("`display`"));
        assert!(!diagnostics[0].message.contains("`width`"));
    }

    #[test]
    fn offsets_align_content_and_order_are_reported() {
        let diagnostics = check(
            "layout-box-properties",
            r#"<sticky-section class="s"><view class="row"/></sticky-section>"#,
            ".s { position: relative; top: 8px; left: 4px; align-content: center; order: 2; }",
        );
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        for property in ["top", "left", "align-content", "order"] {
            assert!(
                diagnostics[0].message.contains(&format!("`{property}`")),
                "缺少 {property}：{}",
                diagnostics[0].message
            );
        }
    }

    #[test]
    fn initial_offsets_align_content_and_order_are_ignored() {
        let diagnostics = check(
            "layout-box-defaults",
            r#"<sticky-section class="s"><view class="row"/></sticky-section>"#,
            ".s { top: auto; right: auto; bottom: auto; left: auto; align-content: normal; order: 0; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// `Full`（swiper-item）与 `BorderBoxTail`（scroll-view）都有 CSS 盒的布局能力，
    /// 即使写了布局属性也不该进这条规则。
    #[test]
    fn full_and_border_box_tail_components_are_not_reported() {
        let diagnostics = check(
            "layout-not-no-css-box",
            r#"<swiper><swiper-item class="page"><view/></swiper-item></swiper><scroll-view class="list"><view/></scroll-view>"#,
            ".page { top: 8px; align-content: center; order: 1; }\n.list { top: 8px; align-content: center; order: 1; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    /// 能力表里没有证据的标签保持不报：宁可漏报，不按「应该有」扩表。
    #[test]
    fn unconfirmed_components_are_not_reported() {
        let diagnostics = check(
            "layout-unconfirmed",
            r#"<picker-view class="p"><view/></picker-view><rich-text class="r"/><cover-view class="c"/><page-container class="pc"><view/></page-container>"#,
            ".p { display: flex; width: 300px; top: 8px; }\n.r { display: flex; order: 1; }\n.c { width: 100px; }\n.pc { padding: 8px; }",
        );
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }

    #[test]
    fn webview_page_is_not_reported() {
        let fixture = Fixture::new("layout-webview");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", r#"{"renderer":"webview"}"#)
            .write(
                "pages/index/index.wxml",
                r#"<list-view class="list"><view class="row"/></list-view>"#,
            )
            .write("pages/index/index.wxss", r#".list { display: flex; }"#);
        let project = fixture.project();
        assert!(NativeLayoutIgnored.check(&project).is_empty());
    }

    /// 片段里 `display` 等属性判不了（祖先未知 / 选择器超出受限文法）：不能当成没写。
    #[test]
    fn undecidable_fragment_layout_is_not_reported() {
        let fixture = Fixture::new("layout-undecidable");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write("app.json", SKYLINE_APP)
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", r#"<include src="./part"/>"#)
            .write(
                "pages/index/part.wxml",
                r#"<list-view class="list"><view/></list-view>"#,
            )
            .write(
                "pages/index/index.wxss",
                ".list { display: flex; }\n.ghost + .list { display: block; }",
            );
        let diagnostics = NativeLayoutIgnored.check(&fixture.project());
        assert!(diagnostics.is_empty(), "{diagnostics:#?}");
    }
}
