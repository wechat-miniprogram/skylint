//! 必须声明 `'worklet'` 的函数位置：WXML 手势绑定、`applyAnimatedStyle`、路由动画处理函数。
//!
//! 三个来源都问同一个问题——「这个函数会被搬到 UI 线程执行吗」，答案是肯定的那几处
//! 就必须有 `'worklet'` 指令，否则它退回 JS 线程，跨线程的延迟会让交互动画抖动：
//!
//! - `worklet:ongesture="onPan"` 等 WXML 绑定（台账 #152）；
//! - `this.applyAnimatedStyle(selector, updater)` 的第二个实参（#154）；
//! - `wx.router.addRouteBuilder(type, builder)` → builder **返回对象**里的
//!   `handlePrimaryAnimation` / `handleSecondaryAnimation` / `handlePreviousPageAnimation`
//!   三个键（#153）。
//!
//! 「函数」既可能是内联写法，也可能是同文件里的具名函数。内联写法由
//! [`ArgInfo::fn_worklet`] / [`ObjectFn::fn_worklet`] 直接给出；具名函数按名字在同一份
//! 脚本里查——跨文件 / mixin 里的函数查不到，一律不报（找不到就等于无法证明它缺指令）。
//!
//! 证据：
//! - `skyline-components/references/gesture/gesture-system.md:63`（`worklet:ongesture`
//!   「必须为 Worklet」）、`:186`（MUST 声明 `'worklet'`）、`:198`（回调在 UI 线程）；
//! - `skyline-worklet/references/core/worklet-overview.md:148`（`applyAnimatedStyle` 的
//!   第二个参数 `updater` 为 worklet 函数）、`:134-139` 的示例；
//!   `skyline-worklet/SKILL.md:39-42`（updater 示例带 `'worklet'`）；
//! - `skyline-route/references/custom-route/custom-route-guide.md:5`（builder 注册入口）、
//!   `skyline-route/references/api/router-api.md:65`（`handle*Animation` 必须 `'worklet'`）。
//!
//! #153 判的是 builder 返回的路由动画处理函数，不是 builder 本身。`router-api.md:65` 的
//! 原文：「`handlePrimaryAnimation` / `handleSecondaryAnimation` /
//! `handlePreviousPageAnimation` 必须使用 `'worklet'` 指令」。官方示例
//! （`router-api.md:41-58`、`skyline-route/SKILL.md:146-155`）里 builder 本身都不写
//! `'worklet'`，所以这里只认三个 `handle*Animation` 键。

use crate::input::js::CallInfo;
use crate::project::{JsFile, Project};
use crate::rules::wxml::load_doc;
use crate::rules::{Rule, RuleMeta};
use crate::{Diagnostic, Severity};

use super::{at_span, scope_script, unit_wxml};

const APPLY_ANIMATED_STYLE: &str = "applyAnimatedStyle";
const ADD_ROUTE_BUILDER: &str = "addRouteBuilder";
const WORKLET_PREFIX: &str = "worklet:";

pub struct WorkletDirective;

impl WorkletDirective {
    pub const ID: &'static str = "skyline/worklet-directive";
}

impl Rule for WorkletDirective {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Error,
            description: "worklet: 绑定、applyAnimatedStyle 与路由动画处理函数缺少 'worklet' 指令",
        }
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        wxml_bindings(project, &mut out);
        for file in &project.scripts {
            for call in file.doc.calls() {
                if call.callee.ends_with(APPLY_ANIMATED_STYLE) {
                    out.extend(updater_diagnostic(file, call));
                }
            }
            out.extend(route_animation_diagnostics(file));
        }
        out
    }
}

/// 路由动画处理函数：builder 返回对象里必须带 `'worklet'` 的那三个键。
const ROUTE_ANIMATIONS: [&str; 3] = [
    "handlePrimaryAnimation",
    "handleSecondaryAnimation",
    "handlePreviousPageAnimation",
];

/// `applyAnimatedStyle` 的 updater：内联函数报在实参上，具名函数报在调用上。
fn updater_diagnostic(file: &JsFile, call: &CallInfo) -> Option<Diagnostic> {
    let arg = call.args.get(1)?;
    let (start, end, subject, help) = if arg.fn_worklet == Some(false) {
        (
            arg.start,
            arg.end,
            "updater 是内联函数，但".to_string(),
            "在内联函数体的第一行加 `'worklet'`".to_string(),
        )
    } else {
        let name = arg.ident.as_deref()?;
        if named_has_worklet(file, name) != Some(false) {
            return None;
        }
        (
            call.start,
            call.end,
            format!("updater `{name}` "),
            format!("在 `{name}` 的函数体第一行加 `'worklet'`"),
        )
    };
    Some(
        at_span(
            WorkletDirective::ID,
            Severity::Error,
            file,
            start,
            end,
            format!("`applyAnimatedStyle` 的 {subject}没有 `'worklet'` 指令，样式不会随共享变量在 UI 线程更新"),
        )
        .help(help),
    )
}

/// builder 返回对象里的三个路由动画处理函数。落点统一在键名上。
///
/// 只认 `addRouteBuilder` 那个 builder 函数体里的对象：同名键在别的对象上很常见，
/// 按「文件里有 addRouteBuilder」整份扫描会误报。
fn route_animation_diagnostics(file: &JsFile) -> Vec<Diagnostic> {
    let builders = route_builder_spans(file);
    if builders.is_empty() {
        return Vec::new();
    }
    file.doc
        .object_fns()
        .iter()
        .filter(|item| ROUTE_ANIMATIONS.contains(&item.key.as_str()))
        .filter(|item| {
            builders
                .iter()
                .any(|(start, end)| (*start..*end).contains(&item.key_start))
        })
        .filter_map(|item| {
            let key = item.key.as_str();
            let (subject, help) = if item.fn_worklet == Some(false) {
                (
                    format!("`{key}` 是内联函数，但"),
                    format!("在 `{key}` 的函数体第一行加 `'worklet'`"),
                )
            } else {
                let name = item.ident.as_deref()?;
                if named_has_worklet(file, name) != Some(false) {
                    return None;
                }
                (
                    format!("`{key}` "),
                    format!("在 `{name}` 的函数体第一行加 `'worklet'`"),
                )
            };
            Some(
                at_span(
                    WorkletDirective::ID,
                    Severity::Error,
                    file,
                    item.key_start,
                    item.key_end,
                    format!("路由动画的 {subject}没有 `'worklet'` 指令，动画不会在 UI 线程执行"),
                )
                .help(help),
            )
        })
        .collect()
}

/// 每个 `addRouteBuilder(type, builder)` 的 builder 函数体区间。
///
/// builder 是具名函数时取它的函数体；是内联函数时取实参本身。查不到定义
/// （跨文件 / mixin）就不收录——找不到就无法证明它缺指令。
fn route_builder_spans(file: &JsFile) -> Vec<(u32, u32)> {
    let mut spans = Vec::new();
    for call in file.doc.calls() {
        if !call.callee.ends_with(ADD_ROUTE_BUILDER) {
            continue;
        }
        let Some(arg) = call.args.get(1) else {
            continue;
        };
        if let Some(name) = arg.ident.as_deref() {
            if let Some(function) = file.doc.functions().iter().find(|f| f.name == Some(name)) {
                spans.push((function.body_start, function.body_end));
            }
        } else if arg.fn_worklet.is_some() {
            spans.push((arg.start, arg.end));
        }
    }
    spans
}

/// 同文件里是否有这个具名函数，以及它是否 worklet；找不到为 `None`。
///
/// 同名函数出现多次时（极罕见）只要有任意一个带指令就放行：宁可漏报。
fn named_has_worklet(file: &JsFile, name: &str) -> Option<bool> {
    let mut found = false;
    let mut any_worklet = false;
    for function in file.doc.functions() {
        if function.name == Some(name) {
            found = true;
            any_worklet |= function.has_worklet;
        }
    }
    found.then_some(any_worklet)
}

/// WXML 上的 `worklet:` 绑定：值必须是静态标识符，且在同一单元脚本里能找到同名函数。
fn wxml_bindings(project: &Project, out: &mut Vec<Diagnostic>) {
    for (path, scope) in unit_wxml(project) {
        let Some(script) = scope_script(project, &scope) else {
            continue;
        };
        let Some(doc) = load_doc(project, path) else {
            continue;
        };
        for element in doc.elements() {
            for attr in &element.attributes {
                let Some(event) = attr.name.strip_prefix(WORKLET_PREFIX) else {
                    continue;
                };
                // 动态值（`{{handler}}`）无法静态确定是哪个函数。
                if !attr.is_static {
                    continue;
                }
                let Some(name) = attr.value.as_deref().filter(|value| is_identifier(value)) else {
                    continue;
                };
                if named_has_worklet(script, name) != Some(false) {
                    continue;
                }
                let start = attr.value_start.unwrap_or(attr.start);
                let (line, column) = doc.position(start);
                let (end_line, end_column) = doc.position(attr.end);
                out.push(
                    Diagnostic::new(
                        WorkletDirective::ID,
                        Severity::Error,
                        format!(
                            "`worklet:{event}` 绑定到 `{name}`，但 `{name}` 没有声明 `'worklet'` 指令，手势回调会退回 JS 线程"
                        ),
                        path,
                        line,
                        column,
                    )
                    .end(end_line, end_column)
                    .help(format!(
                        "在 `{name}` 的函数体第一行加 `'worklet'`，或改用 `bind:{event}` 的普通绑定"
                    )),
                );
            }
        }
    }
}

/// 裸标识符（`onPan`、`$handler`）；`this.fn`、`a.b`、表达式都不算。
fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;
    use crate::rules::js::testutil::{Fixture, messages, single};

    use super::WorkletDirective;

    /// 单页工程，页面 wxml 与 js 可分别指定。
    fn project(name: &str, wxml: &str, js: &str) -> (Fixture, crate::project::Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.js", js);
        let project = f.project();
        (f, project)
    }

    const PAN_WITHOUT: &str = "function onPan(e) { this.offset.value = e.absoluteX; }";
    const PAN_WITH: &str = "function onPan(e) { 'worklet'; this.offset.value = e.absoluteX; }";

    #[test]
    fn gesture_binding_without_worklet_is_reported_on_the_value() {
        let (_f, project) = project(
            "directive-gesture",
            r#"<view><pan-gesture-handler worklet:ongesture="onPan"/></view>"#,
            PAN_WITHOUT,
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert_eq!(diagnostics[0].path, "pages/index/index.wxml");
        // 落在 `onPan` 这个值上，而不是属性名或整行。
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (1, 47));
        assert!(diagnostics[0].message.contains("worklet:ongesture"));
        assert!(diagnostics[0].message.contains("onPan"));
    }

    #[test]
    fn gesture_binding_with_worklet_is_accepted() {
        let (_f, project) = project(
            "directive-gesture-ok",
            r#"<view><pan-gesture-handler worklet:ongesture="onPan"/></view>"#,
            PAN_WITH,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn gesture_binding_to_an_unknown_function_is_ignored() {
        // 函数可能在 mixin / 外部模块里，查不到就不判。
        let (_f, project) = project(
            "directive-gesture-mixin",
            r#"<view><pan-gesture-handler worklet:ongesture="onPan"/></view>"#,
            "Behavior({});",
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    /// 全域扫描后 `utils/helper.ts` 也在脚本图里，但 WXML 绑定只能查该单元自己的脚本：
    /// 同名的 util 函数不是这条绑定的处理器。
    #[test]
    fn gesture_binding_does_not_match_a_function_in_a_loose_script() {
        let f = Fixture::new("directive-gesture-loose");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write(
                "pages/index/index.wxml",
                r#"<pan-gesture-handler worklet:ongesture="onPan"/>"#,
            )
            .write("pages/index/index.js", "Page({});")
            .write("utils/helper.ts", PAN_WITHOUT);
        let project = f.project();
        assert!(
            project
                .scripts
                .iter()
                .any(|file| file.path == "utils/helper.ts")
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn plain_bindings_and_dynamic_values_are_ignored() {
        let (_f, project) = project(
            "directive-gesture-negative",
            r#"<view bind:ongesture="onPan" worklet:ongesture="{{handler}}"/>"#,
            PAN_WITHOUT,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn gesture_binding_in_a_component_uses_the_component_script() {
        let f = Fixture::new("directive-component");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", "Page({});")
            .write(
                "components/pan/pan.json",
                r#"{"usingComponents":{},"component":true}"#,
            )
            .write(
                "components/pan/pan.wxml",
                r#"<pan-gesture-handler worklet:ongesture="onPan"/>"#,
            )
            .write("components/pan/pan.js", PAN_WITHOUT)
            .write(
                "pages/index/index.json",
                r#"{"usingComponents":{"pan":"/components/pan/pan"}}"#,
            );
        let project = f.project();
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/pan/pan.wxml");
    }

    #[test]
    fn inline_apply_animated_style_updater_without_worklet_is_reported() {
        let (_f, project) = single(
            "directive-animated-inline",
            "Page({ onLoad() { this.applyAnimatedStyle('.x', () => ({ transform: 'none' })); } });",
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("applyAnimatedStyle"));
        assert!(diagnostics[0].message.contains("内联函数"));
    }

    #[test]
    fn inline_apply_animated_style_updater_with_worklet_is_accepted() {
        let (_f, project) = single(
            "directive-animated-inline-ok",
            r#"Page({ onLoad() {
              this.applyAnimatedStyle('.x', () => { 'worklet'; return { transform: 'none' }; });
              this.applyAnimatedStyle('.y', function () { 'worklet'; return {}; });
            } });"#,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn named_apply_animated_style_updater_follows_the_function() {
        let (_f, project) = single(
            "directive-animated-named",
            r#"
const updateStyle = () => { return { transform: 'none' }; };
const okStyle = () => { 'worklet'; return { transform: 'none' }; };
Page({
  onLoad() {
    this.applyAnimatedStyle('.a', updateStyle);
    this.applyAnimatedStyle('.b', okStyle);
    this.applyAnimatedStyle('.c', this.computedStyle);
  }
});
"#,
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("updateStyle"));
        // 具名函数报在调用点上。
        assert_eq!(diagnostics[0].line, 6);
    }

    #[test]
    fn official_route_builder_example_is_accepted() {
        // `skyline-route/references/api/router-api.md:41-58` 原样。
        let (_f, project) = single(
            "directive-route-official",
            r#"
// 定义自定义路由：从右侧推入
const slideRouteBuilder = (customRouteContext) => {
  const { primaryAnimation } = customRouteContext
  const { windowWidth } = wx.getWindowInfo()

  const handlePrimaryAnimation = () => {
    'worklet'
    const transX = windowWidth * (1 - primaryAnimation.value)
    return {
      transform: `translateX(${transX}px)`,
    }
  }

  return { handlePrimaryAnimation }
}

// 注册
wx.router.addRouteBuilder('slide', slideRouteBuilder)
"#,
        );
        // builder 本身没有 `'worklet'`——官方写法如此，不能报。
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn returned_handler_without_worklet_is_reported_on_the_key() {
        let (_f, project) = single(
            "directive-route-named-missing",
            r#"
const slideRouteBuilder = (ctx) => {
  const handlePrimaryAnimation = () => ({ transform: 'translateX(0px)' });
  return { handlePrimaryAnimation };
};
wx.router.addRouteBuilder('slide', slideRouteBuilder);
"#,
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Error);
        assert!(diagnostics[0].message.contains("handlePrimaryAnimation"));
        assert!(diagnostics[0].message.contains("路由动画"));
        // 落在返回对象里的键名上（第 4 行、`handlePrimaryAnimation` 开头）。
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (4, 12));
    }

    #[test]
    fn inline_returned_handler_without_worklet_is_reported() {
        let (_f, project) = single(
            "directive-route-inline-missing",
            "wx.router.addRouteBuilder('slide', (ctx) => {
  return { handlePrimaryAnimation: () => ({ transform: 'x' }) };
});",
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("是内联函数"));
        assert_eq!((diagnostics[0].line, diagnostics[0].column), (2, 12));
    }

    #[test]
    fn inline_returned_handler_with_worklet_is_accepted() {
        let (_f, project) = single(
            "directive-route-inline-ok",
            r#"wx.router.addRouteBuilder('slide', (ctx) => {
  return {
    handlePrimaryAnimation: () => { 'worklet'; return { transform: 'x' }; },
    handleSecondaryAnimation() { 'worklet'; return {}; },
  };
});"#,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    #[test]
    fn identifier_handler_follows_the_named_function() {
        let (_f, project) = single(
            "directive-route-ident",
            r#"
const animate = () => { return {}; };
const animateOk = () => { 'worklet'; return {}; };
wx.router.addRouteBuilder('a', () => ({ handlePrimaryAnimation: animate }));
wx.router.addRouteBuilder('b', () => ({ handlePrimaryAnimation: animateOk }));
wx.router.addRouteBuilder('c', () => ({ handlePrimaryAnimation: external }));
"#,
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("handlePrimaryAnimation"));
        assert_eq!(diagnostics[0].line, 4);
    }

    #[test]
    fn every_route_animation_key_is_checked() {
        let (_f, project) = single(
            "directive-route-three-keys",
            r#"wx.router.addRouteBuilder('slide', () => ({
  handlePrimaryAnimation: () => ({}),
  handleSecondaryAnimation: () => ({}),
  handlePreviousPageAnimation: () => ({}),
  transitionDuration: 300,
}));"#,
        );
        assert_eq!(WorkletDirective.check(&project).len(), 3);
    }

    #[test]
    fn same_key_without_add_route_builder_is_ignored() {
        let (_f, project) = single(
            "directive-route-unrelated",
            r#"
const gesture = { handlePrimaryAnimation: () => ({}) };
const config = { handleSecondaryAnimation: function () {} };
Page({ gesture, config });
"#,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    /// 文件里注册了路由，但别的对象上有同名键：不能按「文件级」开关整份扫描。
    #[test]
    fn unrelated_object_in_a_file_that_registers_routes_is_ignored() {
        let (_f, project) = single(
            "directive-route-mixed",
            r#"
const gesture = { handlePrimaryAnimation: () => ({}) };
wx.router.addRouteBuilder('slide', () => ({
  handlePrimaryAnimation: () => { 'worklet'; return {}; },
}));
"#,
        );
        assert_eq!(messages(&WorkletDirective, &project), Vec::<String>::new());
    }

    /// 同一文件里两个对象都缺指令时，只报 builder 里那一个。
    #[test]
    fn only_the_builder_object_is_reported() {
        let (_f, project) = single(
            "directive-route-scope",
            r#"
const gesture = { handlePrimaryAnimation: () => ({}) };
wx.router.addRouteBuilder('slide', () => ({ handlePrimaryAnimation: () => ({}) }));
"#,
        );
        let diagnostics = WorkletDirective.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].line, 3);
    }
}
