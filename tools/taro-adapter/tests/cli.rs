use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn project(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("skylint-taro-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn run(root: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_skylint-taro"))
        .arg(root)
        .arg("--json")
        .output()
        .unwrap();
    assert!(
        output.status.code().is_some(),
        "adapter crashed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid json: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn diagnostics_are_remapped_to_tsx() {
    let root = project("remap");
    write(&root, "project.config.json", r#"{"miniprogramRoot":"dist/"}"#);
    write(
        &root,
        "dist/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    );
    write(&root, "dist/base.wxml", "<!-- taro_tmpl -->");
    write(&root, "dist/pages/index/index.json", "{}");
    write(&root, "dist/pages/index/index.wxml", "<!-- taro_tmpl -->");
    write(&root, "dist/pages/index/index.wxss", ".page{color:red}");
    write(
        &root,
        "src/pages/index/index.tsx",
        r#"import { View, ScrollView, WebView } from '@tarojs/components'

export default function Page() {
  return (
    <View className="page">
      <ScrollView scrollX><View>a</View></ScrollView>
      <WebView src="https://example.com" />
    </View>
  )
}
"#,
    );

    let report = run(&root);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let webview = diagnostics
        .iter()
        .find(|item| item["rule"] == "skyline/unsupported-component")
        .expect("WebView 应被报出来");
    assert_eq!(webview["path"], "src/pages/index/index.tsx");
    assert_eq!(webview["line"], 7);
    // Taro 运行时给 scroll-view 补了 type=list，原生语义下不该再报缺失。
    assert!(
        !diagnostics
            .iter()
            .any(|item| item["rule"] == "skyline/scroll-view-type"),
        "{diagnostics:#?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn inlined_component_diagnostics_point_at_the_component() {
    let root = project("inline-remap");
    write(&root, "project.config.json", r#"{"miniprogramRoot":"dist/"}"#);
    write(
        &root,
        "dist/app.json",
        r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
    );
    write(&root, "dist/base.wxml", "<!-- taro_tmpl -->");
    write(&root, "dist/pages/index/index.json", "{}");
    write(&root, "dist/pages/index/index.wxml", "<!-- taro_tmpl -->");
    write(
        &root,
        "src/pages/index/index.tsx",
        r#"import { View } from '@tarojs/components'
import Card from '../../components/Card'

export default function Page() {
  return <View className="page"><Card /></View>
}
"#,
    );
    write(
        &root,
        "src/components/Card.tsx",
        r#"import { View } from '@tarojs/components'

export default function Card() {
  return (
    <View className="card">
      前<View>内层</View>
    </View>
  )
}
"#,
    );

    let report = run(&root);
    let diagnostics = report["diagnostics"].as_array().unwrap();
    let mixed = diagnostics
        .iter()
        .find(|item| item["rule"] == "skyline/no-inline-text")
        .unwrap_or_else(|| panic!("组件里的文本混排应被报出来：{diagnostics:#?}"));
    assert_eq!(mixed["path"], "src/components/Card.tsx");
    assert_eq!(mixed["line"], 5);

    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn missing_build_output_is_a_usage_error() {
    let root = project("no-dist");
    write(&root, "project.config.json", "{}");
    let output = Command::new(env!("CARGO_BIN_EXE_skylint-taro"))
        .arg(&root)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("找不到构建产物"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
}
