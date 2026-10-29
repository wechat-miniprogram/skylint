//! WXML 视图。
//!
//! 只做「解析 + 定位」：把 glass-easel 的模板 AST 摊平成规则能直接遍历的树，
//! 并把它的 `(line, utf16_col)` 位置换算成诊断使用的「1-based 行 + 按 Unicode
//! 字符计的列」。
//!
//! 结构包装节点（`<block>`、`wx:for`、`wx:if`）在渲染上不产生自己的盒子，因此
//! 这里把它们直接摊平进父节点的子节点列表——规则看到的父子关系与渲染树一致，
//! `<block wx:for>` 里的裸文本会落在真正包含它的元素之下。

use std::ops::Range;

use glass_easel_template_compiler::parse::expr::Expression;
use glass_easel_template_compiler::parse::tag::{
    Attribute, ClassAttribute, Element, ElementKind, EventBinding, Ident, Node, NormalAttribute,
    NormalAttributePrefix, StaticAttribute, StyleAttribute, Value,
};
use glass_easel_template_compiler::parse::{self, ParseErrorLevel, Position, TemplateStructure};

use crate::input::env::{Env, Lit};

/// 解析一个 WXML 文件。`env` 是这份文件里确定取值的绑定，用于折叠 `wx:if`。
pub fn parse(path: &str, source: &str, env: &Env) -> Result<WxmlDoc, String> {
    WxmlDoc::parse(path, source, env)
}

/// 一个 `<template is="x" data="…">` 调用点。
///
/// 模板被摊平后调用元素不在 `nodes` 里（展开内容静态不可知），需要这份单独记录才能
/// 回答「这个调用给了模板体哪些数据」。
#[derive(Debug, Clone)]
pub struct TemplateCall {
    /// 静态模板名；`is="{{…}}"` 这种动态引用不记录。
    pub name: String,
    /// `data` 是对象字面量时的顶层键；不是对象字面量（含 spread、变量）时为 `None`。
    pub data_keys: Option<Vec<String>>,
    /// 这个调用点本身写在哪个模板的定义体里（顶层调用为 `None`）。
    pub template: Option<String>,
    pub start: Position,
    pub end: Position,
}

/// 一个 WXML 文件的解析结果。
#[derive(Debug)]
pub struct WxmlDoc {
    path: String,
    source: String,
    pub nodes: Vec<WxmlNode>,
    /// `<import>` / `<include>` 的 `src` 原始值（编译器已剥掉 `.wxml` 后缀）。
    template_refs: Vec<String>,
    /// `<wxs src>` 的原始值（`Script::GlobalRef`）。
    wxs_refs: Vec<String>,
    /// `<template is="x" data="…">` 调用点（按源码顺序）。
    pub template_calls: Vec<TemplateCall>,
    /// 这份文件不是单元本身，而是被 `<import>` / `<include>` 引用的片段：
    /// 它的根节点在实例化处的父节点静态不可知。
    pub fragment: bool,
}

/// 摊平后的节点。`Comment` 不携带内容：规则不需要，保留它是为了不把注释误当元素。
#[derive(Debug, Clone)]
pub enum WxmlNode {
    Element(WxmlElement),
    Text(WxmlText),
    Comment,
}

#[derive(Debug, Clone)]
pub struct WxmlElement {
    /// 标签名，小写（与 WXML 解析器一致）。
    pub tag: String,
    /// 整个元素（含子节点）的范围。
    pub start: Position,
    pub end: Position,
    /// 开始标签里标签名的范围，用于把诊断落在标签名上。
    pub tag_start: Position,
    pub tag_end: Position,
    pub attributes: Vec<WxmlAttribute>,
    pub children: Vec<WxmlNode>,
    /// 元素所处的条件分支栈，从外到内；顶层元素为空。
    ///
    /// 嵌套 `wx:if` 时会叠加：外层分支与外层 `else` 里的元素各带一条栈，内层元素
    /// 在栈顶再加自己那一层，这样「互斥」判定不会因为嵌套而丢失外层信息。
    pub branches: Vec<Branch>,
    /// 元素来自内联 `<template name="x">` 的定义体，值为模板名。
    ///
    /// 定义体只在被 `<template is="x"/>` 实例化的位置渲染，真实祖先 / 兄弟结构
    /// 静态不可知：标签与属性类规则照常检查，依赖祖先或子节点计数的结构规则
    /// （scroll-view 家族、组件子节点、key 唯一性）必须跳过。
    pub template: Option<String>,
    /// 子节点里有被摊平掉的 `<template is>` / `<include>`：展开后的内容静态不可知。
    pub unknown_children: bool,
    /// 元素是 `<slot/>`：运行时的子节点由父级提供，写在使用它的文件里，
    /// 静态看不到。匹配层据此把 slot 位置当成「子节点未知」的区域。
    pub is_slot: bool,
    /// `<slot name="x">` 的具名槽名；默认槽与普通元素为 `None`。
    ///
    /// glass-easel 把 `name` 从普通属性里摘出去了，只能从 `ElementKind::Slot` 取。
    pub slot_name: Option<String>,
}

impl WxmlElement {
    /// 元素（或它的任一祖先）处在条件里用到 `renderer` 的分支里。
    /// 元素是否来自内联 `<template name>` 的定义体。
    pub fn in_template(&self) -> bool {
        self.template.is_some()
    }

    pub fn renderer_guarded(&self) -> bool {
        self.branches.iter().any(|branch| branch.renderer_guarded)
    }

    /// 这个元素只在某个渲染器下渲染（祖先分支已经把另一侧排除掉）。
    pub fn renderer_arm(&self) -> RendererArm {
        self.branches.iter().fold(RendererArm::Both, |acc, branch| {
            match (acc, branch.renderer) {
                (RendererArm::Both, arm) => arm,
                (arm, RendererArm::Both) => arm,
                // 嵌套条件互相矛盾时静态判不了，保持较窄的那个不一致即可：按「都可能在」处理。
                _ => acc,
            }
        })
    }

    /// 这个元素在 Skyline 下永远不会渲染。
    pub fn webview_only(&self) -> bool {
        self.renderer_arm() == RendererArm::Webview
    }

    /// 这个元素在 WebView 下永远不会渲染（只存在于 Skyline 分支）。
    pub fn skyline_only(&self) -> bool {
        self.renderer_arm() == RendererArm::Skyline
    }

    /// 这个元素是否会因渲染后端而变：处在按 `renderer` 分支的条件里，或者某个属性的值
    /// 绑定了 `renderer`（`class="x {{renderer}}"`、`renderer="{{renderer}}"`）。
    pub fn mentions_renderer(&self) -> bool {
        self.renderer_guarded()
            || self
                .attributes
                .iter()
                .any(|attribute| attribute.references_renderer)
    }
}

/// 一个条件分支的标识：同一个 `wx:if` 节点的所有分支共享 `group`，`arm` 是分支序号
/// （`wx:if` 为 0，`wx:elif` 依次递增，`else` 排在它们之后）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    pub group: u32,
    pub arm: u32,
    /// 这一支的条件里用到了 `renderer`：整支只在特定渲染器下渲染。
    /// 反向（WebView）规则据此跳过作者已经做过两端分支的代码。
    pub renderer_guarded: bool,
    /// 这一支在哪个渲染器下可能渲染（条件按 `renderer` 取值折叠后）。
    pub renderer: RendererArm,
    /// 这一支的条件引用的数据字段（模板数据作用域规则需要知道条件吃了哪些键）。
    pub fields: Vec<String>,
}

/// 一个条件分支在两种渲染器下的可达性。
///
/// `wx:if="{{renderer === 'skyline'}}"` 的 `wx:else` 只会在 WebView 下渲染，
/// 对它讲「Skyline 下会怎样」没有意义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererArm {
    Both,
    Skyline,
    Webview,
}

/// 元素上的一个属性。
///
/// `value` 是解析后的静态字符串（含实体解码）；动态绑定（`{{}}`，或
/// `a{{b}}c` 这类混排）不提供字符串值，`is_static` 为 `false`，规则据此跳过。
/// 没有值的属性（如 `disabled`）`value` 为 `None`，但 `is_static` 仍为 `true`。
#[derive(Debug, Clone)]
pub struct WxmlAttribute {
    /// 属性原名，保留连字符与冒号前缀：`aria-role`、`bind:tap`、`catch:tap`。
    pub name: String,
    pub value: Option<String>,
    pub is_static: bool,
    /// 动态绑定的 class 词元信息：确定出现 / 可能出现的词，是否还可能出现任意词。
    /// 只有 `class` 且取值动态时才有值。
    pub class_tokens: Option<crate::input::expr::ClassTokens>,
    /// 动态绑定的可能取值（`class` 之外）。`None` 表示取值依赖运行时数据、
    /// 或候选太多枚举不出来；只有表达式由字面量 / 三元 / 与或 / 拼接构成时才有值。
    pub candidates: Option<Vec<String>>,
    /// 属性值里引用了 `renderer`：这个属性的取值跟着渲染后端变。
    pub references_renderer: bool,
    /// 动态绑定的表达式原文，静态 / 无值属性为 `None`。
    ///
    /// 保留整棵表达式树而不是预先抽字段：规则需要的东西各不相同（数据字段、对象键、
    /// 是否引用某字段），让规则自己用 `input::expr` 里的辅助函数取。
    pub expression: Option<Expression>,
    /// 属性名（含前缀）到值结尾的范围。
    pub start: Position,
    pub end: Position,
    /// 值的起点（开引号后第一个字符）；没有值时为 `None`。
    ///
    /// 只有 `start..end` 时规则只能落在属性名上，而 `worklet:ongesture="onPan"`
    /// 这类诊断要指向被绑定的函数名。
    pub value_start: Option<Position>,
}

#[derive(Debug, Clone)]
pub struct WxmlText {
    /// 文本内容；静态文本是解码后的值，插值是原文（含 `{{}}`）。
    pub text: String,
    /// 插值的表达式（静态文本为 `None`）。
    pub expression: Option<Expression>,
    pub start: Position,
    pub end: Position,
    /// 文本所处的条件分支栈（`<block wx:if>` 摊平后仍保留）。
    pub branches: Vec<Branch>,
    /// 文本来自内联 `<template name>` 的定义体时，记下模板名。
    pub template: Option<String>,
}

impl WxmlText {
    /// 空白文本不算「有意义的文本」。
    pub fn is_meaningful(&self) -> bool {
        !self.text.trim().is_empty()
    }

    /// 插值里引用到的数据字段（静态文本为空）。
    pub fn data_fields(&self) -> Vec<String> {
        self.expression
            .as_ref()
            .map(crate::input::expr::data_fields)
            .unwrap_or_default()
    }
}

impl WxmlDoc {
    /// `path` 只用于错误消息，不需要是磁盘上的真实路径。
    pub fn parse(path: &str, source: &str, env: &Env) -> Result<Self, String> {
        let source = crate::input::strip_bom(source);
        let (template, state) = parse::parse(path, source);
        if let Some(err) = state
            .warnings()
            .find(|e| e.level() >= ParseErrorLevel::Fatal)
        {
            let start = err.location.start;
            return Err(format!(
                "{}:{}:{}: {}",
                path,
                start.line + 1,
                column_at(source, start),
                err.kind
            ));
        }
        let mut next_group = 0;
        let mut template_calls = Vec::new();
        let mut nodes = flatten(
            &template.content,
            source,
            env,
            &mut next_group,
            &[],
            None,
            &mut template_calls,
        );
        // 内联 `<template name="x">` 的定义体只存在 `globals.sub_templates` 里，不在
        // `content` 中：不摊平的话其中的元素对所有规则都不可见。
        // 定义体的数据作用域是 `<template is data="…">` 新传的对象，看不到组件的
        // data / properties（proc_gen/tag.rs:1275-1287），因此这里不能代入单元 env。
        let empty = Env::default();
        for definition in &template.globals.sub_templates {
            nodes.extend(flatten(
                &definition.content,
                source,
                &empty,
                &mut next_group,
                &[],
                Some(definition.name.name.as_str()),
                &mut template_calls,
            ));
        }
        let template_refs = template
            .globals
            .imports
            .iter()
            .map(|element| element.src.name.to_string())
            .chain(
                template
                    .globals
                    .includes
                    .iter()
                    .map(|element| element.src.name.to_string()),
            )
            // 动态 `src="{{x}}"` 解析成空串，不是可解析的模板引用。
            .filter(|name| !name.is_empty())
            .collect();
        let wxs_refs = template
            .globals
            .scripts
            .iter()
            .filter_map(|script| match script {
                glass_easel_template_compiler::parse::tag::Script::GlobalRef { src, .. } => {
                    Some(src.name.to_string())
                }
                glass_easel_template_compiler::parse::tag::Script::Inline { .. } | &_ => None,
            })
            .filter(|name| !name.is_empty())
            .collect();
        Ok(Self {
            path: path.to_string(),
            source: source.to_string(),
            nodes,
            template_refs,
            wxs_refs,
            template_calls,
            // 由调用方（`load_doc`）按工程视图补上。
            fragment: false,
        })
    }

    /// 本文件的项目相对路径。
    pub fn path(&self) -> &str {
        &self.path
    }

    /// `parse::Position`（0-based 行、UTF-16 列）→ 诊断位置（1-based 行、字符列）。
    pub fn position(&self, pos: Position) -> (u32, u32) {
        (pos.line + 1, column_at(&self.source, pos))
    }

    /// 本文件通过 `<import>` / `<include>` 引用的模板路径（原始 `src` 值，未解析）。
    ///
    /// 编译器在解析时已剥掉 `.wxml` 后缀，所以这里拿到的通常是不带后缀的路径。
    pub fn template_refs(&self) -> &[String] {
        &self.template_refs
    }

    /// 本文件通过 `<wxs src>` 引入的脚本路径（编译器已补 `.wxs` 后缀）。
    pub fn wxs_refs(&self) -> &[String] {
        &self.wxs_refs
    }

    /// 深度优先遍历全部元素节点。
    pub fn elements(&self) -> impl Iterator<Item = &WxmlElement> {
        let mut stack: Vec<&WxmlNode> = self.nodes.iter().rev().collect();
        std::iter::from_fn(move || {
            while let Some(node) = stack.pop() {
                if let WxmlNode::Element(element) = node {
                    stack.extend(element.children.iter().rev());
                    return Some(element);
                }
            }
            None
        })
    }
}

/// 1-based 字符列。`utf16_col` 落在代理对中间时按该字符计入，不会越界。
fn column_at(source: &str, pos: Position) -> u32 {
    let line = source.split('\n').nth(pos.line as usize).unwrap_or("");
    let mut utf16 = 0u32;
    let mut chars = 0u32;
    for ch in line.chars() {
        if utf16 >= pos.utf16_col {
            break;
        }
        utf16 += ch.len_utf16() as u32;
        chars += 1;
    }
    chars + 1
}

/// 位置 → 字节偏移，用于从原文切出插值文本。
fn byte_offset(source: &str, pos: Position) -> Option<usize> {
    let mut offset = 0;
    for (index, line) in source.split('\n').enumerate() {
        if index as u32 == pos.line {
            let mut utf16 = 0u32;
            for (bytes, ch) in line.char_indices() {
                if utf16 >= pos.utf16_col {
                    return Some(offset + bytes);
                }
                utf16 += ch.len_utf16() as u32;
            }
            return Some(offset + line.len());
        }
        offset += line.len() + 1;
    }
    None
}

fn text_node(value: &Value, source: &str, branches: &[Branch], template: Option<&str>) -> WxmlText {
    let range = value.location();
    let (text, expression, start, end) = match value {
        Value::Static { value: raw, .. } => {
            // 解析器把文本前的空白也包进了 Value，这里裁掉并把起点前移，
            // 让诊断落在真正的第一个字符上。
            let trimmed = raw.trim();
            let lead = &raw[..raw.len() - raw.trim_start().len()];
            let start = advance(range.start, lead);
            (trimmed.to_string(), None, start, advance(start, trimmed))
        }
        Value::Dynamic { expression, .. } => (
            slice(source, range.clone()),
            Some((**expression).clone()),
            range.start,
            range.end,
        ),
        _ => (slice(source, range.clone()), None, range.start, range.end),
    };
    WxmlText {
        text,
        expression,
        template: template.map(str::to_string),
        start,
        end,
        branches: branches.to_vec(),
    }
}

fn advance(mut pos: Position, text: &str) -> Position {
    for ch in text.chars() {
        if ch == '\n' {
            pos.line += 1;
            pos.utf16_col = 0;
        } else {
            pos.utf16_col += ch.len_utf16() as u32;
        }
    }
    pos
}

fn slice(source: &str, range: Range<Position>) -> String {
    let (Some(start), Some(end)) = (
        byte_offset(source, range.start),
        byte_offset(source, range.end),
    ) else {
        return String::new();
    };
    source.get(start..end).unwrap_or("").to_string()
}

/// 包装节点摊平；元素保留。
///
/// `next_group` 为文件内递增的条件组编号；`branches` 是当前所处的条件分支栈，
/// 由包装节点透传给摊平后的元素。
fn flatten(
    nodes: &[Node],
    source: &str,
    env: &Env,
    next_group: &mut u32,
    branches: &[Branch],
    template: Option<&str>,
    calls: &mut Vec<TemplateCall>,
) -> Vec<WxmlNode> {
    nodes
        .iter()
        .flat_map(|node| node_nodes(node, source, env, next_group, branches, template, calls))
        .collect()
}

fn node_nodes(
    node: &Node,
    source: &str,
    env: &Env,
    next_group: &mut u32,
    branches: &[Branch],
    template: Option<&str>,
    calls: &mut Vec<TemplateCall>,
) -> Vec<WxmlNode> {
    match node {
        Node::Text(value) => vec![WxmlNode::Text(text_node(value, source, branches, template))],
        Node::Element(element) => {
            element_nodes(element, source, env, next_group, branches, template, calls)
        }
        Node::Comment(_) => vec![WxmlNode::Comment],
        _ => Vec::new(),
    }
}

fn element_nodes(
    element: &Element,
    source: &str,
    env: &Env,
    next_group: &mut u32,
    branches: &[Branch],
    template: Option<&str>,
    calls: &mut Vec<TemplateCall>,
) -> Vec<WxmlNode> {
    match &element.kind {
        ElementKind::Normal {
            tag_name, children, ..
        } => vec![WxmlNode::Element(WxmlElement {
            tag: tag_name.name.to_string(),
            start: element.location().start,
            end: element.location().end,
            tag_start: tag_name.location.start,
            tag_end: tag_name.location.end,
            attributes: attributes(element, source, env),
            unknown_children: has_unknown_children(children),
            children: flatten(children, source, env, next_group, branches, template, calls),
            branches: branches.to_vec(),
            template: template.map(str::to_string),
            is_slot: false,
            slot_name: None,
        })],
        // 透明容器：不产生条件组，但把外层的分支栈透传下去。
        ElementKind::Pure { children, .. } | ElementKind::For { children, .. } => {
            flatten(children, source, env, next_group, branches, template, calls)
        }
        ElementKind::If {
            branches: arms,
            else_branch,
            ..
        } => {
            let group = *next_group;
            *next_group += 1;
            // 只有**自己这一支的条件**引用了 renderer，这一支才算按 renderer 分叉；
            // `wx:if="{{a}}"` 与 `wx:elif="{{renderer === 'skyline'}}"` 同链时，前者
            // 在两个渲染器下都会渲染，不能因为同链有 renderer 就被一起消音。
            // `else` 是「前面都不成立」的补集：链上有 renderer 条件时它必然是另一侧。
            let chain_guarded = arms
                .iter()
                .any(|(_, condition, _)| condition_mentions_renderer(condition));
            // 用 `renderer` 的两个取值各求一次值：哪些 arm 在该渲染器下可能渲染。
            // `else` 是「前面都不成立」的补集，按链上结果算。
            let reachable = |renderer: &str| {
                let mut probe = env.clone();
                probe.insert("renderer", Lit::Str(renderer.to_string()));
                let mut out = Vec::with_capacity(arms.len() + 1);
                let mut decided = false;
                for (_, condition, _) in arms {
                    if decided {
                        out.push(false);
                        continue;
                    }
                    match condition_truthy(condition, &probe) {
                        Some(true) => {
                            decided = true;
                            out.push(true);
                        }
                        Some(false) => out.push(false),
                        None => out.push(true),
                    }
                }
                out.push(!decided);
                out
            };
            let skyline_arms = reachable("skyline");
            let webview_arms = reachable("webview");
            let arm_renderer = |arm: usize| match (skyline_arms[arm], webview_arms[arm]) {
                (true, true) => RendererArm::Both,
                (true, false) => RendererArm::Skyline,
                (false, true) => RendererArm::Webview,
                (false, false) => RendererArm::Both,
            };
            let inner = |arm: u32, guarded: bool, fields: Vec<String>| {
                let mut stack = branches.to_vec();
                stack.push(Branch {
                    group,
                    arm,
                    renderer_guarded: guarded,
                    renderer: arm_renderer(arm as usize),
                    fields,
                });
                stack
            };
            let mut out = Vec::new();
            // 条件能折叠时只留下可能渲染的分支：`{{renderer === 'skyline'}}` 在
            // `renderer` 确定时整条分支链就只剩一支，另一支里的元素不再参与任何规则
            // （否则「专门为 Skyline 改的分支」和「WebView 守卫分支」都会被当成可能渲染）。
            let mut decided = false;
            for (arm, (_, condition, children)) in arms.iter().enumerate() {
                if decided {
                    break;
                }
                match condition_truthy(condition, env) {
                    Some(true) => decided = true,
                    Some(false) => continue,
                    None => {}
                }
                out.extend(flatten(
                    children,
                    source,
                    env,
                    next_group,
                    &inner(
                        arm as u32,
                        condition_mentions_renderer(condition),
                        condition_fields(condition),
                    ),
                    template,
                    calls,
                ));
            }
            if !decided && let Some((_, children)) = else_branch {
                out.extend(flatten(
                    children,
                    source,
                    env,
                    next_group,
                    &inner(arms.len() as u32, chain_guarded, Vec::new()),
                    template,
                    calls,
                ));
            }
            out
        }
        ElementKind::Slot { name, .. } => vec![WxmlNode::Element(WxmlElement {
            tag: "slot".to_string(),
            start: element.location().start,
            end: element.location().end,
            tag_start: element.tag_location.start.0.start,
            tag_end: element.tag_location.start.1.end,
            attributes: attributes(element, source, env),
            children: Vec::new(),
            unknown_children: false,
            branches: branches.to_vec(),
            template: template.map(str::to_string),
            is_slot: true,
            slot_name: match &name.1 {
                Value::Static { value, .. } if !value.is_empty() => Some(value.to_string()),
                _ => None,
            },
        })],
        ElementKind::TemplateRef { target, data, .. } => {
            // 动态 `<template is="{{…}}">` 解析不出目标，跳过。
            let Value::Static { value: name, .. } = &target.1 else {
                return Vec::new();
            };
            if name.is_empty() {
                return Vec::new();
            }
            calls.push(TemplateCall {
                name: name.to_string(),
                data_keys: match &data.1 {
                    Value::Dynamic { expression, .. } => {
                        crate::input::expr::object_keys(expression)
                    }
                    // 没写 `data`：模板体拿到的是空对象。
                    Value::Static { value, .. } if value.is_empty() => Some(Vec::new()),
                    _ => None,
                },
                template: template.map(str::to_string),
                start: element.tag_location.start.0.start,
                end: element.tag_location.start.1.end,
            });
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// `wx:if` 条件在已知环境下的取值；`None` = 判不了（含静态文本条件写法）。
fn condition_truthy(condition: &Value, env: &Env) -> Option<bool> {
    match condition {
        Value::Dynamic { expression, .. } => crate::input::expr::truthy(expression, env),
        // 静态条件在编译产物里是**字符串字面量**：`wx:if="false"` 编成 `"false" ? 1 : 0`，
        // 字符串为真，分支会渲染（proc_gen/tag.rs:1016-1019 的 `gen_lit_str`）。
        // 只有空值等价于「没写这个属性」。
        Value::Static { value, .. } => Some(!value.is_empty()),
        _ => None,
    }
}

/// 条件表达式引用的数据字段；静态条件没有。
fn condition_fields(condition: &Value) -> Vec<String> {
    match condition {
        Value::Dynamic { expression, .. } => crate::input::expr::data_fields(expression),
        _ => Vec::new(),
    }
}

/// 条件里是否引用了 `renderer`：这是工程里唯一能静态认出的渲染器分支写法。
fn condition_mentions_renderer(condition: &Value) -> bool {
    match condition {
        Value::Dynamic { expression, .. } => crate::input::expr::references(expression, "renderer"),
        _ => false,
    }
}

/// 这棵子树里是否有被丢弃的 `<template is>` / `<include>`：展开内容静态不可知。
fn has_unknown_children(nodes: &[Node]) -> bool {
    nodes.iter().any(|node| match node {
        Node::Element(element) => match &element.kind {
            ElementKind::TemplateRef { .. } | ElementKind::Include { .. } => true,
            ElementKind::Normal { children, .. }
            | ElementKind::Pure { children, .. }
            | ElementKind::For { children, .. } => has_unknown_children(children),
            ElementKind::If {
                branches,
                else_branch,
                ..
            } => {
                branches
                    .iter()
                    .any(|(_, _, children)| has_unknown_children(children))
                    || else_branch
                        .as_ref()
                        .is_some_and(|(_, children)| has_unknown_children(children))
            }
            _ => false,
        },
        _ => false,
    })
}

/// 元素属性。只收集「用户写在开始标签里」的属性：`wx:for`/`wx:if` 被解析成
/// 包装节点，不在其内；`<block>` 摊平后属性也不会保留。
fn attributes(element: &Element, source: &str, env: &Env) -> Vec<WxmlAttribute> {
    let mut out = Vec::new();
    match &element.kind {
        ElementKind::Normal {
            attributes: normal,
            class,
            style,
            change_attributes,
            worklet_attributes,
            generics,
            extra_attr,
            let_vars,
            common,
            ..
        } => {
            out.extend(
                normal
                    .iter()
                    .map(|attr| normal_attribute(attr, source, env)),
            );
            out.extend(class_attributes(class, source, env));
            out.extend(style_attributes(style, source, env));
            out.extend(
                change_attributes
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source, env)),
            );
            out.extend(
                worklet_attributes
                    .iter()
                    .map(|attr| static_attribute(attr, source)),
            );
            out.extend(generics.iter().map(|attr| static_attribute(attr, source)));
            out.extend(extra_attr.iter().map(|attr| static_attribute(attr, source)));
            out.extend(
                let_vars
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source, env)),
            );
            if let Some((range, value)) = &common.id {
                out.push(value_attribute("id".to_string(), range, Some(value), env));
            }
            if let Some((range, value)) = &common.slot {
                out.push(value_attribute("slot".to_string(), range, Some(value), env));
            }
            out.extend(
                common
                    .slot_value_refs
                    .iter()
                    .map(|attr| static_attribute(attr, source)),
            );
            out.extend(
                common
                    .data
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source, env)),
            );
            out.extend(
                common
                    .marks
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source, env)),
            );
            out.extend(
                common
                    .event_bindings
                    .iter()
                    .map(|binding| event_attribute(binding, source, env)),
            );
        }
        ElementKind::Slot { values, common, .. } => {
            out.extend(
                values
                    .iter()
                    .map(|attr| prefixed_attribute(attr, source, env)),
            );
            if let Some((range, value)) = &common.id {
                out.push(value_attribute("id".to_string(), range, Some(value), env));
            }
            out.extend(
                common
                    .event_bindings
                    .iter()
                    .map(|binding| event_attribute(binding, source, env)),
            );
        }
        _ => {}
    }
    // 按源码顺序排列，便于规则和调试时阅读。
    out.sort_by_key(|attr| (attr.start.line, attr.start.utf16_col));
    out
}

fn value_attribute(
    name: String,
    range: &Range<Position>,
    value: Option<&Value>,
    env: &Env,
) -> WxmlAttribute {
    match value {
        Some(Value::Static {
            value, location, ..
        }) => WxmlAttribute {
            name,
            value: Some(value.to_string()),
            is_static: true,
            class_tokens: None,
            candidates: None,
            references_renderer: false,
            expression: None,
            start: range.start,
            end: location.end,
            value_start: Some(location.start),
        },
        Some(dynamic @ Value::Dynamic { expression, .. }) => WxmlAttribute {
            class_tokens: name
                .eq_ignore_ascii_case("class")
                .then(|| crate::input::expr::class_tokens(expression, env)),
            candidates: crate::input::expr::candidates(expression, env),
            references_renderer: crate::input::expr::references(expression, "renderer"),
            expression: Some((**expression).clone()),
            name,
            value: None,
            is_static: false,
            start: range.start,
            end: dynamic.location().end,
            value_start: Some(dynamic.location().start),
        },
        // 解析器新增的值形态按「动态」处理，规则不会把它当成静态字符串。
        Some(_) => WxmlAttribute {
            name,
            value: None,
            is_static: false,
            class_tokens: None,
            candidates: None,
            references_renderer: false,
            expression: None,
            start: range.start,
            end: range.end,
            value_start: None,
        },
        None => WxmlAttribute {
            name,
            value: None,
            is_static: true,
            class_tokens: None,
            candidates: None,
            references_renderer: false,
            expression: None,
            start: range.start,
            end: range.end,
            value_start: None,
        },
    }
}

fn normal_attribute(attr: &NormalAttribute, source: &str, env: &Env) -> WxmlAttribute {
    let prefix = match &attr.prefix {
        NormalAttributePrefix::None => None,
        NormalAttributePrefix::Model(range) => Some(range),
    };
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    value_attribute(
        name,
        &(start..attr.name.location.end),
        attr.value.as_ref(),
        env,
    )
}

fn prefixed_attribute(attr: &Attribute, source: &str, env: &Env) -> WxmlAttribute {
    let prefix = attr.prefix_location.as_ref();
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    value_attribute(
        name,
        &(start..attr.name.location.end),
        attr.value.as_ref(),
        env,
    )
}

fn static_attribute(attr: &StaticAttribute, source: &str) -> WxmlAttribute {
    let prefix = attr.prefix_location.as_ref();
    let name = full_name(prefix, &attr.name, source);
    let start = prefix.map_or(attr.name.location.start, |range| range.start);
    WxmlAttribute {
        name,
        value: Some(attr.value.name.to_string()),
        is_static: true,
        class_tokens: None,
        candidates: None,
        references_renderer: false,
        expression: None,
        start,
        end: attr.value.location.end,
        value_start: Some(attr.value.location.start),
    }
}

fn event_attribute(binding: &EventBinding, source: &str, env: &Env) -> WxmlAttribute {
    let name = full_name(Some(&binding.prefix_location), &binding.name, source);
    value_attribute(
        name,
        &(binding.prefix_location.start..binding.name.location.end),
        binding.value.as_ref(),
        env,
    )
}

fn class_attributes(class: &ClassAttribute, source: &str, env: &Env) -> Vec<WxmlAttribute> {
    match class {
        ClassAttribute::None => Vec::new(),
        ClassAttribute::String(range, value) => {
            vec![value_attribute(
                "class".to_string(),
                range,
                Some(value),
                env,
            )]
        }
        ClassAttribute::Multiple(items) => items
            .iter()
            .map(|(range, name, value)| {
                let full = full_name(Some(range), name, source);
                let range = range.start..name.location.end;
                value_attribute(full, &range, value.as_ref(), env)
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn style_attributes(style: &StyleAttribute, source: &str, env: &Env) -> Vec<WxmlAttribute> {
    match style {
        StyleAttribute::None => Vec::new(),
        StyleAttribute::String(range, value) => {
            vec![value_attribute(
                "style".to_string(),
                range,
                Some(value),
                env,
            )]
        }
        StyleAttribute::Multiple(items) => items
            .iter()
            .map(|(range, name, value)| {
                let full = full_name(Some(range), name, source);
                let range = range.start..name.location.end;
                value_attribute(full, &range, Some(value), env)
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// 带前缀的属性名（`bind:tap`、`model:value`、`mark:foo`）从原文切出，
/// 前缀与名字之间的冒号因此得以保留。
fn full_name(prefix: Option<&Range<Position>>, name: &Ident, source: &str) -> String {
    match prefix {
        Some(prefix) => slice(source, prefix.start..name.location.end),
        // 模板编译器会改写属性名：`data-theme` 被剥成 `theme`、自定义组件插槽上的
        // 连字符名被驼峰化。名字以原文为准，位置区间始终指向原始拼写。
        None => {
            let raw = slice(source, name.location.start..name.location.end);
            if raw.is_empty() {
                name.name.to_string()
            } else {
                raw
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 测试默认在「没有常量环境」下解析。
    fn doc_of(source: &str) -> WxmlDoc {
        WxmlDoc::parse("t.wxml", source, &Env::default()).expect("parse")
    }

    fn text_nodes(doc: &WxmlDoc) -> Vec<&WxmlText> {
        fn walk<'a>(nodes: &'a [WxmlNode], out: &mut Vec<&'a WxmlText>) {
            for node in nodes {
                match node {
                    WxmlNode::Text(text) => out.push(text),
                    WxmlNode::Element(element) => walk(&element.children, out),
                    WxmlNode::Comment => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&doc.nodes, &mut out);
        out
    }

    #[test]
    fn branches_record_which_renderer_can_reach_them() {
        let doc = doc_of(
            r#"<view wx:if="{{renderer === 'skyline'}}"><text/></view><view wx:else><text/></view>"#,
        );
        let arms: Vec<RendererArm> = doc
            .nodes
            .iter()
            .filter_map(|node| match node {
                WxmlNode::Element(element) => Some(element.renderer_arm()),
                _ => None,
            })
            .collect();
        assert_eq!(arms, vec![RendererArm::Skyline, RendererArm::Webview]);
        let else_element = doc
            .nodes
            .iter()
            .filter_map(|node| match node {
                WxmlNode::Element(element) => Some(element),
                _ => None,
            })
            .nth(1)
            .unwrap();
        assert!(else_element.webview_only());
    }

    #[test]
    fn unrelated_condition_reaches_both_renderers() {
        let doc = doc_of(r#"<view wx:if="{{a}}"><text/></view><view wx:else><text/></view>"#);
        for node in &doc.nodes {
            if let WxmlNode::Element(element) = node {
                assert_eq!(element.renderer_arm(), RendererArm::Both);
            }
        }
    }

    #[test]
    fn ascii_columns_are_one_based() {
        let doc = doc_of("<view>hi</view>");
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(doc.position(text.end), (1, 9));
    }

    #[test]
    fn emoji_columns_count_characters_not_utf16_units() {
        // `<view>` = 6 个 UTF-16 单元；`😀` 占 2 个单元但只算 1 列。
        let doc = doc_of("<view>😀hi</view>");
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(
            doc.position(Position {
                line: 0,
                utf16_col: 8
            }),
            (1, 8)
        );
        assert_eq!(doc.position(text.end), (1, 10));
    }

    #[test]
    fn chinese_columns_are_character_based() {
        let doc = doc_of("<view>中文</view>");
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (1, 7));
        assert_eq!(doc.position(text.end), (1, 9));
    }

    #[test]
    fn multiline_positions() {
        let doc = doc_of("<view>\n  <text>a</text>\n</view>");
        let text = text_nodes(&doc)[0];
        assert_eq!(doc.position(text.start), (2, 9));
    }

    #[test]
    fn interpolation_keeps_source_text() {
        let doc = doc_of("<view>{{ a }}</view>");
        let text = text_nodes(&doc)[0];
        assert_eq!(text.text, "{{ a }}");
        assert!(text.is_meaningful());
    }

    #[test]
    fn fatal_parse_error_is_reported_with_position() {
        let err = WxmlDoc::parse("t.wxml", "<view>{{ a</view>", &Env::default()).unwrap_err();
        assert!(err.starts_with("t.wxml:1:"), "{err}");
    }

    #[test]
    fn non_fatal_warnings_do_not_fail() {
        // 缺少结束标签只是 Warn。
        assert!(WxmlDoc::parse("t.wxml", "<view>", &Env::default()).is_ok());
    }

    fn attributes(doc: &WxmlDoc) -> Vec<&WxmlAttribute> {
        doc.elements().next().unwrap().attributes.iter().collect()
    }

    #[test]
    fn attribute_names_keep_hyphens_and_colons() {
        let doc = doc_of(
            r#"<view aria-role="button" bind:tap="onTap" catch:tap="onCatch" type="list"/>"#,
        );
        let names: Vec<&str> = attributes(&doc).iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"aria-role"), "{names:?}");
        assert!(names.contains(&"bind:tap"), "{names:?}");
        assert!(names.contains(&"catch:tap"), "{names:?}");

        let aria = attributes(&doc)
            .into_iter()
            .find(|a| a.name == "aria-role")
            .unwrap();
        assert_eq!(aria.value.as_deref(), Some("button"));
        assert!(aria.is_static);
    }

    #[test]
    fn worklet_attribute_keeps_its_colon_prefix_and_value_span() {
        let doc = doc_of(r#"<view worklet:ongesture="onPan" bind:ongesture="x"/>"#);
        let attrs = attributes(&doc);
        let worklet = attrs
            .iter()
            .find(|a| a.name == "worklet:ongesture")
            .unwrap();
        assert_eq!(worklet.value.as_deref(), Some("onPan"));
        assert!(worklet.is_static);
        assert_eq!(doc.position(worklet.value_start.unwrap()), (1, 26));
        // 普通绑定保留原有名字，不会被当成 worklet 绑定。
        assert!(attrs.iter().any(|a| a.name == "bind:ongesture"));
    }

    #[test]
    fn dynamic_attribute_value_is_not_a_static_string() {
        let doc = doc_of(r#"<view type="{{t}}"/>"#);
        let attr = &attributes(&doc)[0];
        assert_eq!(attr.name, "type");
        assert!(!attr.is_static);
        assert_eq!(attr.value, None);
    }

    #[test]
    fn mixed_interpolation_attribute_value_is_not_static() {
        let doc = doc_of(r#"<view type="a{{t}}"/>"#);
        assert!(!attributes(&doc)[0].is_static);
    }

    #[test]
    fn structural_wrappers_are_transparent() {
        let doc = WxmlDoc::parse(
            "t.wxml",
            "<block wx:for=\"{{list}}\"><view>{{item}}</view></block>",
            &Env::default(),
        )
        .unwrap();
        let tags: Vec<&str> = doc.elements().map(|e| e.tag.as_str()).collect();
        assert_eq!(tags, ["view"]);
        assert_eq!(text_nodes(&doc)[0].text, "{{item}}");
    }

    #[test]
    fn if_and_else_branches_share_group_with_distinct_arms() {
        let doc = doc_of(r#"<block wx:if="{{a}}"><view/></block><block wx:else><text/></block>"#);
        let branches: Vec<&Vec<Branch>> = doc.elements().map(|element| &element.branches).collect();
        let (Some(if_branch), Some(else_branch)) = (branches[0].first(), branches[1].first())
        else {
            panic!("两个分支的元素都应带 branch: {branches:?}");
        };
        assert_eq!(if_branch.group, else_branch.group);
        assert_ne!(if_branch.arm, else_branch.arm);
    }

    #[test]
    fn template_refs_expose_the_parsed_src_without_the_wxml_suffix() {
        let doc = doc_of(
            r#"<import src="../../templates/pagination" /><include src="../shared/cell.wxml"/>"#,
        );
        // 实测：`src="x.wxml"` 会被编译器剥掉后缀，`src="x"` 原样保留。
        assert_eq!(
            doc.template_refs(),
            ["../../templates/pagination", "../shared/cell"]
        );
    }

    #[test]
    fn inline_template_definition_body_is_visible() {
        let doc = doc_of(
            r#"<template name="card"><view class="c"><web-view src="x"/></view></template>"#,
        );
        let tags: Vec<&str> = doc.elements().map(|element| element.tag.as_str()).collect();
        assert_eq!(tags, ["view", "web-view"]);
    }

    #[test]
    fn template_instantiation_adds_no_element_of_its_own() {
        let doc = doc_of(r#"<template is="card" data="{{d}}"/>"#);
        assert_eq!(doc.elements().count(), 0);
    }

    #[test]
    fn nested_if_keeps_the_outer_branch_on_the_stack() {
        let doc = parse(
            "a.wxml",
            r#"<block wx:if="{{a}}"><block wx:if="{{b}}"><view/></block></block>"#,
            &Env::default(),
        )
        .unwrap();
        let element = doc
            .elements()
            .find(|element| element.tag == "view")
            .unwrap();
        assert_eq!(element.branches.len(), 2, "{:?}", element.branches);
        assert_ne!(element.branches[0].group, element.branches[1].group);
    }

    #[test]
    fn known_condition_prunes_the_unreachable_branch() {
        let mut env = Env::default();
        env.insert("renderer", crate::input::env::Lit::Str("skyline".into()));
        let doc = WxmlDoc::parse(
            "t.wxml",
            r#"<view wx:if="{{renderer === 'skyline'}}" class="a"/><view wx:else class="b"/>"#,
            &env,
        )
        .unwrap();
        let tags: Vec<(&str, &str)> = doc
            .elements()
            .map(|element| {
                let class = element
                    .attributes
                    .iter()
                    .find(|attr| attr.name == "class")
                    .and_then(|attr| attr.value.as_deref())
                    .unwrap_or("");
                (element.tag.as_str(), class)
            })
            .collect();
        // 只剩可达的那一支，另外一支里的元素不进树。
        assert_eq!(tags, [("view", "a")]);
    }

    #[test]
    fn static_false_condition_still_renders() {
        // `wx:if="false"` 编成 `"false" ? 1 : 0`，字符串为真——分支会渲染。
        // 只有空值等价于「没写这个属性」（`parse/tag.rs:4386-4387`）。
        let doc = doc_of(r#"<view wx:if="false" class="a"/><view wx:else class="b"/>"#);
        let classes: Vec<&str> = doc
            .elements()
            .filter_map(|element| {
                element
                    .attributes
                    .iter()
                    .find(|attr| attr.name == "class")
                    .and_then(|attr| attr.value.as_deref())
            })
            .collect();
        assert_eq!(classes, ["a"], "`else` 分支不参与渲染");
    }

    #[test]
    fn guard_flag_belongs_to_the_arm_that_mentions_renderer() {
        // `wx:if="{{a}}"` 两支在不同渲染器下都会渲染，不能因为同链有 renderer 条件
        // 就被一起当成「按 renderer 分叉」。
        let doc = doc_of(
            r#"<view wx:if="{{a}}" class="a"/><view wx:elif="{{renderer === 'skyline'}}" class="b"/><view wx:else class="c"/>"#,
        );
        let guarded: Vec<(&str, bool)> = doc
            .elements()
            .map(|element| {
                let class = element
                    .attributes
                    .iter()
                    .find(|attr| attr.name == "class")
                    .and_then(|attr| attr.value.as_deref())
                    .unwrap_or("");
                (class, element.renderer_guarded())
            })
            .collect();
        assert_eq!(guarded, [("a", false), ("b", true), ("c", true)]);
    }

    #[test]
    fn unknown_condition_keeps_every_branch() {
        let doc = doc_of(
            r#"<view wx:if="{{renderer === 'skyline'}}" class="a"/><view wx:else class="b"/>"#,
        );
        let count = doc.elements().count();
        assert_eq!(count, 2);
    }

    #[test]
    fn known_condition_folds_class_ternary() {
        let mut env = Env::default();
        env.insert("renderer", crate::input::env::Lit::Str("skyline".into()));
        let doc = WxmlDoc::parse(
            "t.wxml",
            r#"<view class="heart {{renderer === 'skyline' ? 'skyline' : ''}}"/>"#,
            &env,
        )
        .unwrap();
        let class = doc
            .elements()
            .next()
            .unwrap()
            .attributes
            .iter()
            .find(|attr| attr.name == "class")
            .unwrap();
        let tokens = class.class_tokens.as_ref().expect("dynamic class tokens");
        assert!(tokens.is_definite("skyline", true));
        assert!(!tokens.any);
    }

    #[test]
    fn for_inside_if_branch_inherits_branch() {
        let doc =
            doc_of(r#"<block wx:if="{{a}}"><block wx:for="{{list}}"><view/></block></block>"#);
        let element = doc.elements().next().unwrap();
        assert_eq!(element.branches.first().map(|branch| branch.arm), Some(0));
    }
}
