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
    ast::{AtRule, Declaration, SelectorList, Statement, Stylesheet},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WxssSelector {
    /// 去掉首尾空白后的选择器原文。
    pub text: String,
    /// 选择器在源文件中的字节偏移。
    pub offset: usize,
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
    let children: Vec<WxssSelector> = list
        .selectors
        .iter()
        .filter_map(|complex| {
            let raw = source.get(complex.span.start..complex.span.end)?;
            let text = raw.trim();
            (!text.is_empty()).then(|| WxssSelector {
                text: text.to_string(),
                offset: complex.span.start + (raw.len() - raw.trim_start().len()),
            })
        })
        .collect();
    if parents.is_empty() {
        return children;
    }
    let mut out = Vec::new();
    for child in &children {
        for parent in parents {
            let text = if child.text.contains('&') {
                child.text.replace('&', &parent.text)
            } else {
                format!("{} {}", parent.text, child.text)
            };
            out.push(WxssSelector {
                text,
                offset: child.offset,
            });
        }
    }
    out
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
    Some(WxssDeclaration {
        name: name.to_string(),
        value: raw_value.trim().to_string(),
        important: important.is_some(),
        offset: decl.span.start + (name_raw.len() - name_raw.trim_start().len()),
        value_offset: decl.colon_span.end + lead,
    })
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
}
