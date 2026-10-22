//! JS / TS 视图。
//!
//! 只做「解析 + 定位」：把源文件解析成 AST，抽出所有函数，以及函数体的指令序言里
//! 有没有 `"worklet"`。不做作用域分析、不建调用图——那些属于规则层。
//!
//! 语法交给 `oxc_parser`，这里只遍历一遍 AST 收集函数。`.js` / `.ts` 的差异由
//! `SourceType::from_path` 按后缀决定（脚本还是模块、要不要解析类型）；`.wxs` 是
//! 独立方言，不在此列，由调用方挡掉。
//!
//! 容错策略与 `input/wxss.rs` 一致：不 panic，解析失败返回 `Err`，由调用方决定是否
//! 跳过。差别在于这里不接受可恢复的语法错误：oxc 能带着错误产出一棵「大致像样」的
//! AST，但错位之后的节点形状不可信，规则在它上面报的告警多半是误报，宁可整份跳过。

use std::fmt;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ArrowFunctionBody, ArrowFunctionExpression, BindingPattern, Expression, Function, FunctionBody,
    MethodDefinition, ObjectProperty, PropertyDefinition, PropertyKey, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use oxc_parser::Parser;
use oxc_span::{SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

/// worklet 函数的标记：函数体指令序言里的 `"worklet"`。
const WORKLET: &str = "worklet";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// 不含任何有效内容：空文件或只有注释。
    Empty,
    /// 语法错误，整份文件无法建立可信结构。
    Syntax(String),
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("脚本文件不含任何有效内容"),
            Self::Syntax(detail) => f.write_str(detail),
        }
    }
}

/// 一份解析后的 JS/TS 文档。
///
/// AST 借用 `Allocator`，出了 `parse` 就没法活；所以解析时就把要对外暴露的函数信息
/// 拷成 owned 形态（`source` 是调用方的切片），`functions()` 再从 owned 数据借出。
/// 这样结构体没有自引用，也不需要 unsafe。
#[derive(Debug, Clone)]
pub struct JsDoc<'a> {
    path: String,
    source: &'a str,
    functions: Vec<OwnedFunction>,
}

impl<'a> JsDoc<'a> {
    pub fn parse(path: &str, source: &'a str) -> Result<Self, ParseError> {
        let source_type = SourceType::from_path(path)
            .map_err(|_| ParseError::Syntax(format!("不支持的文件后缀：{path}")))?;

        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, source, source_type).parse();
        // 只有 error 级诊断才否定整份文件；warning 不影响结构。
        if let Some(error) = ret.diagnostics.errors().next() {
            return Err(ParseError::Syntax(error.to_string()));
        }
        if ret.program.body.is_empty() && ret.program.directives.is_empty() {
            return Err(ParseError::Empty);
        }

        let mut collector = Collector::default();
        collector.visit_program(&ret.program);

        Ok(Self {
            path: path.to_string(),
            source,
            functions: collector.functions,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn source(&self) -> &'a str {
        self.source
    }

    /// 所有函数（函数声明、函数表达式、箭头函数、对象方法、类方法），按遍历顺序。
    pub fn functions(&self) -> Vec<FunctionInfo<'_>> {
        self.functions
            .iter()
            .map(|f| FunctionInfo {
                name: f.name.as_deref(),
                has_worklet: f.has_worklet,
                start: f.start,
                end: f.end,
                body_directives: f.body_directives.iter().map(String::as_str).collect(),
            })
            .collect()
    }

    /// 字节偏移 → `(line, column)`，均 1-based，`column` 按 Unicode 字符计。
    ///
    /// 与 `WxssDoc::position` 同语义：偏移落在多字节字符中间时向下取整到该字符起点，
    /// 超出文件长度时取文件末尾。
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

/// 一个函数及其 worklet 标记。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionInfo<'a> {
    /// 声明名 / 变量名 / 方法名；匿名函数为 `None`。
    pub name: Option<&'a str>,
    /// 指令序言里是否有 `"worklet"`。
    pub has_worklet: bool,
    /// 函数整体（含 `function` 关键字 / 参数表）的字节区间。
    pub start: u32,
    pub end: u32,
    /// 函数体指令序言的全部指令原文，按出现顺序。
    pub body_directives: Vec<&'a str>,
}

/// 对外数据用 owned 形态存，避开「Allocator + 借用它的 AST」的自引用。
#[derive(Debug, Clone)]
struct OwnedFunction {
    name: Option<String>,
    has_worklet: bool,
    start: u32,
    end: u32,
    body_directives: Vec<String>,
}

#[derive(Default)]
struct Collector {
    functions: Vec<OwnedFunction>,
    /// 下一个函数表达式 / 箭头函数该沿用的名字（变量名或方法名）。
    pending: Option<String>,
}

impl Collector {
    fn push(&mut self, name: Option<String>, span: Span, body: Option<&FunctionBody>) {
        let body_directives: Vec<String> = body
            .map(|b| {
                b.directives
                    .iter()
                    .map(|d| d.directive.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();
        self.functions.push(OwnedFunction {
            has_worklet: body_directives.iter().any(|d| d == WORKLET),
            name,
            start: span.start,
            end: span.end,
            body_directives,
        });
    }
}

impl<'a> Visit<'a> for Collector {
    fn visit_function(&mut self, it: &Function<'a>, flags: ScopeFlags) {
        let name = it
            .id
            .as_ref()
            .map(|id| id.name.as_str().to_string())
            .or_else(|| self.pending.take());
        // 无论名字来自哪里，pending 都属于这个函数，不能漏给它的内部函数。
        self.pending = None;
        self.push(name, it.span, it.body.as_deref());
        walk::walk_function(self, it, flags);
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let name = self.pending.take();
        let body = match &it.body {
            ArrowFunctionBody::FunctionBody(body) => Some(body.as_ref()),
            _ => None,
        };
        self.push(name, it.span, body);
        walk::walk_arrow_function_expression(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        // 只认「初始化值本身就是函数」的情况：`const f = wrap(() => {})` 里的箭头
        // 不该叫 f。
        if it.init.as_ref().is_some_and(is_function) {
            self.pending = binding_name(&it.id);
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if is_function(&it.value) {
            self.pending = key_name(&it.key);
        }
        walk::walk_object_property(self, it);
    }

    fn visit_method_definition(&mut self, it: &MethodDefinition<'a>) {
        self.pending = key_name(&it.key);
        walk::walk_method_definition(self, it);
    }

    fn visit_property_definition(&mut self, it: &PropertyDefinition<'a>) {
        if it.value.as_ref().is_some_and(is_function) {
            self.pending = key_name(&it.key);
        }
        walk::walk_property_definition(self, it);
    }
}

fn is_function(expr: &Expression) -> bool {
    matches!(
        expr,
        Expression::FunctionExpression(_) | Expression::ArrowFunctionExpression(_)
    )
}

fn binding_name(pattern: &BindingPattern) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_string()),
        _ => None,
    }
}

/// 属性名；计算属性（`[x]`、`[x + 1]`）与私有名拿不到静态值，返回 `None`。
fn key_name(key: &PropertyKey) -> Option<String> {
    key.static_name()
        .and_then(|name| name.as_js_str().as_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_js(src: &str) -> JsDoc<'_> {
        JsDoc::parse("a.js", src).expect("应当解析成功")
    }

    #[test]
    fn function_declaration_worklet() {
        let src = "function f() { 'worklet'; }";
        let doc = parse_js(src);
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, Some("f"));
        assert!(fns[0].has_worklet);
        assert_eq!(fns[0].body_directives, ["worklet"]);
        assert_eq!((fns[0].start, fns[0].end), (0, src.len() as u32));
        assert_eq!(doc.position(fns[0].start as usize), (1, 1));
    }

    #[test]
    fn arrow_function_worklet() {
        let doc = parse_js("const f = () => { \"worklet\"; };");
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, Some("f"));
        assert!(fns[0].has_worklet);
        assert_eq!(fns[0].body_directives, ["worklet"]);
    }

    #[test]
    fn worklet_after_use_strict() {
        // `"use strict"` 同样算序言，两个指令都该记下来。
        let doc = parse_js("const f = function () { 'use strict'; 'worklet'; };");
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert!(fns[0].has_worklet);
        assert_eq!(fns[0].body_directives, ["use strict", "worklet"]);
    }

    #[test]
    fn worklet_outside_directive_prologue() {
        let doc = parse_js("function f() { const x = 1; 'worklet'; }");
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert!(!fns[0].has_worklet);
        assert!(fns[0].body_directives.is_empty());
    }

    #[test]
    fn object_and_class_methods() {
        let src = "const o = { m() { 'worklet'; }, n: () => { 'worklet'; } }; \
                   class C { p() { 'worklet'; } q = () => { 'use strict'; }; }";
        let doc = parse_js(src);
        let fns = doc.functions();
        let names: Vec<_> = fns.iter().map(|f| f.name).collect();
        assert_eq!(names, [Some("m"), Some("n"), Some("p"), Some("q")]);
        assert_eq!(
            fns.iter().map(|f| f.has_worklet).collect::<Vec<_>>(),
            [true, true, true, false]
        );
        assert_eq!(fns[3].body_directives, ["use strict"]);
    }

    #[test]
    fn nested_functions_are_all_collected() {
        let doc = parse_js("function outer() { const inner = () => { 'worklet'; }; }");
        let fns = doc.functions();
        assert_eq!(fns.len(), 2);
        assert_eq!(fns[0].name, Some("outer"));
        assert!(!fns[0].has_worklet);
        assert_eq!(fns[1].name, Some("inner"));
        assert!(fns[1].has_worklet);
    }

    #[test]
    fn empty_and_syntax_error() {
        assert!(matches!(JsDoc::parse("a.js", ""), Err(ParseError::Empty)));
        assert!(matches!(
            JsDoc::parse("a.js", "// 只有注释\n"),
            Err(ParseError::Empty)
        ));
        assert!(matches!(
            JsDoc::parse("a.js", "function ("),
            Err(ParseError::Syntax(_))
        ));
    }

    #[test]
    fn module_kind_follows_content() {
        // `.js` 是 Unambiguous：有 `import` / `export` 按 ESM 解析，否则按脚本。
        // 小程序的两种写法都得能解析，否则后续规则会整片跳过。
        let esm = JsDoc::parse(
            "a.js",
            "import { x } from './x';\nexport default function f() { 'worklet'; }",
        )
        .expect("ESM 应当解析成功");
        assert_eq!(esm.functions()[0].name, Some("f"));

        let cjs = JsDoc::parse(
            "a.js",
            "module.exports = { f: function () { 'worklet'; } };",
        )
        .expect("CommonJS 应当解析成功");
        let fns = cjs.functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, Some("f"));
        assert!(fns[0].has_worklet);
    }

    #[test]
    fn typescript_source() {
        let src = "const f = (n: number): number => { 'worklet'; return n; };";
        let doc = JsDoc::parse("a.ts", src).expect("TS 应当解析成功");
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, Some("f"));
        assert!(fns[0].has_worklet);
    }

    #[test]
    fn unsupported_extension() {
        assert!(matches!(
            JsDoc::parse("a.wxs", "var a = 1;"),
            Err(ParseError::Syntax(_))
        ));
    }
}
