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
//! 容错策略：坏块逐块丢弃，不因单条规则失败而让整个文件不可用。

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

/// 解析样式表。`path` 决定是否启用 `//` 行注释（仅预处理器）。
pub fn parse(path: &str, source: &str) -> Result<WxssDoc, String> {
    if !has_token(source) {
        return Err("样式文件不含任何有效内容".to_string());
    }

    let mut scanner = Scanner {
        src: source,
        pos: 0,
        line_comments: is_preprocessor(path),
    };
    let mut out = Collector::default();
    scanner.body(&mut out, &[], &[], false, 0);

    Ok(WxssDoc {
        path: path.to_string(),
        source: source.to_string(),
        rules: out.rules,
        imports: out.imports,
    })
}

fn is_preprocessor(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".less") || lower.ends_with(".scss") || lower.ends_with(".sass")
}

/// 文件里是否存在非空白、非注释的内容。
fn has_token(source: &str) -> bool {
    let mut scanner = Scanner {
        src: source,
        pos: 0,
        line_comments: true,
    };
    scanner.skip_trivia();
    scanner.pos < source.len()
}

const MAX_DEPTH: usize = 64;

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
const TRANSPARENT: [&str; 8] = [
    "mixin", "function", "if", "else", "each", "for", "while", "include",
];

#[derive(Default)]
struct Collector {
    rules: Vec<WxssRule>,
    imports: Vec<WxssImport>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Stop {
    Open,
    Semi,
    Close,
    Eof,
}

struct Scanner<'a> {
    src: &'a str,
    pos: usize,
    /// `.less` / `.scss` 里 `//` 是行注释；`.wxss` 里不是。
    line_comments: bool,
}

/// less 的混入定义：`.mixin(...)` / `#ns(...)`——括号紧跟名字，和 `:not(...)` 不同。
fn is_mixin_definition(raw: &str) -> bool {
    let mut chars = raw.chars();
    if !matches!(chars.next(), Some('.') | Some('#')) {
        return false;
    }
    let rest = &raw[1..];
    let name_len = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_alphanumeric() || *ch == '-' || *ch == '_')
        .map(|(i, ch)| i + ch.len_utf8())
        .last()
        .unwrap_or(0);
    name_len > 0 && rest[name_len..].starts_with('(')
}

impl<'a> Scanner<'a> {
    fn body(
        &mut self,
        out: &mut Collector,
        parents: &[WxssSelector],
        at: &[WxssAtRule],
        emit: bool,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut declarations = Vec::new();

        loop {
            let (start, end, stop) = self.scan();
            match stop {
                Stop::Semi => {
                    // 消费 `;`：空语句（`;;`）也要前进
                    self.pos += 1;
                    let raw = self.src[start..end].trim();
                    // at-rule 语句优先：`@include flex($a: 1)` 里的冒号不是声明分隔符
                    if raw.starts_with('@') {
                        let (name, rest) = at_rule_head(raw);
                        if name == "import"
                            && let Some((target, offset)) = import_target(&rest, start)
                        {
                            out.imports.push(WxssImport { target, offset });
                        }
                    } else if let Some(decl) = declaration(raw, start) {
                        declarations.push(decl);
                    }
                }
                Stop::Open => {
                    let raw = self.src[start..end].trim();
                    if raw.starts_with('@') {
                        // at-rule 自己决定是吃掉块还是跳过（`{` 由它消费）
                        self.at_rule(out, parents, at, depth, raw, start);
                    } else if is_mixin_definition(raw) {
                        // less 的 `.mixin(@x) { … }` 是函数体，不是选择器
                        self.pos += 1;
                        self.body(out, parents, at, true, depth + 1);
                    } else {
                        self.pos += 1;
                        let selectors = self.selectors(raw, start, parents);
                        self.body(out, &selectors, at, true, depth + 1);
                    }
                }
                Stop::Close => {
                    self.pos += 1;
                    break;
                }
                Stop::Eof => break,
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
        &mut self,
        out: &mut Collector,
        parents: &[WxssSelector],
        at: &[WxssAtRule],
        depth: usize,
        raw: &str,
        start: usize,
    ) {
        let (name, rest) = at_rule_head(raw);
        if RULE_CONTAINERS.contains(&name.as_str()) {
            let mut nested = at.to_vec();
            let prelude_offset = start + raw.find(&rest).unwrap_or(1);
            nested.push(WxssAtRule {
                name,
                prelude: rest,
                offset: prelude_offset,
            });
            self.pos += 1;
            self.body(out, parents, &nested, !parents.is_empty(), depth + 1);
        } else if TRANSPARENT.contains(&name.as_str()) {
            self.pos += 1;
            self.body(out, parents, at, true, depth + 1);
        } else {
            // `@keyframes` / `@font-face` / `@page` 的块内容不是元素样式，整块跳过。
            self.skip_block();
        }
    }

    /// 组合父选择器与子选择器：`&` 原地替换，否则用后代组合器拼接。
    fn selectors(&self, raw: &str, start: usize, parents: &[WxssSelector]) -> Vec<WxssSelector> {
        let lead = raw.len() - raw.trim_start().len();
        let children = split_selectors(raw, start + lead);
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

    /// 扫描到下一个顶层 `{` / `;` / `}`，返回 `(起, 止, 终止符)`；终止符本身未消费。
    fn scan(&mut self) -> (usize, usize, Stop) {
        self.skip_trivia();
        let start = self.pos;
        let bytes = self.src.as_bytes();
        let mut depth = 0i32;
        let mut quote: Option<u8> = None;
        let mut escaped = false;

        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if let Some(q) = quote {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    quote = None;
                }
                self.pos += 1;
                continue;
            }
            match b {
                b'"' | b'\'' => {
                    quote = Some(b);
                    self.pos += 1;
                }
                b'/' if bytes.get(self.pos + 1) == Some(&b'*') => {
                    self.skip_comment(false);
                }
                b'/' if self.line_comments && bytes.get(self.pos + 1) == Some(&b'/') => {
                    self.skip_comment(true);
                }
                b'#' | b'@' if bytes.get(self.pos + 1) == Some(&b'{') => {
                    // 插值 `#{…}` / `@{…}`：整体跳过，不做选择器切分
                    self.pos += 2;
                    while self.pos < bytes.len() && bytes[self.pos] != b'}' {
                        self.pos += 1;
                    }
                    if self.pos < bytes.len() {
                        self.pos += 1;
                    }
                }
                b'(' | b'[' => {
                    depth += 1;
                    self.pos += 1;
                }
                b')' | b']' => {
                    depth -= 1;
                    self.pos += 1;
                }
                b'{' if depth <= 0 => return (start, self.pos, Stop::Open),
                b';' if depth <= 0 => return (start, self.pos, Stop::Semi),
                b'}' if depth <= 0 => return (start, self.pos, Stop::Close),
                _ => self.pos += 1,
            }
        }
        (start, bytes.len(), Stop::Eof)
    }

    /// 消费当前块的 `{ … }`（含括号本身）。
    fn skip_block(&mut self) {
        // 调用时 `{` 尚未消费。
        if self.src.as_bytes().get(self.pos) == Some(&b'{') {
            self.pos += 1;
        } else {
            return;
        }
        let mut depth = 1i32;
        let bytes = self.src.as_bytes();
        let mut quote: Option<u8> = None;
        let mut escaped = false;
        while self.pos < bytes.len() {
            let b = bytes[self.pos];
            if let Some(q) = quote {
                if escaped {
                    escaped = false;
                } else if b == b'\\' {
                    escaped = true;
                } else if b == q {
                    quote = None;
                }
                self.pos += 1;
                continue;
            }
            match b {
                b'"' | b'\'' => quote = Some(b),
                b'/' if bytes.get(self.pos + 1) == Some(&b'*') => {
                    self.skip_comment(false);
                    continue;
                }
                b'/' if self.line_comments && bytes.get(self.pos + 1) == Some(&b'/') => {
                    self.skip_comment(true);
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos += 1;
                        return;
                    }
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    fn skip_comment(&mut self, line: bool) {
        let rest = &self.src[self.pos..];
        let end = if line {
            rest.find('\n').unwrap_or(rest.len())
        } else {
            rest[2..].find("*/").map(|i| i + 4).unwrap_or(rest.len())
        };
        self.pos += end;
    }

    fn skip_trivia(&mut self) {
        loop {
            while let Some(ch) = self.src[self.pos..].chars().next() {
                if !ch.is_whitespace() {
                    break;
                }
                self.pos += ch.len_utf8();
            }
            let rest = &self.src[self.pos..];
            if rest.starts_with("/*") {
                self.skip_comment(false);
                continue;
            }
            if self.line_comments && rest.starts_with("//") {
                self.skip_comment(true);
                continue;
            }
            return;
        }
    }
}

/// 拆出 at-rule 名（小写，不含 `@`）与 `@` 之后的原文。
fn at_rule_head(raw: &str) -> (String, String) {
    let rest = raw.strip_prefix('@').unwrap_or(raw);
    let end = rest
        .char_indices()
        .take_while(|(_, ch)| ch.is_ascii_alphanumeric() || *ch == '-' || *ch == '_')
        .map(|(i, ch)| i + ch.len_utf8())
        .last()
        .unwrap_or(0);
    (
        rest[..end].to_ascii_lowercase(),
        rest[end..].trim().to_string(),
    )
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

/// 按顶层逗号切分选择器。括号 / 方括号 / 字符串内的逗号不切。
fn split_selectors(prelude: &str, base: usize) -> Vec<WxssSelector> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (i, ch) in prelude.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ',' if depth == 0 => {
                push_selector(&mut out, &prelude[start..i], base + start);
                start = i + ch.len_utf8();
            }
            _ => {}
        }
    }
    push_selector(&mut out, &prelude[start..], base + start);
    out
}

fn push_selector(out: &mut Vec<WxssSelector>, raw: &str, base: usize) {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let lead = raw.len() - raw.trim_start().len();
    out.push(WxssSelector {
        text: trimmed.to_string(),
        offset: base + lead,
    });
}

/// 一条声明：`name: value`。没有顶层冒号（比如坏写法的 `color red`）返回 `None`。
fn declaration(raw: &str, base: usize) -> Option<WxssDeclaration> {
    let colon = top_level_colon(raw)?;
    let name_raw = &raw[..colon];
    let name = name_raw.trim();
    // 预处理器变量定义（`$gap: 4px;`）不是声明
    if name.is_empty() || name.starts_with('$') || name.contains(['{', '}']) {
        return None;
    }
    let name_lead = name_raw.len() - name_raw.trim_start().len();
    let (value, value_offset, important) = normalize_value(&raw[colon + 1..], base + colon + 1);
    Some(WxssDeclaration {
        name: name.to_string(),
        value,
        important,
        offset: base + name_lead,
        value_offset,
    })
}

/// 顶层冒号：跳过字符串、括号、方括号内的冒号（`url(http://…)`、`[data-x="a:b"]`）。
fn top_level_colon(raw: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (i, ch) in raw.char_indices() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            '(' | '[' => depth += 1,
            ')' | ']' => depth -= 1,
            ':' if depth <= 0 => return Some(i),
            _ => {}
        }
    }
    None
}

/// 去掉首尾空白与结尾的 `!important`，返回 `(值, 值起点, 是否 important)`。
fn normalize_value(raw: &str, base: usize) -> (String, usize, bool) {
    let lead = raw.len() - raw.trim_start().len();
    let value = raw.trim();
    match strip_important(value) {
        Some(head) => (head.to_string(), base + lead, true),
        None => (value.to_string(), base + lead, false),
    }
}

/// 剥离结尾的 `!important`（ASCII 大小写不敏感）。字符串里的同名字样不算。
fn strip_important(value: &str) -> Option<&str> {
    const FLAG: &str = "!important";
    let idx = value.len().checked_sub(FLAG.len())?;
    if !value.is_char_boundary(idx)
        || !value[idx..].eq_ignore_ascii_case(FLAG)
        || inside_string(&value[..idx])
    {
        return None;
    }
    Some(value[..idx].trim_end())
}

/// 扫描到末尾时是否仍处于引号内。
fn inside_string(text: &str) -> bool {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in text.chars() {
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == q {
                quote = None;
            }
        } else if ch == '"' || ch == '\'' {
            quote = Some(ch);
        }
    }
    quote.is_some()
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
        // 坏声明被丢弃，同文件后续规则仍要解析出来；没声明的规则不留。
        let doc = parse("a.wxss", ".a { color red; ; } .b { width: 1px; }").unwrap();
        assert_eq!(doc.rules.len(), 1);
        assert_eq!(doc.rules[0].selectors[0].text, ".b");
        assert_eq!(doc.rules[0].declarations[0].value, "1px");
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
        // .wxss 里 `//` 不是注释：原样落进选择器文本（规则层会跳过解析不了的选择器）
        let wxss = parse("a.wxss", "// x\n.b { color: red; }").unwrap();
        assert!(wxss.rules[0].selectors[0].text.contains("//"));
    }

    #[test]
    fn interpolation_in_selector_is_kept_verbatim() {
        let doc = parse("a.scss", ".icon-#{$name} { color: red; }").unwrap();
        assert_eq!(doc.rules[0].selectors[0].text, ".icon-#{$name}");
    }

    #[test]
    fn empty_stylesheet_is_an_error() {
        assert!(parse("a.wxss", "").is_err());
        assert!(parse("a.wxss", "  \n\t ").is_err());
        assert!(parse("a.wxss", "/* only a comment */").is_err());
    }
}
