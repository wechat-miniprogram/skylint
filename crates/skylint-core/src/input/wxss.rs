//! WXSS 视图。
//!
//! 只做「解析 + 定位」：把样式表拆成规则、选择器、声明、`@import`，保留原始文本与
//! 字节偏移。不做值解析、选择器匹配、级联——那些属于规则层或编译器。
//!
//! 容错策略：语法坏块逐块丢弃，不因单条规则失败而让整个文件不可用；只有整份文件
//! 读不出任何 token（空文件 / 纯空白 / 纯注释）时才返回 `Err`。

use cssparser::{
    AtRuleParser, CowRcStr, Delimiter, ParseError, Parser, ParserInput, ParserState,
    QualifiedRuleParser, StyleSheetParser, Token,
};

/// 一份解析后的 WXSS 文档。
#[derive(Debug, Clone)]
pub struct WxssDoc {
    path: String,
    source: String,
    /// 样式规则（含 at-rule 内的嵌套规则），按源码顺序。
    pub rules: Vec<WxssRule>,
    /// `@import` 的目标路径，按源码顺序。P0 只收集，不解析依赖图。
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

/// 解析 WXSS。`path` 只用于回填诊断路径。
pub fn parse(path: &str, source: &str) -> Result<WxssDoc, String> {
    if !has_token(source) {
        return Err("WXSS 文件不含任何有效 token".to_string());
    }

    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    let mut collector = Collector::default();
    for result in StyleSheetParser::new(&mut parser, &mut collector) {
        // 坏规则直接丢弃：lint 场景下局部错误不应让整份样式表失效。
        let _ = result;
    }

    Ok(WxssDoc {
        path: path.to_string(),
        source: source.to_string(),
        rules: collector.rules,
        imports: collector.imports,
    })
}

/// 文件里是否存在非空白、非注释的 token。
fn has_token(source: &str) -> bool {
    let mut input = ParserInput::new(source);
    let mut parser = Parser::new(&mut input);
    loop {
        match parser.next_including_whitespace_and_comments() {
            Ok(Token::WhiteSpace(_) | Token::Comment(_)) => continue,
            Ok(_) => return true,
            Err(_) => return false,
        }
    }
}

const MAX_AT_RULE_DEPTH: usize = 32;

#[derive(Default)]
struct Collector {
    rules: Vec<WxssRule>,
    imports: Vec<WxssImport>,
    at_rules: Vec<WxssAtRule>,
}

impl<'i> QualifiedRuleParser<'i> for Collector {
    type Prelude = Vec<WxssSelector>;
    type QualifiedRule = ();
    type Error = ();

    fn parse_prelude(
        &mut self,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace().is_ok() {}
        Ok(split_selectors(input.slice_from(start), start.byte_index()))
    }

    fn parse_block(
        &mut self,
        selectors: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self::QualifiedRule, ParseError<'i, Self::Error>> {
        let mut rule = WxssRule {
            selectors,
            declarations: Vec::new(),
            at_rules: self.at_rules.clone(),
        };
        parse_declarations(input, &mut rule.declarations);
        if !rule.selectors.is_empty() {
            self.rules.push(rule);
        }
        Ok(())
    }
}

impl<'i> AtRuleParser<'i> for Collector {
    /// `(小写名, prelude 原文, prelude 起点)`
    type Prelude = (String, String, usize);
    type AtRule = ();
    type Error = ();

    fn parse_prelude(
        &mut self,
        name: CowRcStr<'i>,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self::Prelude, ParseError<'i, Self::Error>> {
        let start = input.position();
        while input.next_including_whitespace().is_ok() {}
        Ok((
            name.to_ascii_lowercase(),
            input.slice_from(start).trim().to_string(),
            start.byte_index(),
        ))
    }

    fn rule_without_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
    ) -> Result<Self::AtRule, ()> {
        let (name, text, offset) = prelude;
        if name == "import" {
            if let Some((target, target_offset)) = import_target(&text, offset) {
                self.imports.push(WxssImport {
                    target,
                    offset: target_offset,
                });
            }
        }
        Ok(())
    }

    fn parse_block(
        &mut self,
        prelude: Self::Prelude,
        _start: &ParserState,
        input: &mut Parser<'i, '_>,
    ) -> Result<Self::AtRule, ParseError<'i, Self::Error>> {
        let (name, text, offset) = prelude;
        if !is_rule_container(&name) || self.at_rules.len() >= MAX_AT_RULE_DEPTH {
            // @keyframes / @font-face 等：块内不是样式规则，整块跳过。
            return Ok(());
        }
        self.at_rules.push(WxssAtRule {
            name,
            prelude: text,
            offset,
        });
        for result in StyleSheetParser::new(input, self) {
            let _ = result;
        }
        self.at_rules.pop();
        Ok(())
    }
}

/// 块内仍然是「样式规则列表」的 at-rule。其余（keyframes/font-face/page…）的块内容
/// 不是普通声明列表，P0 直接跳过。
fn is_rule_container(name: &str) -> bool {
    matches!(name, "media" | "supports" | "container" | "layer" | "scope")
}

/// 从 `@import` 的 prelude 里取目标路径，返回 `(target, 源文件偏移)`。
fn import_target(prelude: &str, base: usize) -> Option<(String, usize)> {
    let trimmed = prelude.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (raw, lead) = if let Some(rest) = trimmed.strip_prefix("url(") {
        let inner = rest.split(')').next().unwrap_or(rest).trim();
        (inner, trimmed.len() - rest.len())
    } else {
        let inner = trimmed.split_whitespace().next()?;
        (inner, 0)
    };
    let target = raw.trim_matches(['"', '\'']).to_string();
    if target.is_empty() {
        return None;
    }
    Some((target, base + lead))
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

/// 逐条解析声明；坏声明跳过后继续。
fn parse_declarations<'i>(input: &mut Parser<'i, '_>, out: &mut Vec<WxssDeclaration>) {
    loop {
        input.skip_whitespace();
        if input.is_exhausted() {
            break;
        }
        let state = input.state();
        let decl_start = state.position().byte_index();

        let parsed = input.try_parse(|i| -> Result<WxssDeclaration, ParseError<'i, ()>> {
            let name = i.expect_ident_cloned()?;
            i.expect_colon()?;
            let (value, value_offset, important) = i.parse_until_after(
                Delimiter::Semicolon,
                |inner| -> Result<(String, usize, bool), ParseError<'i, ()>> {
                    let value_start = inner.position();
                    while inner.next_including_whitespace().is_ok() {}
                    let raw = inner.slice_from(value_start);
                    Ok(normalize_value(raw, value_start.byte_index()))
                },
            )?;
            Ok(WxssDeclaration {
                name: name.to_string(),
                value,
                important,
                offset: decl_start,
                value_offset,
            })
        });

        match parsed {
            Ok(decl) => out.push(decl),
            Err(_) => {
                input.reset(&state);
                let _ = input.parse_until_after(Delimiter::Semicolon, |inner| {
                    while inner.next_including_whitespace().is_ok() {}
                    Ok::<(), ParseError<'i, ()>>(())
                });
                // 兜底：极端输入下没有前进时强推一个 token，避免死循环。
                if input.position().byte_index() == decl_start {
                    let _ = input.next_including_whitespace();
                }
            }
        }
    }
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
        // 坏声明被丢弃，同文件后续规则仍要解析出来。
        let doc = parse("a.wxss", ".a { color red; ; } .b { width: 1px; }").unwrap();
        assert_eq!(doc.rules.len(), 2);
        assert_eq!(doc.rules[1].selectors[0].text, ".b");
        assert_eq!(doc.rules[1].declarations[0].value, "1px");
    }

    #[test]
    fn empty_stylesheet_is_an_error() {
        assert!(parse("a.wxss", "").is_err());
        assert!(parse("a.wxss", "  \n\t ").is_err());
        assert!(parse("a.wxss", "/* only a comment */").is_err());
    }
}
