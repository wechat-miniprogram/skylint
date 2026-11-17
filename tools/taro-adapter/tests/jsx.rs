use std::collections::HashSet;
use std::path::PathBuf;

use skylint_taro::emit::emit;
use skylint_taro::jsx::parse_page;

fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "skylint-taro-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn synth(dir: &PathBuf, rel: &str, source: &str, declared: &[&str]) -> String {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, source).unwrap();
    let declared: HashSet<String> = declared.iter().map(|name| name.to_string()).collect();
    let parsed = parse_page(dir, &path, source, &declared).unwrap();
    emit(&parsed.roots, parsed.sources).text
}

#[test]
fn scroll_view_gets_runtime_default_type() {
    let dir = fixture("scroll-default");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        "<ScrollView scrollX><View>a</View></ScrollView>",
        &[],
    );
    assert!(wxml.contains(r#"<scroll-view scroll-x type="list">"#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn conditional_branches_and_map_are_expanded() {
    let dir = fixture("branches");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        r#"export default function Page() {
  const list = [1, 2]
  return (
    <View>
      {ok ? <Text className="a">a</Text> : <Text className="b">b</Text>}
      {list.map((item) => <View className="row">{item}</View>)}
    </View>
  )
}"#,
        &[],
    );
    assert!(wxml.contains(r#"<text class="a" wx:if="{{_}}">a</text>"#), "{wxml}");
    assert!(wxml.contains(r#"<text class="b" wx:else>b</text>"#), "{wxml}");
    // 循环只展开一次，重复由 `wx:for` 承载。
    assert_eq!(wxml.matches(r#"class="row""#).count(), 1, "{wxml}");
    assert!(wxml.contains(r#"wx:for="{{_}}""#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn local_component_is_inlined_with_children() {
    let dir = fixture("local");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        r#"const Row = ({ children }) => <View className="row">{children}</View>

export default function Page() {
  return <Row><Text className="inner">x</Text></Row>
}"#,
        &[],
    );
    assert!(wxml.contains(r#"<view class="row"><text class="inner">x</text></view>"#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_component_is_inlined() {
    let dir = fixture("imported");
    std::fs::create_dir_all(dir.join("pages/index")).unwrap();
    std::fs::write(
        dir.join("pages/index/card.tsx"),
        "<View className=\"card\"><Text>{props.title}</Text></View>",
    )
    .unwrap();
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        r#"import Card from './card'

export default function Page() {
  return <Card title="hi" />
}"#,
        &[],
    );
    assert!(wxml.contains(r#"<view class="card"><text>{{_}}</text></view>"#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn barrel_star_and_generic_named_export() {
    let dir = fixture("barrel");
    std::fs::create_dir_all(dir.join("src/components/Ui/Form")).unwrap();
    std::fs::write(
        dir.join("src/components/Ui/index.ts"),
        "export * from './Form'\nexport * from './Tabs'\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src/components/Ui/Form/index.tsx"),
        r#"import { View } from '@tarojs/components'

export const SelectInput = function <T extends { name: string }>({ list }: { list: T[] }) {
  return <View className="select-input">{list.length}</View>
}
"#,
    )
    .unwrap();
    let wxml = synth(
        &dir,
        "src/pages/index/index.tsx",
        r#"import { SelectInput } from '@/src/components/Ui'

export default function Page() {
  return <SelectInput list={[1]} />
}"#,
        &[],
    );
    assert!(wxml.contains(r#"class="select-input""#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn declared_component_stays_a_tag() {
    let dir = fixture("declared");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        "<View><Card title=\"hi\" /></View>",
        &["card"],
    );
    assert!(wxml.contains(r#"<card title="hi" />"#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dynamic_class_does_not_keep_static_prefix() {
    let dir = fixture("class");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        r#"<View className={`box ${active ? 'on' : ''}`} />"#,
        &[],
    );
    // 原生 `class="box {{x}}"` 对 matcher 同样是 Unknown，不能保留静态前缀。
    assert!(wxml.contains(r#"class="{{_}}""#), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nested_condition_around_loop_uses_block() {
    let dir = fixture("nested-loop");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        "<View>{ok && list.map((n) => <Text>{n}</Text>)}</View>",
        &[],
    );
    // `wx:if` 与 `wx:for` 不能叠在同一个元素上：外层包成 `<block wx:if>`。
    assert!(wxml.contains(r#"<block wx:if="{{_}}">"#), "{wxml}");
    assert!(!wxml.contains("wx:for=\"{{_}}\" wx:if"), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn events_become_bind_attributes() {
    let dir = fixture("events");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        "<View onClick={noop} onTouchMove={noop} catchMove />",
        &[],
    );
    assert!(wxml.contains(r#"bindtap="eh""#), "{wxml}");
    assert!(wxml.contains(r#"bindtouchmove="eh""#), "{wxml}");
    // `catchMove` 的语义是阻断 touchmove 冒泡，不是 Taro 编译模式产物里的 `catch-move`。
    assert!(wxml.contains(r#"catchtouchmove="eh""#), "{wxml}");
    assert!(!wxml.contains("onClick") && !wxml.contains("catch-move"), "{wxml}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn style_object_is_folded() {
    let dir = fixture("style");
    let wxml = synth(
        &dir,
        "pages/index/index.tsx",
        r#"<View style={{ position: 'fixed', width: 100 }} />"#,
        &[],
    );
    assert!(
        wxml.contains(r#"style="position:fixed;width:100px;""#),
        "{wxml}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
