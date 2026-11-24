//! WXML 的 `worklet:` 事件绑定在 WebView 下静默不触发。
//!
//! 模板编译把 `worklet:ongesture` 编成 `wl(...)`，只挂一个 lifetime 钩子；唯一消费方是
//! Skyline 组件的 `worklet-handler` behavior（Skyline 组件实现），
//! WebView 侧的 `wx-component` 完全没有对应实现——既不报错也不退化成普通事件，
//! 回调永远不会被调用（`glass-easel-template-compiler` 的 proc_gen 只产出 `wl` 标记）。
//!
//! 两端兼容：同时挂普通 `bind:xxx`，或用 `wx:if="{{renderer === 'skyline'}}"` 把
//! worklet 绑定隔离在 Skyline 分支里。注意 `if (wx.worklet)` 不能当守卫——
//! 这个 API 在 WebView 下也存在（有 JS 模拟实现），只是 `worklet:` 绑定没消费方。

use crate::diag::{Diagnostic, Severity};
use crate::project::Project;
use crate::rules::{
    Rule, RuleMeta, Target, VisualAction, VisualCase, VisualComponent, VisualExpect,
};

use super::{for_each_element, span_diagnostic};

pub const ID: &str = "webview/worklet-binding";

/// 视觉用例的组件夹具：`worklet:` 只能挂在真正的 Skyline 组件上才会被消费。
///
/// 页面里的 `<view worklet:tap>` 两端都没有消费方（Skyline 只给 `scroll-view`、
/// `*-gesture-handler` 等组件的特定属性挂 worklet-handler），所以视觉用例改用
/// `scroll-view` 的 `worklet:onscrollupdate`：点一下页面让 scroll-view 以
/// `scroll-top=160` 挂载，Skyline 的原生滚动会触发 worklet 回调，WebView 的
/// `wl()` 则没有任何消费方。
///
/// 生成器的 `project.config.json` 固定不带 `setting.compileWorklet`，所以这里手写
/// 编译器会产出的 worklet 标记（`__worklet` / `_closure` / `asString`），否则两端的
/// 处理器都不会被识别成 worklet。
const WORKLET_JS: &str = r#"function mark() {
  const page = getCurrentPages()[0];
  const probe = page.selectComponent('#probe');
  if (probe) probe.setData({ fired: true, label: 'worklet' });
}
function onUpdate() {
  const { wx, mark } = jsThis._closure;
  wx.worklet.runOnJS(mark)();
}
Object.assign(onUpdate, {
  _closure: { wx: { worklet: { runOnJS: wx.worklet.runOnJS } }, mark },
  asString: "function onUpdate(){const{wx,mark}=jsThis._closure;{wx.worklet.runOnJS(mark)();}}",
  __workletHash: 4242424242,
  __location: "components/worklet-probe/index.js (7:0)",
  __worklet: true,
});
Component({
  properties: { armed: { type: Boolean, value: false } },
  data: { fired: false, label: 'idle' },
  methods: { onUpdate: onUpdate },
});
"#;

static WORKLET_COMPONENTS: &[VisualComponent] = &[VisualComponent {
    name: "worklet-probe",
    wxml: r#"<view class="probe {{fired ? 'hit' : ''}}">
  <text>{{label}}</text>
  <scroll-view wx:if="{{armed}}" class="list" scroll-y scroll-top="160" worklet:onscrollupdate="onUpdate">
    <view class="row">r1</view>
    <view class="row">r2</view>
    <view class="row">r3</view>
    <view class="row">r4</view>
    <view class="row">r5</view>
  </scroll-view>
</view>"#,
    wxss: ".probe { width: 300px; height: 240px; background: #888; }\n.probe.hit { background: #c2402a; }\n.list { height: 160px; }\n.row { height: 60px; background: #f4f6fb; }\n",
    js: Some(WORKLET_JS),
}];

/// 视觉用例：点一下页面，scroll-view 才挂载并以 `scroll-top=160` 做初始滚动。
///
/// 实测（2026-11）：Skyline 侧 worklet 回调经 `runOnJS` 把 `fired` 置 true、文案变
/// `worklet`（方块由灰变红）；WebView 侧 `worklet:onscrollupdate` 编成 `wl()` 后没有
/// 消费方，`fired` 保持 false、文案仍是 `idle`。组件内部节点从页面选择器查不到，
/// 所以走整页比对。
static VISUAL_CASES: &[VisualCase] = &[VisualCase {
    wxml: r#"<view class="wrap" id="target" bind:tap="onTap"><worklet-probe id="probe" armed="{{armed}}"/></view>"#,
    wxss: ".wrap { padding: 8px; background: #f4f6fb; }\n",
    js: Some("Page({ data: { armed: false }, onTap() { this.setData({ armed: true }); } });\n"),
    expect: VisualExpect::Differ,
    note: "`scroll-view` 的 `worklet:onscrollupdate`：点一下让 scroll-view 以 scroll-top=160 挂载。Skyline 的 worklet 回调跑起来（方块变红、文案 worklet），WebView 的 `wl()` 没有消费方（方块保持灰、文案 idle）。两端 scroll-view 停在同一位置（r4/r5 间距都是 60），所以这 10.99% 不全是 worklet：还含 label 行盒差 18.5 vs 21.5（内部内容整体低约 5px）与 WebView 的滚动条槽（内容右缘 292.5，Skyline 的滚动条叠在内容上）。整页比对",
    target: "",
}];

pub struct WorkletBinding;

impl Rule for WorkletBinding {
    fn meta(&self) -> RuleMeta {
        RuleMeta {
            id: ID,
            severity: Severity::Error,
            description: "`worklet:` 事件绑定在 WebView 下不触发回调，且不报错",
        }
    }

    fn target(&self) -> Target {
        Target::Webview
    }

    fn visual_cases(&self) -> &'static [VisualCase] {
        VISUAL_CASES
    }

    fn visual_components(&self) -> &'static [VisualComponent] {
        WORKLET_COMPONENTS
    }

    fn visual_actions(&self) -> &'static [VisualAction] {
        const ACTIONS: &[VisualAction] = &[VisualAction::Tap("#target")];
        ACTIONS
    }

    fn check(&self, project: &Project) -> Vec<Diagnostic> {
        let mut out = Vec::new();
        for_each_element(project, self.target(), |site| {
            let (doc, element) = (site.doc, site.element);
            for attribute in &element.attributes {
                if !attribute.name.starts_with("worklet:") {
                    continue;
                }
                out.push(
                    span_diagnostic(
                        ID,
                        doc,
                        attribute.start,
                        attribute.end,
                        doc.path(),
                        Severity::Error,
                        format!(
                            "`{}` 在 WebView 下没有消费方，回调不会触发也不会报错",
                            attribute.name
                        ),
                    )
                    .help("同时挂普通 `bind:` 事件，或用 `wx:if=\"{{renderer === 'skyline'}}\"` 隔离 worklet 绑定"),
                );
            }
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::WorkletBinding;
    use crate::rules::testutil::wxml::{Fixture, check_rule, project};

    #[test]
    fn worklet_binding_is_reported() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet",
                "<pan-gesture-handler worklet:ongesture=\"onPan\"/>",
            ),
            &WorkletBinding,
        );
        assert_eq!(diagnostics.len(), 1);
        assert!(diagnostics[0].message.contains("worklet:ongesture"));
    }

    #[test]
    fn renderer_guard_silences() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet-guard",
                "<view wx:if=\"{{renderer === 'skyline'}}\"><pan-gesture-handler worklet:ongesture=\"onPan\"/></view>",
            ),
            &WorkletBinding,
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn visual_case_is_reported() {
        let case = &super::VISUAL_CASES[0];
        let component = &super::WORKLET_COMPONENTS[0];
        let fixture = Fixture::new("webview/worklet-visual");
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write(
                "pages/index/index.json",
                &format!(
                    r#"{{"usingComponents":{{"{}":"/components/{}/index"}}}}"#,
                    component.name, component.name
                ),
            )
            .write("pages/index/index.wxml", case.wxml)
            .write("pages/index/index.js", case.js.unwrap())
            .write("pages/index/index.wxss", case.wxss)
            .write(
                &format!("components/{}/index.json", component.name),
                r#"{"component":true}"#,
            )
            .write(
                &format!("components/{}/index.wxml", component.name),
                component.wxml,
            )
            .write(
                &format!("components/{}/index.js", component.name),
                component.js.unwrap(),
            );
        let diagnostics = check_rule(&fixture, &WorkletBinding);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:#?}");
        assert_eq!(diagnostics[0].path, "components/worklet-probe/index.wxml");
        assert!(diagnostics[0].message.contains("worklet:onscrollupdate"));
    }

    #[test]
    fn plain_binding_is_fine() {
        let diagnostics = check_rule(
            &project(
                "webview/worklet-none",
                "<view bind:tap=\"onTap\"><text>a</text></view>",
            ),
            &WorkletBinding,
        );
        assert!(diagnostics.is_empty());
    }
}
