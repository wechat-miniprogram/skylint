//! 把规则的 [`VisualCase`] 写成可运行的小程序工程。
//!
//! 用法（由 `tools/visual/run.sh` 调用，也可以手工跑）：
//!
//! ```text
//! cargo run --example rule_visual_cases -- --renderer skyline --out .tmp/visual/probe-sky
//! cargo run --example rule_visual_cases -- --renderer webview --out .tmp/visual/probe-wv
//! ```
//!
//! 产物布局：`<out>/cases.json` + `<out>/project/`（小程序工程）。`cases.json` 故意放在
//! 工程同级：工程目录里多一个非小程序文件会干扰构建与文件监听。
//!
//! renderer 必须同时写进 app.json、每个页面 json 以及每个组件 json：只写 app 级时，
//! 部分页面会退回 WebView（踩过的坑）。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use skylint::Registry;
use skylint::rules::VisualComponent;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut renderer: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    // 规则 id 子串；用于排除「用例本身会让编译器失败」的规则，保证其余批次能跑。
    let mut skips: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--renderer" => renderer = args.next(),
            "--out" => out = args.next().map(PathBuf::from),
            "--skip" => {
                if let Some(value) = args.next() {
                    skips.push(value);
                }
            }
            other => return Err(format!("未知参数：{other}").into()),
        }
    }
    let renderer = renderer.ok_or("必须指定 --renderer skyline|webview")?;
    if !matches!(renderer.as_str(), "skyline" | "webview") {
        return Err(format!("--renderer 只能是 skyline 或 webview，收到 {renderer}").into());
    }
    let out = out.ok_or("必须指定 --out <目录>")?;

    let project = out.join("project");
    if project.exists() {
        fs::remove_dir_all(&project)?;
    }
    fs::create_dir_all(&project)?;

    write(
        &project.join("project.config.json"),
        r#"{"miniprogramRoot":"","appid":"touristappid"}"#,
    )?;
    write(&project.join("app.js"), "App({})\n")?;

    let mut pages = Vec::new();
    let mut cases = Vec::new();
    // 规则级 app.json 覆盖；整批工程共用一份 app.json，所以只允许行为中性的覆盖。
    let mut app_overrides = Vec::new();
    // 组件目录是全局的 `components/<name>/`：同名组件必须内容一致，否则后写的会
    // 静默覆盖先写的，某个 case 的夹具就不是它声明的那个了。
    let mut components: BTreeMap<&'static str, &'static VisualComponent> = BTreeMap::new();
    for (meta, visual_cases, rule_components, app_override) in Registry::builtin().visual_cases() {
        if skips.iter().any(|skip| meta.id.contains(skip.as_str())) {
            continue;
        }
        if let Some(text) = app_override {
            app_overrides.push(parse_app_override(meta.id, text)?);
        }
        let base = slugify(meta.id);
        // 规则级夹具：该规则的每个 case 页面都引用同一组组件。
        register_components(&mut components, meta.id, rule_components)?;
        for (index, case) in visual_cases.iter().enumerate() {
            let slug = if index == 0 {
                base.clone()
            } else {
                format!("{base}-{}", index + 1)
            };

            let route = format!("pages/{slug}/index");
            let dir = project.join("pages").join(&slug);
            fs::create_dir_all(&dir)?;
            write(&dir.join("index.wxml"), case.wxml)?;
            write(&dir.join("index.wxss"), case.wxss)?;
            write(&dir.join("index.js"), case.js.unwrap_or("Page({})\n"))?;
            let mut page_json = serde_json::json!({
                "renderer": renderer,
                "componentFramework": "glass-easel",
            });
            if !rule_components.is_empty() {
                let using: serde_json::Map<String, serde_json::Value> = rule_components
                    .iter()
                    .map(|component| {
                        (
                            component.name.to_string(),
                            serde_json::Value::String(format!(
                                "/components/{}/index",
                                component.name
                            )),
                        )
                    })
                    .collect();
                page_json["usingComponents"] = serde_json::Value::Object(using);
            }
            assert_no_renderer_options(&page_json, &format!("页面 json {slug}"))?;
            write(&dir.join("index.json"), &serde_json::to_string(&page_json)?)?;
            pages.push(route.clone());
            cases.push(serde_json::json!({
                "id": meta.id,
                "slug": slug,
                "page": route,
                "expect": case.expect.as_str(),
                "note": case.note,
                "target": case.target,
                "mode": case.mode().as_str(),
            }));
        }
    }

    for component in components.values() {
        let dir = project.join("components").join(component.name);
        write(&dir.join("index.wxml"), component.wxml)?;
        write(&dir.join("index.wxss"), component.wxss)?;
        write(
            &dir.join("index.js"),
            component.js.unwrap_or("Component({})\n"),
        )?;
        write(
            &dir.join("index.json"),
            &format!(r#"{{"component":true,"renderer":"{renderer}"}}"#),
        )?;
    }

    // 先合并规则覆盖，再写生成器管理的必备键：覆盖不能替换页面表与渲染器。
    let mut app = serde_json::Map::new();
    for override_map in app_overrides {
        app.extend(override_map);
    }
    for (key, value) in [
        ("pages", serde_json::json!(pages)),
        ("renderer", serde_json::json!(renderer)),
        ("componentFramework", serde_json::json!("glass-easel")),
        ("lazyCodeLoading", serde_json::json!("requiredComponents")),
    ] {
        app.insert(key.to_string(), value);
    }
    let app = serde_json::Value::Object(app);
    write(
        &project.join("app.json"),
        &serde_json::to_string_pretty(&app)?,
    )?;
    write(
        &out.join("cases.json"),
        &format!("{}\n", serde_json::to_string_pretty(&cases)?),
    )?;

    println!(
        "wrote {} case(s) for renderer={renderer} → {}",
        cases.len(),
        project.display()
    );
    Ok(())
}

fn write(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// 把规则级组件夹具登记进全局表；同名组件内容必须一致。
fn register_components(
    components: &mut BTreeMap<&'static str, &'static VisualComponent>,
    rule_id: &str,
    rule_components: &'static [VisualComponent],
) -> Result<(), Box<dyn std::error::Error>> {
    for component in rule_components {
        validate_component_name(component.name)?;
        match components.get(component.name) {
            Some(existing) if !same_component(existing, component) => {
                return Err(format!(
                    "规则 `{rule_id}` 的组件 `{}` 与其它用例定义不一致（{} vs {}）",
                    component.name, existing.wxml, component.wxml
                )
                .into());
            }
            Some(_) => {}
            None => {
                components.insert(component.name, component);
            }
        }
    }
    Ok(())
}

/// 页面 json 不得写 `rendererOptions`。
///
/// `defaultContentBox` / `defaultDisplayBlock` 保持关闭是好几条用例的前提
/// （`box-sizing`、`flex-direction`、`default-layout-drift` 与三条 `fixed-*`）；
/// 确实要改 app 级配置的规则走 [`skylint::rules::Rule::visual_app_json`]，并自行
/// 保证覆盖值在运行时等价于缺省（例如 `defaultDisplayBlock: null`）。
fn assert_no_renderer_options(
    json: &serde_json::Value,
    what: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if json.get("rendererOptions").is_some() {
        return Err(format!(
            "{what} 不能写 rendererOptions：defaultContentBox / defaultDisplayBlock 关闭是 box-sizing、flex-direction、default-layout-drift 与三条 fixed-* 用例的前提；确有需要时先同步复核这些用例"
        )
        .into());
    }
    Ok(())
}

/// 规则级 app.json 覆盖：必须是 JSON 对象，且不能写生成器管理的键。
fn parse_app_override(
    rule_id: &str,
    text: &'static str,
) -> Result<serde_json::Map<String, serde_json::Value>, Box<dyn std::error::Error>> {
    let value: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| format!("规则 `{rule_id}` 的 visual_app_json 不是合法 JSON：{error}"))?;
    let serde_json::Value::Object(map) = value else {
        return Err(format!("规则 `{rule_id}` 的 visual_app_json 必须是 JSON 对象").into());
    };
    const GENERATED: &[&str] = &["pages", "renderer", "componentFramework", "lazyCodeLoading"];
    if let Some(key) = GENERATED.iter().find(|key| map.contains_key(**key)) {
        return Err(format!(
            "规则 `{rule_id}` 的 visual_app_json 不能写 `{key}`：这些键由生成器写入"
        )
        .into());
    }
    Ok(map)
}

/// 组件标签名会拼进文件路径与 `usingComponents`，只允许安全字符。
fn validate_component_name(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-');
    if valid {
        Ok(())
    } else {
        Err(format!("组件标签名不合法：{name:?}").into())
    }
}

/// 同名组件必须完全一致，否则全局的 `components/<name>/` 无法同时服务两个 case。
fn same_component(a: &VisualComponent, b: &VisualComponent) -> bool {
    a.wxml == b.wxml && a.wxss == b.wxss && a.js == b.js
}

/// 规则 id → 目录名：只保留 `[a-z0-9-]`，保证跨平台可用。
fn slugify(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    let mut pending_dash = false;
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            pending_dash = false;
        } else if !pending_dash && !out.is_empty() {
            out.push('-');
            pending_dash = true;
        }
    }
    out.trim_end_matches('-').to_string()
}
