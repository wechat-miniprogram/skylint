//! `wx.navigateTo` 等跳转的 `routeType` 必须是已注册的自定义路由类型，或 `wx://` 预设。
//!
//! 两遍扫描：先收集整个工程里 `wx.router.addRouteBuilder('<name>', ...)` 注册过的名字，
//! 再检查每个跳转调用实参对象里的 `routeType` 字符串字面量。注册表是工程级的——注册
//! 写在 app.js 或别的页面里都算。`wx://` 开头的预设路由不做白名单校验：预设表随基础库
//! 版本增减，硬编码会在新版本上误报，所以只认前缀（大小写敏感）。
//!
//! 动态取值（变量、模板字符串、调用结果）不做判定；只读字符串字面量。
//! `addRouteBuilder` 注册后没人用不报——多注册合法，可能留给动态拼出来的 routeType。
//!
//! 证据：
//! - `skyline-route/references/preset-route/preset-route.md`（`wx://bottom-sheet` 等预设表）；
//! - `skyline-route/references/custom-route/custom-route-guide.md:5`（注册后由
//!   `wx.navigateTo` 的 `routeType` 触发）；
//! - `skyline-route/references/api/router-api.md:11`、`SKILL.md:132-135`。

use std::collections::HashSet;

use crate::Severity;
use crate::input::js::{ArgInfo, JsDoc};
use crate::project::Project;
use crate::rules::{Rule, RuleMeta};

use super::{Callee, at_span};

const ROUTE_TYPE: &str = "routeType";
const PRESET_PREFIX: &str = "wx://";
const REGISTER: Callee = Callee::Qualified("addRouteBuilder");
/// 只有 navigateTo 会消费 `routeType`：运行时在 `openType === 'navigateTo'` 分支才
/// 读它（wxapplib `src/sdk/src/appservice/app/onAppRoute.js:501`），redirectTo / reLaunch
/// 传了这个键也不会被读。
const NAVIGATION: Callee = Callee::Method("navigateTo");

pub struct RouteType;

impl RouteType {
    pub const ID: &'static str = "skyline/route-type";
}

/// 跳转调用的接收者必须是已知的对象：`navigateTo` 裸调用、`wx.*`、`*.pageRouter`。
/// 其它接收者（`myRouter.navigateTo`、类实例方法）不看，语义无法静态确认。
fn navigation_callee(callee: &str) -> bool {
    if !NAVIGATION.matches(callee) {
        return false;
    }
    match callee.rsplit_once('.') {
        None => true,
        Some((receiver, _)) => receiver == "wx" || receiver.ends_with("pageRouter"),
    }
}

/// `'slide'` / `"slide"` 字面量的内容。注册名不做转义解码，带反斜杠的跳过。
fn string_literal<'a>(doc: &'a JsDoc, arg: &ArgInfo) -> Option<&'a str> {
    let text = doc.source().get(arg.start as usize..arg.end as usize)?;
    let quote = *text.as_bytes().first()?;
    if quote != b'\'' && quote != b'"' {
        return None;
    }
    let inner = text[1..].strip_suffix(quote as char)?;
    (!inner.contains('\\')).then_some(inner)
}

/// 跳转实参对象里的 `routeType` 字面量，与它的键位置（诊断落点）。
fn route_types<'a>(arg: &'a ArgInfo) -> impl Iterator<Item = (&'a str, (u32, u32))> + 'a {
    arg.object_keys
        .iter()
        .enumerate()
        .filter(|(_, key)| key.as_str() == ROUTE_TYPE)
        .filter_map(move |(index, _)| {
            Some((
                arg.object_string_values.get(index)?.as_deref()?,
                arg.object_key_spans.get(index).copied()?,
            ))
        })
}

impl Rule for RouteType {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "routeType 必须是 wx.router.addRouteBuilder 注册过的类型或 wx:// 预设",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        // `Project.scripts` 已覆盖全域（含 `utils/router.js` 这类被 require 的工具模块），
        // 注册点不会漏——这是本规则最容易产生系统性误报的地方。
        // 注册名不是字面量时整个注册表不可知，此时不做「未注册」判定。
        let mut dynamic_registration = false;
        let mut registered: HashSet<&str> = HashSet::new();
        for doc in project.scripts.iter().map(|file| &file.doc) {
            for call in doc.calls() {
                if !REGISTER.matches(&call.callee) {
                    continue;
                }
                match call.args.first().and_then(|arg| string_literal(doc, arg)) {
                    Some(name) => {
                        registered.insert(name);
                    }
                    None => dynamic_registration = true,
                }
            }
        }
        if dynamic_registration {
            return Vec::new();
        }

        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                if !navigation_callee(&call.callee) {
                    continue;
                }
                for arg in &call.args {
                    for (name, (start, end)) in route_types(arg) {
                        // 空串等同没写：运行时 `if (spaData.routeType)` 是假值，不会
                        // 走自定义路由。
                        if name.is_empty()
                            || name.starts_with(PRESET_PREFIX)
                            || registered.contains(name)
                        {
                            continue;
                        }
                        out.push(at_span(
                            Self::ID,
                            Severity::Warning,
                            file,
                            start,
                            end,
                            format!(
                                "`routeType: '{name}'` 没有对应的 `wx.router.addRouteBuilder('{name}', ...)` 注册，跳转时不会应用自定义路由动画"
                            ),
                        ));
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::project::Project;
    use crate::rules::Rule;

    use super::RouteType;
    use crate::rules::testutil::js::{Fixture, messages, single};

    /// 两个页面：注册可以写在另一个页面的 js 里。
    fn two_pages(name: &str, index_js: &str, other_js: &str) -> (Fixture, Project) {
        let f = Fixture::new(name);
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index","pages/other/other"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", index_js)
            .write("pages/other/other.json", "{}")
            .write("pages/other/other.wxml", "<view/>")
            .write("pages/other/other.js", other_js);
        let project = f.project();
        (f, project)
    }

    #[test]
    fn unregistered_route_type_is_reported_on_the_key() {
        let src = "wx.navigateTo({ url: 'pageB', routeType: 'slide' });";
        let (_f, project) = single("route-type-unregistered", src);
        let diagnostics = RouteType.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].severity, crate::Severity::Warning);
        assert_eq!(
            diagnostics[0].message,
            "`routeType: 'slide'` 没有对应的 `wx.router.addRouteBuilder('slide', ...)` 注册，跳转时不会应用自定义路由动画"
        );
        // 落点在键上（ArgInfo 只给键位置）。
        assert_eq!(diagnostics[0].line, 1);
        assert_eq!(
            diagnostics[0].column as usize,
            src.find("routeType").unwrap() + 1
        );
    }

    #[test]
    fn registered_in_the_same_file_is_accepted() {
        let (_f, project) = single(
            "route-type-registered",
            r#"
            wx.router.addRouteBuilder('slide', ({ primaryAnimation }) => {
              return { handlePrimaryAnimation() { 'worklet'; return {}; } };
            });
            wx.navigateTo({ url: 'pageB', routeType: 'slide' });
            "#,
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    /// 路由注册常写在 `utils/router.js` 这种被 require 的工具模块里，它不属于任何页面 /
    /// 组件，所以不在 `Project.scripts`。漏掉它会把整个工程判成「没注册」。
    #[test]
    fn registration_in_a_loose_utility_module_counts() {
        let f = Fixture::new("route-type-loose-util");
        f.write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write(
                "pages/index/index.js",
                "wx.navigateTo({ url: 'pageB', routeType: 'slide' });",
            )
            .write(
                "utils/router.js",
                "export const register = () => wx.router.addRouteBuilder('slide', () => ({}));",
            );
        let project = f.project();
        assert_eq!(project.scripts.len(), 2, "页面 + utils 都在脚本图里");
        assert!(matches!(
            project.scripts[1].scope,
            crate::project::Scope::Loose(_)
        ));
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn registration_in_another_page_counts() {
        let (_f, project) = two_pages(
            "route-type-cross-page",
            "wx.navigateTo({ url: '/pages/other/other', routeType: 'slide' });",
            "wx.router.addRouteBuilder('slide', () => ({ handlePrimaryAnimation() {} }));",
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn only_navigate_to_is_checked() {
        let (_f, project) = single(
            "route-type-apis",
            r#"
            this.pageRouter.navigateTo({ url: 'a', routeType: 'slide' });
            wx.redirectTo({ url: 'b', routeType: 'fade' });
            wx.reLaunch({ url: 'c', routeType: 'rise' });
            "#,
        );
        let diagnostics = RouteType.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("slide"));
    }

    #[test]
    fn preset_route_is_accepted_without_registration() {
        let (_f, project) = single(
            "route-type-preset",
            r#"
            wx.navigateTo({ url: 'a', routeType: 'wx://bottom-sheet' });
            wx.navigateTo({ url: 'b', routeType: 'wx://cubeEffect' });
            "#,
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn dynamic_values_are_not_judged() {
        let (_f, project) = single(
            "route-type-dynamic",
            r#"
            const route = 'slide';
            wx.navigateTo({ url: 'a', routeType: route });
            wx.navigateTo({ url: 'b', routeType: `slide` });
            wx.navigateTo({ url: 'c', routeType: getType() });
            wx.navigateTo({ url: 'd', routeType: cond ? 'a' : 'b' });
            "#,
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    /// 空串是假值，运行时不会走自定义路由，等同于没写。
    #[test]
    fn empty_route_type_is_not_reported() {
        let (_f, project) = single(
            "route-type-empty",
            "wx.navigateTo({ url: 'a', routeType: '' });",
        );
        assert!(messages(&RouteType, &project).is_empty());
    }

    #[test]
    fn route_type_outside_a_navigation_call_is_ignored() {
        let (_f, project) = single(
            "route-type-elsewhere",
            r#"
            this.setData({ routeType: 'slide' });
            foo({ routeType: 'slide' });
            wx.router.addRouteBuilder('other', () => ({}));
            "#,
        );
        // 注册点自己的第一实参不是 navigateTo 的使用点。
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn unused_registration_is_accepted() {
        let (_f, project) = single(
            "route-type-unused",
            r#"
            wx.router.addRouteBuilder('slide', () => ({}));
            wx.navigateTo({ url: 'a' });
            "#,
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn registration_under_another_name_still_reports() {
        let (_f, project) = single(
            "route-type-wrong-name",
            r#"
            wx.router.addRouteBuilder('fade', () => ({}));
            wx.navigateTo({ url: 'a', routeType: 'slide' });
            "#,
        );
        let diagnostics = RouteType.check(&project);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert!(diagnostics[0].message.contains("'slide'"));
    }

    #[test]
    fn empty_registry_still_checks_literal_route_types() {
        // 只写 navigateTo、忘了注册：工程里没有注册点，字面量仍然要报。
        let (_f, project) = single(
            "route-type-empty-registry",
            "wx.navigateTo({ url: 'a', routeType: 'slide' });",
        );
        assert_eq!(messages(&RouteType, &project).len(), 1);
    }

    /// 注册名是变量时整个注册表不可知：任何字面量都可能已注册，全部不报。
    #[test]
    fn dynamic_registration_disables_the_check() {
        let (_f, project) = single(
            "route-type-dynamic-register",
            r#"
            wx.router.addRouteBuilder(name, builder);
            wx.navigateTo({ url: 'a', routeType: 'slide' });
            "#,
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    /// 未知接收者的 `.navigateTo` 不是小程序跳转 API，不看。
    #[test]
    fn unknown_receiver_is_not_checked() {
        let (_f, project) = single(
            "route-type-receiver",
            "myRouter.navigateTo({ url: 'a', routeType: 'slide' });",
        );
        assert_eq!(messages(&RouteType, &project), Vec::<String>::new());
    }

    #[test]
    fn page_router_calls_are_checked() {
        let (_f, project) = single(
            "route-type-page-router",
            "this.pageRouter.navigateTo({ url: 'a', routeType: 'slide' });",
        );
        assert_eq!(messages(&RouteType, &project).len(), 1);
    }
}
