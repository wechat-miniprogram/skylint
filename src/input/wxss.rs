//! WXSS / LESS / SCSS 视图。
//!
//! 只做「解析 + 定位」：把样式表拆成规则、选择器、声明、`@import`，保留原始文本与
//! 字节偏移。不做值解析、选择器匹配、级联——那些属于规则层或编译器。
//!
//! 三种后缀走同一个解析器。预处理器差异只体现在「怎么把嵌套摊平成规则」：
//! - 嵌套按后代组合器拼接，`&` 原地替换；
//! - `@media` / `@supports` 等容器保留父选择器上下文（`.a { @media … { … } }`）；
//! - `@mixin` / `@if` / `@include` 这类「内容仍是声明」的块按透明处理，声明记到
//!   当前选择器上；顶层 mixin 的函数体没有选择器，于是产出一条无选择器的规则——
//!   属性/值类规则照样能检查，选择器类规则自然跳过；
//! - `$var` / `@var` / `#{}` 一律原样保留：静态求值不了，交给规则层判断。
//!
//! 词法与语法交给 `oxc-css-parser`，这里只负责把 AST 里的嵌套摊平成规则。
//! 容错策略：oxc 能恢复的坏块照常产出 AST；它彻底放弃解析时才返回 `Err`——
//! 此时没有可用的结构，宁可让调用方跳过这份文件，也不臆造规则。

use oxc_css_parser::{
    Allocator, ParserBuilder, Span, Syntax,
    ast::{
        AtRule, Calc, ComplexSelector, ComplexSelectorChild, ComponentValue, CompoundSelector,
        Declaration, DelimiterKind, FunctionName, InterpolableIdent, InterpolableStr,
        PseudoClassSelectorArgKind, SelectorList, SimpleSelector, Statement, Stylesheet,
        TypeSelector,
    },
    token::Comment,
};

/// 一份解析后的样式文档。
#[derive(Debug, Clone)]
pub struct WxssDoc {
    path: String,
    source: String,
    /// 样式规则（含 at-rule 内的嵌套规则），按源码顺序。
    pub rules: Vec<WxssRule>,
    /// `@import` 的目标路径，按源码顺序。
    pub imports: Vec<WxssImport>,
}

impl WxssDoc {
    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// 字节偏移 → `(line, column)`，均 1-based，`column` 按 Unicode 字符计。
    ///
    /// 偏移落在多字节字符中间时向下取整到该字符起点；超出文件长度时取文件末尾。
    pub fn position(&self, offset: usize) -> (u32, u32) {
        let mut end = offset.min(self.source.len());
        while end > 0 && !self.source.is_char_boundary(end) {
            end -= 1;
        }
        let prefix = &self.source[..end];
        let line = prefix.bytes().filter(|b| *b == b'\n').count() as u32 + 1;
        let line_start = prefix.rfind('\n').map_or(0, |i| i + 1);
        let column = self.source[line_start..end].chars().count() as u32 + 1;
        (line, column)
    }
}

/// 一条样式规则。
#[derive(Debug, Clone)]
pub struct WxssRule {
    /// 逗号分隔后的选择器，各自保留原文与偏移。
    ///
    /// 为空表示这条规则没有选择器上下文（顶层 `@mixin` 的函数体）：选择器类规则
    /// 应当跳过，属性/值类规则仍可检查其中的声明。
    pub selectors: Vec<WxssSelector>,
    pub declarations: Vec<WxssDeclaration>,
    /// 从外到内的 at-rule 上下文；顶层规则为空。
    pub at_rules: Vec<WxssAtRule>,
}

impl WxssRule {
    /// 按属性名（ASCII 大小写不敏感）找第一条声明。
    pub fn declaration(&self, name: &str) -> Option<&WxssDeclaration> {
        self.declarations
            .iter()
            .find(|d| d.name.eq_ignore_ascii_case(name))
    }

    /// 该规则是否位于某个 at-rule 内。
    pub fn in_at_rule(&self) -> bool {
        !self.at_rules.is_empty()
    }
}

/// 选择器里能静态判定的部件。`Other` 表示组合符 / 属性选择器 / 插值等静态判定不了
/// 的形状；伪类与伪元素单独建模，因为规则需要区分「不影响命中的伪类」和「会让选择器被丢弃的
/// 伪类」（后者仍归 `Pseudo`，由规则按名字判定，见 `skyline/no-pseudo`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectorPart {
    Tag(String),
    Universal,
    Class(String),
    Id(String),
    /// 伪类 / 伪元素。`name` 已转小写且不带冒号；`args` 表示带括号参数。
    Pseudo {
        name: String,
        element: bool,
        args: bool,
    },
    Other,
}

/// 选择器里的一个伪类 / 伪元素，带源文件里的绝对字节位置（诊断落点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssPseudo {
    pub name: String,
    pub element: bool,
    pub offset: usize,
    pub len: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssSelector {
    /// 去掉首尾空白后的选择器原文。
    pub text: String,
    /// 选择器在源文件中的字节偏移。
    pub offset: usize,
    /// AST 拆出的部件；只有一个 compound 且全为标签 / `*` / class / id / 伪类时才有意义。
    pub parts: Vec<SelectorPart>,
    /// 该选择器（含嵌套时继承的父选择器）里的全部伪类 / 伪元素。
    pub pseudos: Vec<WxssPseudo>,
}

/// 声明值的一个词元。只建模规则会用到的形状，其余保留原文切片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueToken {
    /// `name(args…)`，args 已递归解析。
    Function {
        name: String,
        args: Vec<ValueToken>,
    },
    /// `12px` / `1.5rem` / `0`
    Dimension {
        value: String,
        unit: String,
    },
    Number(String),
    Percentage(String),
    /// `#fff`
    Color(String),
    /// `url(...)`，原样含内容。
    Url(String),
    /// 标识符（`to`、`left`、`no-repeat`、`ellipsis`…）。
    Ident(String),
    /// 引号字符串，保留引号。
    Str(String),
    /// 顶层逗号。
    Comma,
    /// 预处理器变量 / 插值 / 无法静态判定（`$x`、`@x`、`#{…}`、`@{}`、backtick）。
    Dynamic,
    /// 其余（运算符、括号块、map、list 等），保留原文切片。
    Other(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssDeclaration {
    /// 属性名原文（未小写化）。
    pub name: String,
    /// 值原文（去掉首尾空白与 `!important`）。
    pub value: String,
    pub important: bool,
    /// 声明起点（属性名）的字节偏移。
    pub offset: usize,
    /// 值起点（跳过前导空白）的字节偏移。
    pub value_offset: usize,
    /// 由 `decl.value` 类型化 AST 递归构造的词元；`value_is_raw` 时只有一段 `Other`。
    pub tokens: Vec<ValueToken>,
}

impl WxssDeclaration {
    /// 值在源文件中的结束字节偏移。
    pub fn value_end(&self) -> usize {
        self.value_offset + self.value.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssAtRule {
    /// 小写化的 at-rule 名，不含 `@`。
    pub name: String,
    /// `@` 之后、`{` 之前的原文（已 trim）。
    pub prelude: String,
    /// prelude 起点的字节偏移。
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssImport {
    /// `@import` 的目标（引号 / `url()` 已剥离）。
    pub target: String,
    /// 目标在源文件中的字节偏移。
    pub offset: usize,
}

/// 解析失败的原因。区分「空文件」（跳过是正常的）与「语法错误」（必须报出来，
/// 否则整份样式会在没有任何提示的情况下不被检查）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 不含任何有效内容：空文件或只有注释。
    Empty,
    /// 语法错误，整份文件无法建立结构。
    Syntax(String),
}

impl std::error::Error for ParseError {}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => f.write_str("样式文件不含任何有效内容"),
            Self::Syntax(detail) => f.write_str(detail),
        }
    }
}

/// 解析样式表。`path` 决定语法与 `//` 行注释的判定（仅预处理器）。
pub fn parse(path: &str, source: &str) -> Result<WxssDoc, ParseError> {
    let source = crate::input::strip_bom(source);
    let depth = nesting_depth(source);
    if depth > MAX_DEPTH {
        return Err(ParseError::Syntax(format!(
            "样式嵌套超过 {MAX_DEPTH} 层，已跳过"
        )));
    }
    // oxc 是递归下降解析器，深嵌套时递归帧很大（调试构建下 64 层能吃掉几 MB 栈），
    // 超过安全阈值就换到 16 MiB 栈的线程里跑，不拖垮调用方的栈。
    if depth > INLINE_DEPTH {
        let (path, source) = (path.to_string(), source.to_string());
        return std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(move || parse_document(&path, &source))
            .map_err(|e| ParseError::Syntax(format!("无法创建解析线程：{e}")))?
            .join()
            .map_err(|_| ParseError::Syntax("样式解析线程异常退出".to_string()))?;
    }
    parse_document(path, source)
}

fn parse_document(path: &str, source: &str) -> Result<WxssDoc, ParseError> {
    let allocator = Allocator::default();
    let mut parser = ParserBuilder::new(&allocator, source)
        .syntax(syntax_for(path))
        .comments()
        .build();
    // lint 场景下局部坏块不该让整文件失效：oxc 自带错误恢复，优先用 `Ok` 的结果，
    // 只有它彻底放弃解析时才把错误抛给调用方。
    let ast = parser
        .parse::<Stylesheet>()
        .map_err(|e| ParseError::Syntax(format!("{e:?}")))?;
    if !has_content(source, parser.comments()) {
        return Err(ParseError::Empty);
    }

    let mut out = Collector::default();
    walk(&mut out, &ast.statements, source, &[], &[], false, 0);

    Ok(WxssDoc {
        path: path.to_string(),
        source: source.to_string(),
        rules: out.rules,
        imports: out.imports,
    })
}

/// 后缀 → 语法。`.wxss` 等其余后缀按纯 CSS 处理。
fn syntax_for(path: &str) -> Syntax {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".less") {
        Syntax::Less
    } else if lower.ends_with(".scss") {
        Syntax::Scss
    } else if lower.ends_with(".sass") {
        Syntax::Sass
    } else {
        Syntax::Css
    }
}

/// 去掉注释后是否还剩内容。注释由解析器收集：`.wxss`（CSS）里 `//` 不是注释。
fn has_content(source: &str, comments: &[Comment<'_>]) -> bool {
    let mut pos = 0;
    for comment in comments {
        let start = comment.span.start.max(pos);
        if !source[pos..start].trim().is_empty() {
            return true;
        }
        pos = comment.span.end.max(start).min(source.len());
    }
    !source[pos..].trim().is_empty()
}

const MAX_DEPTH: usize = 64;

/// 不超过这个层数就在当前线程里解析（普通样式表远远到不了）。
const INLINE_DEPTH: usize = 16;

/// 花括号嵌套的最大层数。只做粗测（跳过字符串与块注释），
/// 宁可误拒极端的合法输入，也不让解析器打爆栈。
fn nesting_depth(source: &str) -> usize {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut max = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'"' | b'\'') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = source[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |end| i + end + 4);
                continue;
            }
            b'{' => {
                depth += 1;
                max = max.max(depth);
                if depth > MAX_DEPTH {
                    return depth;
                }
            }
            b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    max
}

/// 块内仍然是「样式规则列表」的 at-rule：父选择器上下文继续有效。
const RULE_CONTAINERS: [&str; 7] = [
    "media",
    "supports",
    "container",
    "layer",
    "scope",
    "document",
    "at-root",
];

/// 块内仍是「声明 + 嵌套规则」的 at-rule：按透明容器处理。
///
/// 顶层 `@mixin` 的函数体没有选择器，于是产出一条无选择器的规则——属性/值类规则
/// 照样能检查到 mixin 里的声明。
const TRANSPARENT: [&str; 10] = [
    "mixin", "function", "if", "else", "each", "for", "while", "include", "use", "forward",
];

#[derive(Default)]
struct Collector {
    rules: Vec<WxssRule>,
    imports: Vec<WxssImport>,
}

/// 按源码顺序遍历一层语句：声明归到当前选择器，嵌套规则递归摊平。
fn walk<'a>(
    out: &mut Collector,
    statements: &[Statement<'a>],
    source: &str,
    parents: &[WxssSelector],
    at: &[WxssAtRule],
    emit: bool,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut declarations = Vec::new();
    for statement in statements {
        match statement {
            Statement::Declaration(decl) => {
                if let Some(decl) = declaration(decl, source) {
                    declarations.push(decl);
                }
            }
            Statement::QualifiedRule(rule) => {
                let selectors = selectors(&rule.selector, source, parents);
                walk(
                    out,
                    &rule.block.statements,
                    source,
                    &selectors,
                    at,
                    true,
                    depth + 1,
                );
            }
            Statement::LessConditionalQualifiedRule(rule) => {
                let selectors = selectors(&rule.selector, source, parents);
                walk(
                    out,
                    &rule.block.statements,
                    source,
                    &selectors,
                    at,
                    true,
                    depth + 1,
                );
            }
            // less 的 `.mixin(@x) { … }` 是函数体，不是选择器。
            Statement::LessMixinDefinition(def) => {
                walk(
                    out,
                    &def.block.statements,
                    source,
                    parents,
                    at,
                    true,
                    depth + 1,
                );
            }
            // Scss 的 `@if / @else if / @else` 各是一个块，都按透明容器摊平。
            Statement::SassIfAtRule(rule) => {
                walk(
                    out,
                    &rule.if_clause.block.statements,
                    source,
                    parents,
                    at,
                    true,
                    depth + 1,
                );
                for clause in &rule.else_if_clauses {
                    walk(
                        out,
                        &clause.block.statements,
                        source,
                        parents,
                        at,
                        true,
                        depth + 1,
                    );
                }
                if let Some(block) = &rule.else_clause {
                    walk(out, &block.statements, source, parents, at, true, depth + 1);
                }
            }
            Statement::AtRule(rule) => at_rule(out, rule, source, parents, at, depth),
            // 变量、混入调用、extend、错误恢复节点等：不是样式规则，忽略。
            _ => {}
        }
    }

    // 没有声明就不留规则：空规则对任何规则层判断都没有意义。
    // 无选择器但含声明的块是顶层 `@mixin` / `@if` 的函数体。
    if emit && !declarations.is_empty() {
        out.rules.push(WxssRule {
            selectors: parents.to_vec(),
            declarations,
            at_rules: at.to_vec(),
        });
    }
}

fn at_rule(
    out: &mut Collector,
    rule: &AtRule<'_>,
    source: &str,
    parents: &[WxssSelector],
    at: &[WxssAtRule],
    depth: usize,
) {
    let name = rule.name.name.to_ascii_lowercase();
    if name == "import"
        && let Some((prelude, offset)) = prelude(rule, source)
        && let Some((target, offset)) = import_target(prelude, offset)
    {
        out.imports.push(WxssImport { target, offset });
    }
    let Some(block) = &rule.block else {
        return;
    };
    if RULE_CONTAINERS.contains(&name.as_str()) {
        let mut nested = at.to_vec();
        let (text, offset) = prelude(rule, source).unwrap_or(("", rule.name.span.end));
        nested.push(WxssAtRule {
            name,
            prelude: text.to_string(),
            offset,
        });
        walk(
            out,
            &block.statements,
            source,
            parents,
            &nested,
            !parents.is_empty(),
            depth + 1,
        );
    } else if TRANSPARENT.contains(&name.as_str()) {
        walk(out, &block.statements, source, parents, at, true, depth + 1);
    }
    // `@keyframes` / `@font-face` / `@page` 的块内容不是元素样式，整块跳过。
}

/// at-rule 的 prelude 原文（已 trim）与起点偏移；没有 prelude 时返回 `None`。
fn prelude<'s>(rule: &AtRule<'_>, source: &'s str) -> Option<(&'s str, usize)> {
    let span = rule.prelude.as_ref()?.span();
    let raw = source.get(span.start..span.end)?;
    Some((
        raw.trim(),
        span.start + (raw.len() - raw.trim_start().len()),
    ))
}

/// 组合父选择器与子选择器：`&` 原地替换，否则用后代组合器拼接。
fn selectors(list: &SelectorList<'_>, source: &str, parents: &[WxssSelector]) -> Vec<WxssSelector> {
    // 每个 `ComplexSelector.span` 就是一个逗号分隔后的选择器；直接切原文，
    // 保留 `.a>.b`、`#{…}` 这些既有写法。
    let children: Vec<(&ComplexSelector<'_>, String, usize)> = list
        .selectors
        .iter()
        .filter_map(|complex| {
            let raw = source.get(complex.span.start..complex.span.end)?;
            let text = raw.trim();
            (!text.is_empty()).then(|| {
                (
                    complex,
                    text.to_string(),
                    complex.span.start + (raw.len() - raw.trim_start().len()),
                )
            })
        })
        .collect();
    if parents.is_empty() {
        return children
            .into_iter()
            .map(|(complex, text, offset)| WxssSelector {
                text,
                offset,
                parts: complex_parts(complex),
                pseudos: complex_pseudos(complex),
            })
            .collect();
    }
    let mut out = Vec::new();
    for (complex, child_text, offset) in &children {
        for parent in parents {
            let (text, parts) = if child_text.contains('&') {
                (
                    child_text.replace('&', &parent.text),
                    nested_parts(complex, source, &parent.parts),
                )
            } else {
                (
                    format!("{} {}", parent.text, child_text),
                    vec![SelectorPart::Other],
                )
            };
            // 嵌套选择器继承了父选择器，父选择器上的伪类对这条规则同样生效；
            // 两者的 span 都是源文件绝对偏移，合起来不会重复计算位置。
            let mut pseudos = parent.pseudos.clone();
            pseudos.extend(complex_pseudos(complex));
            out.push(WxssSelector {
                text,
                offset: *offset,
                parts,
                pseudos,
            });
        }
    }
    out
}

/// 单个 compound 且全为标签 / `*` / class / id 时才产出可静态判定的部件。
fn complex_parts(complex: &ComplexSelector<'_>) -> Vec<SelectorPart> {
    if complex.children.len() != 1 {
        return vec![SelectorPart::Other];
    }
    let ComplexSelectorChild::CompoundSelector(compound) = &complex.children[0] else {
        return vec![SelectorPart::Other];
    };
    let mut parts = Vec::new();
    for simple in compound.children.iter() {
        match simple {
            SimpleSelector::Type(TypeSelector::TagName(tag)) => {
                if tag.name.prefix.is_some() {
                    return vec![SelectorPart::Other];
                }
                let Some(name) = literal_ident(&tag.name.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Tag(name.to_ascii_lowercase()));
            }
            SimpleSelector::Type(TypeSelector::Universal(universal)) => {
                if universal.prefix.is_some() {
                    return vec![SelectorPart::Other];
                }
                parts.push(SelectorPart::Universal);
            }
            SimpleSelector::Class(class) => {
                let Some(name) = literal_ident(&class.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Class(name.to_string()));
            }
            SimpleSelector::Id(id) => {
                let Some(name) = literal_ident(&id.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Id(name.to_string()));
            }
            SimpleSelector::PseudoClass(pseudo) => {
                let Some(name) = literal_ident(&pseudo.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Pseudo {
                    name: name.to_ascii_lowercase(),
                    element: false,
                    args: pseudo.arg.is_some(),
                });
            }
            SimpleSelector::PseudoElement(pseudo) => {
                let Some(name) = literal_ident(&pseudo.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Pseudo {
                    name: name.to_ascii_lowercase(),
                    element: true,
                    args: pseudo.arg.is_some(),
                });
            }
            _ => return vec![SelectorPart::Other],
        }
    }
    if parts.is_empty() {
        vec![SelectorPart::Other]
    } else {
        parts
    }
}

/// 选择器里的全部伪类 / 伪元素（含 `:not()` / `:nth-child()` 参数里的），带绝对位置。
fn complex_pseudos(complex: &ComplexSelector<'_>) -> Vec<WxssPseudo> {
    let mut out = Vec::new();
    collect_complex_pseudos(complex, &mut out);
    out
}

fn collect_complex_pseudos(complex: &ComplexSelector<'_>, out: &mut Vec<WxssPseudo>) {
    for child in &complex.children {
        let ComplexSelectorChild::CompoundSelector(compound) = child else {
            continue;
        };
        collect_compound_pseudos(compound, out);
    }
}

fn collect_compound_pseudos(compound: &CompoundSelector<'_>, out: &mut Vec<WxssPseudo>) {
    for simple in compound.children.iter() {
        match simple {
            SimpleSelector::PseudoClass(pseudo) => {
                push_pseudo(out, &pseudo.name, pseudo.span, false);
                let Some(arg) = &pseudo.arg else { continue };
                match &arg.kind {
                    PseudoClassSelectorArgKind::CompoundSelectorList(list) => {
                        for compound in list.selectors.iter() {
                            collect_compound_pseudos(compound, out);
                        }
                    }
                    PseudoClassSelectorArgKind::SelectorList(list) => {
                        for complex in list.selectors.iter() {
                            collect_complex_pseudos(complex, out);
                        }
                    }
                    PseudoClassSelectorArgKind::RelativeSelectorList(list) => {
                        for relative in list.selectors.iter() {
                            collect_complex_pseudos(&relative.complex_selector, out);
                        }
                    }
                    _ => {}
                }
            }
            SimpleSelector::PseudoElement(pseudo) => {
                push_pseudo(out, &pseudo.name, pseudo.span, true);
            }
            _ => {}
        }
    }
}

/// 动态伪类名（`:#{$x}`）没有确定名字，不登记——宁可漏报。
fn push_pseudo(out: &mut Vec<WxssPseudo>, name: &InterpolableIdent<'_>, span: Span, element: bool) {
    let Some(name) = literal_ident(name) else {
        return;
    };
    out.push(WxssPseudo {
        name: name.to_ascii_lowercase(),
        element,
        offset: span.start,
        len: span.end - span.start,
    });
}

/// 嵌套选择器 `&…` 的部件：父选择器部件 + `&` 后缀。
/// 只有「单个 compound、以 `&` 开头、后缀全是 class/id」才能静态判定。
fn nested_parts(
    complex: &ComplexSelector<'_>,
    source: &str,
    parent: &[SelectorPart],
) -> Vec<SelectorPart> {
    if parent.contains(&SelectorPart::Other) || complex.children.len() != 1 {
        return vec![SelectorPart::Other];
    }
    let ComplexSelectorChild::CompoundSelector(compound) = &complex.children[0] else {
        return vec![SelectorPart::Other];
    };
    let mut simple = compound.children.iter();
    let Some(SimpleSelector::Nesting(nesting)) = simple.next() else {
        return vec![SelectorPart::Other];
    };
    let mut parts = parent.to_vec();
    if let Some(suffix) = &nesting.suffix {
        let Some(raw) = source.get(suffix.span().start..suffix.span().end) else {
            return vec![SelectorPart::Other];
        };
        if !is_plain_ident(raw) {
            return vec![SelectorPart::Other];
        }
        match parts.last_mut() {
            Some(SelectorPart::Tag(name)) => name.push_str(&raw.to_ascii_lowercase()),
            Some(SelectorPart::Class(name)) | Some(SelectorPart::Id(name)) => name.push_str(raw),
            _ => return vec![SelectorPart::Other],
        }
    }
    for simple in simple {
        match simple {
            SimpleSelector::Class(class) => {
                let Some(name) = literal_ident(&class.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Class(name.to_string()));
            }
            SimpleSelector::Id(id) => {
                let Some(name) = literal_ident(&id.name) else {
                    return vec![SelectorPart::Other];
                };
                parts.push(SelectorPart::Id(name.to_string()));
            }
            _ => return vec![SelectorPart::Other],
        }
    }
    parts
}

/// 选择器里能当标签 / class / id 名的字面 ident（无插值）。
fn literal_ident<'a>(ident: &InterpolableIdent<'a>) -> Option<&'a str> {
    match ident {
        InterpolableIdent::Literal(ident) if is_plain_ident(ident.raw) => Some(ident.raw),
        _ => None,
    }
}

/// 与旧扫描器一致：名字只接受 ASCII 字母数字、`-`、`_`。
fn is_plain_ident(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_'))
}

/// 一条声明。名字与值都从源码切片取，避开 `ComponentValue` 的几十个变体。
fn declaration(decl: &Declaration<'_>, source: &str) -> Option<WxssDeclaration> {
    let name_raw = source.get(decl.span.start..decl.colon_span.start)?;
    let name = name_raw.trim();
    // 预处理器变量定义（`$gap: 4px;`）不是声明
    if name.is_empty() || name.starts_with('$') {
        return None;
    }
    // 只有紧贴的 `!important` 才算标记：`! important`（中间带空白）旧实现不识别，
    // 保留原文交给规则层，而 oxc 会把两者都认成 ImportantAnnotation。
    let important = decl
        .important
        .as_ref()
        .filter(|annotation| is_important(&annotation.span, source));
    let value_end = important.map_or(decl.span.end, |annotation| annotation.span.start);
    let raw_value = source.get(decl.colon_span.end..value_end)?;
    let lead = raw_value.len() - raw_value.trim_start().len();
    let value = raw_value.trim();
    Some(WxssDeclaration {
        name: name.to_string(),
        value: value.to_string(),
        important: important.is_some(),
        offset: decl.span.start + (name_raw.len() - name_raw.trim_start().len()),
        value_offset: decl.colon_span.end + lead,
        tokens: if decl.value_is_raw || is_raw_value(value) {
            vec![ValueToken::Other(value.to_string())]
        } else {
            value_tokens(&decl.value, source)
        },
    })
}

fn value_tokens(values: &[ComponentValue<'_>], source: &str) -> Vec<ValueToken> {
    values
        .iter()
        .filter(|value| !matches!(value, ComponentValue::ImportantAnnotation(_)))
        .map(|value| value_token(value, source))
        .collect()
}

/// oxc 把 IE 的 `progid:` 值解析成 `progid(...)` 函数，但它的参数是原始记号，
/// 静态上没有意义；按「整值原文」处理（与自定义属性同一口径）。
fn is_raw_value(value: &str) -> bool {
    value.to_ascii_lowercase().starts_with("progid:")
}

fn value_token(value: &ComponentValue<'_>, source: &str) -> ValueToken {
    match value {
        ComponentValue::Function(function) => {
            let Some(name) = function_name(&function.name) else {
                return other_token(value, source);
            };
            // `calc()` 在 AST 里是 `Function(calc, [Calc])`；摊平内层，避免出现
            // 两个同名的 calc 调用。
            if name.eq_ignore_ascii_case("calc")
                && function.args.len() == 1
                && let ComponentValue::Calc(calc) = &function.args[0]
            {
                return ValueToken::Function {
                    name,
                    args: calc_tokens(calc, source),
                };
            }
            ValueToken::Function {
                name,
                args: value_tokens(&function.args, source),
            }
        }
        // 裸 `Calc`（例如值本身就是表达式）也按同名函数处理。
        ComponentValue::Calc(calc) => ValueToken::Function {
            name: "calc".to_string(),
            args: calc_tokens(calc, source),
        },
        ComponentValue::Dimension(dimension) => ValueToken::Dimension {
            value: dimension.value.raw.to_string(),
            unit: dimension.unit.raw.to_string(),
        },
        ComponentValue::Number(number) => ValueToken::Number(number.raw.to_string()),
        ComponentValue::Percentage(percentage) => {
            ValueToken::Percentage(slice(source, percentage.span()))
        }
        ComponentValue::HexColor(color) => ValueToken::Color(slice(source, &color.span)),
        ComponentValue::Url(url) => ValueToken::Url(slice(source, url.span())),
        ComponentValue::InterpolableIdent(ident) => match ident {
            InterpolableIdent::Literal(ident) => ValueToken::Ident(ident.raw.to_string()),
            _ => ValueToken::Dynamic,
        },
        ComponentValue::InterpolableStr(string) => match string {
            InterpolableStr::Literal(string) => ValueToken::Str(string.raw.to_string()),
            _ => ValueToken::Dynamic,
        },
        ComponentValue::Delimiter(delimiter) if matches!(delimiter.kind, DelimiterKind::Comma) => {
            ValueToken::Comma
        }
        // 预处理器变量 / 插值 / backtick：静态求值不了。
        ComponentValue::LessVariable(_)
        | ComponentValue::LessVariableCall(_)
        | ComponentValue::LessVariableVariable(_)
        | ComponentValue::LessPropertyVariable(_)
        | ComponentValue::LessNamespaceValue(_)
        | ComponentValue::LessJavaScriptSnippet(_)
        | ComponentValue::SassVariable(_)
        | ComponentValue::PostcssSimpleVar(_) => ValueToken::Dynamic,
        _ => other_token(value, source),
    }
}

fn calc_tokens(calc: &Calc<'_>, source: &str) -> Vec<ValueToken> {
    let mut args = value_tokens(std::slice::from_ref(calc.left.as_ref()), source);
    let op = source
        .get(calc.op.span.start..calc.op.span.end)
        .unwrap_or_default();
    if !op.is_empty() {
        args.push(ValueToken::Other(op.to_string()));
    }
    args.extend(value_tokens(
        std::slice::from_ref(calc.right.as_ref()),
        source,
    ));
    args
}

fn function_name(name: &FunctionName<'_>) -> Option<String> {
    match name {
        FunctionName::Ident(InterpolableIdent::Literal(ident)) => Some(ident.raw.to_string()),
        _ => None,
    }
}

fn other_token(value: &ComponentValue<'_>, source: &str) -> ValueToken {
    ValueToken::Other(slice(source, value.span()))
}

fn slice(source: &str, span: &Span) -> String {
    source
        .get(span.start..span.end)
        .unwrap_or_default()
        .to_string()
}

/// 源码里的 `!important` 是否紧贴（`! important` 不算）。
fn is_important(span: &Span, source: &str) -> bool {
    source
        .get(span.start..span.end)
        .is_some_and(|text| text.eq_ignore_ascii_case("!important"))
}

/// 从 `@import` 的 prelude 里取目标路径，返回 `(target, 源文件偏移)`。
fn import_target(prelude: &str, base: usize) -> Option<(String, usize)> {
    let trimmed = prelude.trim();
    if trimmed.is_empty() {
        return None;
    }
    // less 的 `@import (reference) "x";`：跳过括号里的选项
    let after_options = if trimmed.starts_with('(') {
        match trimmed.find(')') {
            Some(i) => trimmed[i + 1..].trim_start(),
            None => trimmed,
        }
    } else {
        trimmed
    };
    let (raw, lead) = if let Some(rest) = after_options.strip_prefix("url(") {
        let inner = rest.split(')').next().unwrap_or(rest).trim();
        (inner, after_options.len() - rest.len())
    } else {
        let inner = after_options.split_whitespace().next()?;
        (inner, 0)
    };
    let target = raw.trim_matches(['"', '\'']).to_string();
    if target.is_empty() {
        return None;
    }
    Some((
        target,
        base + prelude.find(after_options).unwrap_or(0) + lead,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_is_one_based_and_counts_chars() {
        let src = "a {\n  color: 红;\n}\n/* 😀 */\n.x{}\n";
        let doc = parse("a.wxss", src).unwrap();

        assert_eq!(doc.position(0), (1, 1));
        // 换行后的缩进：两个空格 → 第 3 列。
        let color = src.find("color").unwrap();
        assert_eq!(doc.position(color), (2, 3));
        // 中文按一个字符计：`  color: ` 共 9 个字符后是「红」。
        let red = src.find('红').unwrap();
        assert_eq!(doc.position(red), (2, 10));
        // emoji 是一个字符（不是 2 个 UTF-16 code unit）。
        let emoji = src.find('😀').unwrap();
        assert_eq!(doc.position(emoji), (4, 4));
        // 落在多字节字符中间时向下取整。
        assert_eq!(doc.position(red + 1), (2, 10));
        // 超出末尾时取文件末尾。
        assert_eq!(doc.position(src.len() + 100), (6, 1));
    }

    #[test]
    fn parses_selectors_and_declarations() {
        let doc = parse(
            "a.wxss",
            ".a, .b:hover { color: red; width: 10rpx !important; }",
        )
        .unwrap();
        assert_eq!(doc.rules.len(), 1);
        let rule = &doc.rules[0];
        assert_eq!(rule.selectors.len(), 2);
        assert_eq!(rule.selectors[0].text, ".a");
        assert_eq!(rule.selectors[1].text, ".b:hover");
        assert_eq!(rule.declarations.len(), 2);
        assert_eq!(rule.declarations[0].name, "color");
        assert_eq!(rule.declarations[0].value, "red");
        assert!(!rule.declarations[0].important);
        assert_eq!(rule.declarations[1].value, "10rpx");
        assert!(rule.declarations[1].important);
        assert!(!rule.in_at_rule());
    }

    #[test]
    fn keeps_at_rule_context_and_imports() {
        let doc = parse(
            "a.wxss",
            "@import \"./base.wxss\";\n@media (min-width: 10px) { .a { color: red; } }",
        )
        .unwrap();
        assert_eq!(doc.imports.len(), 1);
        assert_eq!(doc.imports[0].target, "./base.wxss");
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].at_rules.len(), 1);
        assert_eq!(doc.rules[0].at_rules[0].name, "media");
        assert_eq!(doc.rules[0].declarations[0].name, "color");
    }

    #[test]
    fn splits_selectors_without_breaking_functions() {
        let doc = parse("a.wxss", ":is(.a, .b), [data-x=\"a,b\"] { color: red; }").unwrap();
        let texts: Vec<_> = doc.rules[0]
            .selectors
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(texts, vec![":is(.a, .b)", "[data-x=\"a,b\"]"]);
    }

    #[test]
    fn important_flag_works_with_and_without_space() {
        let doc = parse(
            "a.wxss",
            ".a { margin: 0!important; content: \"!important\"; padding: 1px ! important; }",
        )
        .unwrap();
        let decls = &doc.rules[0].declarations;
        assert_eq!(decls[0].value, "0");
        assert!(decls[0].important);
        // 字符串里的 !important 不是标记。
        assert_eq!(decls[1].value, "\"!important\"");
        assert!(!decls[1].important);
        // `! important`（带空格）也是合法写法，P0 不识别，保留原文。
        assert_eq!(decls[2].value, "1px ! important");
    }

    #[test]
    fn bad_blocks_are_dropped_not_fatal() {
        // 空语句块被丢弃，同文件后续规则仍要解析出来；没声明的规则不留。
        let doc = parse("a.wxss", ".a { ; } .b { width: 1px; }").unwrap();
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].selectors[0].text, ".b");
        assert_eq!(doc.rules[0].declarations[0].value, "1px");

        // 缺少冒号的声明（`color red`）在 CSS 语法下是硬错误，oxc 不做块级恢复：
        // 整份解析失败，因此整文件被拒绝。旧的自写扫描器只丢这一块。
        // 这条失败必须冒泡成「文件未被检查」的告警，不能静默。
        assert!(matches!(
            parse("a.wxss", ".a { color red; ; } .b { width: 1px; }"),
            Err(ParseError::Syntax(_))
        ));
    }

    // ---- 预处理器：嵌套摊平 ----

    #[test]
    fn flattens_nested_selectors() {
        let doc = parse(
            "a.scss",
            ".a {\n  display: flex;\n  .b { color: red; }\n}\n",
        )
        .unwrap();
        assert_eq!(doc.rules.len(), 2);
        assert_eq!(doc.rules[0].selectors[0].text, ".a .b");
        assert_eq!(doc.rules[0].declarations[0].name, "color");
        assert_eq!(doc.rules[1].selectors[0].text, ".a");
        assert_eq!(doc.rules[1].declarations[0].value, "flex");
    }

    #[test]
    fn replaces_ampersand_and_crosses_selector_lists() {
        let doc = parse(
            "a.less",
            ".a, .b { &:hover { color: red; } .c & { margin: 0; } }",
        )
        .unwrap();
        let texts: Vec<_> = doc
            .rules
            .iter()
            .flat_map(|r| r.selectors.iter().map(|s| s.text.as_str()))
            .filter(|text| text.contains(":hover"))
            .collect();
        assert_eq!(texts, vec![".a:hover", ".b:hover"]);
    }

    #[test]
    fn at_rule_container_keeps_parent_selector() {
        let doc = parse("a.scss", ".a { @media (min-width: 100px) { color: red; } }").unwrap();
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].selectors[0].text, ".a");
        assert_eq!(doc.rules[0].at_rules[0].name, "media");
        assert_eq!(doc.rules[0].declarations[0].value, "red");
    }

    #[test]
    fn mixin_body_declarations_are_kept_without_selector() {
        let doc = parse(
            "a.scss",
            "@mixin trunc { text-overflow: ellipsis; }
.a { @include trunc; color: $c; }",
        )
        .unwrap();
        let mixin = doc
            .rules
            .iter()
            .find(|r| r.selectors.is_empty())
            .expect("mixin 函数体应产出一条无选择器规则");
        assert_eq!(mixin.declarations[0].name, "text-overflow");
        // 变量原样保留，不猜值
        let vars = doc
            .rules
            .iter()
            .find(|r| r.selectors.first().is_some_and(|s| s.text == ".a"))
            .unwrap();
        assert_eq!(vars.declarations[0].value, "$c");
    }

    #[test]
    fn less_variables_and_mixin_calls_are_not_rules() {
        let doc = parse(
            "a.less",
            "@brand: #fff;
.mixin(@x) { color: @x; }
.a { .mixin(#000); color: @brand; }",
        )
        .unwrap();
        let texts: Vec<_> = doc
            .rules
            .iter()
            .filter(|r| !r.selectors.is_empty())
            .map(|r| r.selectors[0].text.as_str())
            .collect();
        // 变量定义和混入调用都不是规则；`.mixin(@x)` 的函数体保留为无选择器规则
        assert_eq!(texts, vec![".a"]);
        let a = doc
            .rules
            .iter()
            .find(|r| r.selectors.first().is_some_and(|s| s.text == ".a"))
            .unwrap();
        assert_eq!(a.declarations[0].value, "@brand");
    }

    #[test]
    fn keyframes_block_is_skipped() {
        let doc = parse(
            "a.wxss",
            "@keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(1turn); } }\n.a { color: red; }",
        )
        .unwrap();
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].selectors[0].text, ".a");
    }

    #[test]
    fn line_comments_only_apply_to_preprocessor_files() {
        let scss = parse("a.scss", "// 注释\n.a { color: red; }").unwrap();
        assert_eq!(scss.rules.len(), 1);
        // .wxss（CSS 语法）里 `//` 不是注释，且 `//` 不能当选择器开头：oxc 直接报错。
        // 新行为是整文件被拒绝（旧扫描器会把 `// x` 当选择器原文保留）。
        assert!(parse("a.wxss", "// x\n.b { color: red; }").is_err());
    }

    #[test]
    fn interpolation_in_selector_is_kept_verbatim() {
        let doc = parse("a.scss", ".icon-#{$name} { color: red; }").unwrap();
        assert_eq!(doc.rules[0].selectors[0].text, ".icon-#{$name}");
    }

    #[test]
    fn url_with_protocol_is_not_a_line_comment() {
        let doc = parse(
            "a.scss",
            ".a {\n  background-image: url(https://x.com/a.svg);\n  background-size: cover;\n}\n.b { color: red; }",
        )
        .unwrap();
        assert_eq!(doc.rules.len(), 2, "{:?}", doc.rules);
        assert_eq!(doc.rules[0].declarations.len(), 2);
        assert_eq!(
            doc.rules[0].declarations[0].value,
            "url(https://x.com/a.svg)"
        );
    }

    #[test]
    fn last_declaration_without_semicolon_is_kept() {
        let doc = parse(
            "a.scss",
            ".a {\n  color: red;\n  padding-bottom: 0\n}\n.b { margin: 0 }",
        )
        .unwrap();
        assert_eq!(doc.rules.len(), 2);
        assert_eq!(doc.rules[0].declarations.len(), 2);
        assert_eq!(doc.rules[0].declarations[1].name, "padding-bottom");
        assert_eq!(doc.rules[0].declarations[1].value, "0");
        assert_eq!(doc.rules[1].declarations[0].name, "margin");
    }

    #[test]
    fn empty_stylesheet_is_an_error() {
        // 空文件与「只有注释」是正常情况：跳过即可，不该产生告警。
        for source in ["", "  \n\t ", "/* only a comment */"] {
            assert_eq!(parse("a.wxss", source).unwrap_err(), ParseError::Empty);
        }
    }

    #[test]
    fn excessive_nesting_is_rejected_instead_of_overflowing() {
        // oxc 是递归下降解析器，极深嵌套会打爆栈；解析前先守住深度上限。
        let deep = |n: usize| format!("{}{}{}", ".a{".repeat(n), "color:red;", "}".repeat(n));
        assert!(parse("a.scss", &deep(64)).is_ok());
        assert!(parse("a.scss", &deep(65)).is_err());
    }

    // ---- 值词元 ----

    fn tokens(path: &str, value: &str) -> Vec<ValueToken> {
        let source = format!(".a {{ x: {value}; }}");
        let doc = parse(path, &source).unwrap();
        doc.rules[0].declarations[0].tokens.clone()
    }

    #[test]
    fn function_args_keep_dimensions_and_commas() {
        assert_eq!(
            tokens("a.wxss", "translate(10px, 20px)"),
            vec![ValueToken::Function {
                name: "translate".to_string(),
                args: vec![
                    ValueToken::Dimension {
                        value: "10".to_string(),
                        unit: "px".to_string(),
                    },
                    ValueToken::Comma,
                    ValueToken::Dimension {
                        value: "20".to_string(),
                        unit: "px".to_string(),
                    },
                ],
            }]
        );
    }

    #[test]
    fn url_keeps_its_raw_text_including_spaces() {
        assert_eq!(
            tokens("a.wxss", r#"url("a b.png")"#),
            vec![ValueToken::Url(r#"url("a b.png")"#.to_string())]
        );
    }

    #[test]
    fn leading_dot_numbers_and_hex_colors_are_typed() {
        assert_eq!(
            tokens("a.wxss", "rgba(0, 0, 0, .2)"),
            vec![ValueToken::Function {
                name: "rgba".to_string(),
                args: vec![
                    ValueToken::Number("0".to_string()),
                    ValueToken::Comma,
                    ValueToken::Number("0".to_string()),
                    ValueToken::Comma,
                    ValueToken::Number("0".to_string()),
                    ValueToken::Comma,
                    ValueToken::Number(".2".to_string()),
                ],
            }]
        );
    }

    #[test]
    fn gradient_prelude_and_stops_are_idents_percentages_and_colors() {
        assert_eq!(
            tokens("a.wxss", "linear-gradient(to right, #fff 0%, red 100%)"),
            vec![ValueToken::Function {
                name: "linear-gradient".to_string(),
                args: vec![
                    ValueToken::Ident("to".to_string()),
                    ValueToken::Ident("right".to_string()),
                    ValueToken::Comma,
                    ValueToken::Color("#fff".to_string()),
                    ValueToken::Percentage("0%".to_string()),
                    ValueToken::Comma,
                    ValueToken::Ident("red".to_string()),
                    ValueToken::Percentage("100%".to_string()),
                ],
            }]
        );
    }

    #[test]
    fn calc_nests_var_as_a_function() {
        assert_eq!(
            tokens("a.wxss", "calc(100% - var(--x))"),
            vec![ValueToken::Function {
                name: "calc".to_string(),
                args: vec![
                    ValueToken::Percentage("100%".to_string()),
                    ValueToken::Other("-".to_string()),
                    ValueToken::Function {
                        name: "var".to_string(),
                        args: vec![ValueToken::Ident("--x".to_string())],
                    },
                ],
            }]
        );
    }

    #[test]
    fn preprocessor_variables_and_interpolation_are_dynamic() {
        assert_eq!(
            tokens("a.less", "@w"),
            vec![ValueToken::Dynamic],
            "@w 是 less 变量"
        );
        assert_eq!(
            tokens("a.scss", "#{$x}"),
            vec![ValueToken::Dynamic],
            "#{{$x}} 是 scss 插值"
        );
        // 二元运算整体落进 Other（里面仍含 `@`，由规则层判 dynamic）。
        assert_eq!(
            tokens("a.less", "@w + @h"),
            vec![ValueToken::Other("@w + @h".to_string())]
        );
    }

    #[test]
    fn raw_values_are_a_single_other_token() {
        let value =
            "progid:DXImageTransform.Microsoft.gradient(startColorstr='#000', endColorstr='#fff')";
        assert_eq!(
            tokens("a.wxss", value),
            vec![ValueToken::Other(value.to_string())]
        );
    }

    // ---- 选择器部件 ----

    fn parts(source: &str) -> Vec<SelectorPart> {
        let doc = parse("a.scss", source).unwrap();
        doc.rules[0].selectors[0].parts.clone()
    }

    #[test]
    fn selector_parts_cover_static_shapes() {
        assert_eq!(
            parts("view.foo { color: red; }"),
            vec![
                SelectorPart::Tag("view".to_string()),
                SelectorPart::Class("foo".to_string())
            ]
        );
        assert_eq!(
            parts(".a.b { color: red; }"),
            vec![
                SelectorPart::Class("a".to_string()),
                SelectorPart::Class("b".to_string())
            ]
        );
        assert_eq!(
            parts("#id { color: red; }"),
            vec![SelectorPart::Id("id".to_string())]
        );
        assert_eq!(parts("* { color: red; }"), vec![SelectorPart::Universal]);
    }

    #[test]
    fn selector_parts_reject_combinators_attributes_and_nesting() {
        for source in [
            "view .foo { color: red; }",
            "view > .foo { color: red; }",
            "[data-x] { color: red; }",
            "&:hover { color: red; }",
        ] {
            assert_eq!(parts(source), vec![SelectorPart::Other], "{source}");
        }
    }

    #[test]
    fn selector_parts_include_pseudos() {
        assert_eq!(
            parts("a:hover { color: red; }"),
            vec![
                SelectorPart::Tag("a".to_string()),
                SelectorPart::Pseudo {
                    name: "hover".to_string(),
                    element: false,
                    args: false,
                },
            ]
        );
        assert_eq!(
            parts("a:nth-child(2) { color: red; }"),
            vec![
                SelectorPart::Tag("a".to_string()),
                SelectorPart::Pseudo {
                    name: "nth-child".to_string(),
                    element: false,
                    args: true,
                },
            ]
        );
        assert_eq!(
            parts("a::before { color: red; }"),
            vec![
                SelectorPart::Tag("a".to_string()),
                SelectorPart::Pseudo {
                    name: "before".to_string(),
                    element: true,
                    args: false,
                },
            ]
        );
    }

    #[test]
    fn selector_pseudos_carry_the_source_span() {
        //            0123456789012345678
        let source = ".a:hover, .b::after { color: red; }";
        let doc = parse("a.scss", source).unwrap();
        let pseudos: Vec<(&str, bool, usize, usize)> = doc.rules[0]
            .selectors
            .iter()
            .flat_map(|s| s.pseudos.iter())
            .map(|p| (p.name.as_str(), p.element, p.offset, p.len))
            .collect();
        assert_eq!(
            pseudos,
            vec![("hover", false, 2, 6), ("after", true, 12, 7)]
        );
        assert_eq!(doc.position(2), (1, 3));
    }
}
