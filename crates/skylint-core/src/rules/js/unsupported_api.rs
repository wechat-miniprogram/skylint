//! Skyline 不支持的 JS API / 页面配置项。
//!
//! 数据表 `rules/data/unsupported_js_api.txt` 是唯一依据，Rust 侧只做三种匹配：
//! `call:<callee>` 命中 callee 精确相等的调用；`plugin-call:<prefix>` 同上（callee 等于
//! `<prefix>` 或以 `<prefix>.` 开头），但只在插件工程里判；`page-option:<key>` 命中裸
//! `Page({...})` 第一个对象实参里的静态键。加条目只改数据文件。
//!
//! 证据：
//! - `skyline-overview/references/migration/compatibility.md:198-205`（Q: 不支持 animate
//!   动画接口，给了 worklet 替代写法）、`:300`（Page.onPageScroll ❌）；
//! - `skyline-overview/SKILL.md:100`（不支持 animate 接口）；
//! - `skyline-route/references/api/router-api.md:9`（所有 `wx.router` 方法均不支持小程序插件）。
//!
//! 只认裸 `Page`：`wx.Page`、`some.Page` 不是页面注册入口。注释与字符串里的
//! `this.animate` 不在 AST 的调用节点上，天然不会命中。

use std::sync::OnceLock;

use crate::project::Project;
use crate::rules::{Rule, RuleMeta};
use crate::Severity;

use super::{at_span, is_plugin};

const SOURCE: &str = include_str!("../data/unsupported_js_api.txt");

/// 页面注册函数。规范写法就是裸 `Page`。
const PAGE: &str = "Page";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// 调用点：callee 精确等于目标文本。
    Call,
    /// 调用点：callee 等于目标名或以 `目标名.` 开头，且只在插件工程里判。
    PluginCall,
    /// `Page({...})` 第一个对象实参里的静态键。
    PageOption,
}

#[derive(Debug)]
struct Entry {
    kind: Kind,
    target: &'static str,
    message: &'static str,
}

pub struct UnsupportedApi;

impl UnsupportedApi {
    pub const ID: &'static str = "skyline/unsupported-api";
}

impl Rule for UnsupportedApi {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: Self::ID,
            severity: Severity::Warning,
            description: "Skyline 不支持的 JS API / 页面配置（如 this.animate、Page.onPageScroll）",
        }
    }

    fn check(&self, project: &Project) -> Vec<crate::Diagnostic> {
        let entries = data();
        let plugin = is_plugin(project);
        let mut out = Vec::new();
        for file in &project.scripts {
            for call in file.doc.calls() {
                for entry in entries {
                    let span = match entry.kind {
                        Kind::Call => (call.callee == entry.target)
                            .then_some((call.start, call.end)),
                        Kind::PluginCall => (plugin && matches_namespace(&call.callee, entry.target))
                            .then_some((call.start, call.end)),
                        Kind::PageOption => {
                            if call.callee != PAGE {
                                continue;
                            }
                            call.args
                                .first()
                                .and_then(|page| page.key_span(entry.target))
                        }
                    };
                    let Some((start, end)) = span else { continue };
                    out.push(at_span(
                        Self::ID,
                        Severity::Warning,
                        file,
                        start,
                        end,
                        entry.message,
                    ));
                }
            }
        }
        out
    }
}

fn data() -> &'static [Entry] {
    static TABLE: OnceLock<Vec<Entry>> = OnceLock::new();
    TABLE.get_or_init(|| parse(SOURCE).expect("JS 不支持 API 表格式错误"))
}

/// `wx.router` 命中 `wx.router` 本身与它下面的一切方法。
fn matches_namespace(callee: &str, prefix: &str) -> bool {
    callee == prefix
        || callee
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// 解析数据文件。格式错误一律返回带行号的说明，便于同步时定位。
fn parse(source: &'static str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    for (index, raw) in source.lines().enumerate() {
        let line_no = index + 1;
        let line = raw.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((pattern, message)) = line.split_once('|') else {
            return Err(format!("第 {line_no} 行：缺少 `模式 | 说明` 分隔符：{line}"));
        };
        let pattern = pattern.trim();
        let (kind, target) = if let Some(target) = pattern.strip_prefix("call:") {
            (Kind::Call, target.trim())
        } else if let Some(target) = pattern.strip_prefix("plugin-call:") {
            (Kind::PluginCall, target.trim())
        } else if let Some(target) = pattern.strip_prefix("page-option:") {
            (Kind::PageOption, target.trim())
        } else {
            return Err(format!(
                "第 {line_no} 行：模式必须以 `call:`、`plugin-call:` 或 `page-option:` 开头：{pattern}"
            ));
        };
        if target.is_empty() {
            return Err(format!("第 {line_no} 行：模式缺少目标：{pattern}"));
        }
        let message = message.trim();
        if message.is_empty() {
            return Err(format!("第 {line_no} 行：说明为空"));
        }
        entries.push(Entry {
            kind,
            target,
            message,
        });
    }
    if entries.is_empty() {
        return Err("JS 不支持 API 表为空".to_string());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use crate::rules::Rule;

    use super::super::testutil::{messages, plugin, single, single_with_app};
    use super::{Kind, UnsupportedApi, parse};

    #[test]
    fn unsupported_calls_and_page_options_are_reported() {
        let (_f, project) = single(
            "unsupported-positive",
            "this.animate('.a', [], 1000);\nPage({\n  onPageScroll() {},\n  data: {},\n});\nthis.animate('.b', [], 200);",
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 3, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::Severity::Warning)
        );
        assert_eq!(diagnostics[0].path, "pages/index/index.js");
        assert!(diagnostics[0].message.contains("this.animate"));
        assert!(diagnostics[1].message.contains("onPageScroll"));
        assert!(diagnostics[1].message.contains("bindscroll"));
        // Page 选项报在键上，不是调用起点。
        assert_eq!(diagnostics[1].line, 3);
        assert_eq!(diagnostics[1].column, 3);
        assert_eq!(diagnostics[2].message, diagnostics[0].message);
    }

    #[test]
    fn app_js_is_scanned_too() {
        let (_f, project) = single_with_app(
            "unsupported-app",
            "Page({ onPageScroll() {} });",
            Some("this.animate('.a', [], 1);"),
        );
        let diagnostics = UnsupportedApi.check(&project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "app.js");
        assert_eq!(diagnostics[1].path, "pages/index/index.js");
    }

    #[test]
    fn non_calls_and_non_page_options_are_ignored() {
        let (_f, project) = single(
            "unsupported-negative",
            r#"
            // this.animate('.a', [], 1);
            const text = 'this.animate';
            page.onPageScroll = () => {};
            wx.Page({ onPageScroll() {} });
            this.animateLater('.b', [], 1);
            Page({ onLoad() {} });
            "#,
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn non_literal_page_argument_is_ignored() {
        let (_f, project) = single("unsupported-dynamic", "Page(config);");
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }

    #[test]
    fn data_file_parses_every_prefix() {
        let entries = parse(super::SOURCE).unwrap();
        assert_eq!(entries.len(), 3);
        assert!(entries.iter().any(|e| e.target == "this.animate"));
        assert!(entries.iter().any(|e| e.target == "onPageScroll"));
        assert!(
            entries
                .iter()
                .any(|e| e.kind == Kind::PluginCall && e.target == "wx.router")
        );

        let err = parse("call:foo\n").unwrap_err();
        assert!(err.contains("分隔符"), "{err}");
        let err = parse("foo:bar | baz\n").unwrap_err();
        assert!(err.contains("call:"), "{err}");
        let err = parse("call: | baz\n").unwrap_err();
        assert!(err.contains("缺少目标"), "{err}");
    }

    #[test]
    fn router_calls_are_reported_only_in_plugin_projects() {
        let (_f, plugin_project) = plugin(
            "unsupported-plugin-router",
            "wx.router.navigateTo({ url: 'x' });\nwx.router.addRouteBuilder('a', b);",
        );
        let diagnostics = UnsupportedApi.check(&plugin_project);
        assert_eq!(diagnostics.len(), 2, "{diagnostics:#?}");
        assert!(
            diagnostics
                .iter()
                .all(|d| d.message.contains("wx.router"))
        );

        let (_n, normal) = single(
            "unsupported-normal-router",
            "wx.router.navigateTo({ url: 'x' });",
        );
        assert_eq!(messages(&UnsupportedApi, &normal), Vec::<String>::new());
    }

    #[test]
    fn plugin_prefix_does_not_match_similar_names() {
        let (_f, project) = plugin(
            "unsupported-plugin-prefix",
            "wx.routers.navigateTo({});\nmywx.router.x();\nwx.routerX.y();",
        );
        assert_eq!(messages(&UnsupportedApi, &project), Vec::<String>::new());
    }
}
