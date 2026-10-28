//! JS / TS 视图。
//!
//! 只做「解析 + 定位」：把源文件解析成 AST，抽出所有函数（含函数体的指令序言里
//! 有没有 `"worklet"`）与所有调用表达式（callee 的点号链文本、实参对象字面量的
//! 静态键、调用点是否位于 worklet 函数内）。不做作用域分析、不建调用图——那些
//! 属于规则层。
//!
//! 语法交给 `oxc_parser`，这里只遍历一遍 AST。`.js` / `.ts` 的差异由
//! `SourceType::from_path` 按后缀决定（脚本还是模块、要不要解析类型）；`.wxs` 是
//! 独立方言，不在此列，由调用方挡掉。
//!
//! 容错策略与 `input/wxss.rs` 一致：不 panic，解析失败返回 `Err`，由调用方决定是否
//! 跳过。差别在于这里不接受可恢复的语法错误：oxc 能带着错误产出一棵「大致像样」的
//! AST，但错位之后的节点形状不可信，规则在它上面报的告警多半是误报，宁可整份跳过。

use std::fmt;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrowFunctionBody, ArrowFunctionExpression, AssignmentExpression, AssignmentTarget,
    BindingPattern, CallExpression, ChainElement, Expression, Function, FunctionBody,
    MethodDefinition, ObjectExpression, ObjectProperty, ObjectPropertyKind, PropertyDefinition,
    PropertyKey, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use std::collections::{BTreeMap, BTreeSet};

use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::scope::ScopeFlags;

use crate::input::env::{Env, Lit};

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
/// AST 借用 `Allocator`，出了 `parse` 就没法活；所以解析时就把要对外暴露的信息
/// 拷成 owned 形态，`functions()` / `calls()` 再从 owned 数据借出。这样结构体没有
/// 自引用，也不需要 unsafe，`Project` 可以长期持有它。
#[derive(Debug, Clone)]
pub struct JsDoc {
    path: String,
    source: String,
    functions: Vec<OwnedFunction>,
    calls: Vec<CallInfo>,
    this_data_destructures: Vec<ThisDataDestructure>,
    object_fns: Vec<ObjectFn>,
    literals: BTreeMap<String, Lit>,
    dirty_data: BTreeSet<String>,
    dirty_all: bool,
}

impl JsDoc {
    pub fn parse(path: &str, source: &str) -> Result<Self, ParseError> {
        let source_type = SourceType::from_path(path)
            .map_err(|_| ParseError::Syntax(format!("不支持的文件后缀：{path}")))?;

        let owned = source.to_string();
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &owned, source_type)
            // 括号不保留成 `ParenthesizedExpression`：`Page(({ onPageScroll() {} }))`、
            // `wx.navigateTo(({ routeType: 'x' }))` 与不加括号的写法必须得到同一棵树，
            // 否则实参/callee 判定会静默丢失一层。
            .with_options(ParseOptions {
                preserve_parens: false,
                ..ParseOptions::default()
            })
            .parse();
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
            source: owned,
            functions: collector.functions,
            calls: collector.calls,
            this_data_destructures: collector.this_data_destructures,
            object_fns: collector.object_fns,
            literals: collector.literals,
            dirty_data: collector.dirty_data,
            dirty_all: collector.dirty_all,
        })
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// `properties` 默认值里能确定取值的绑定（已被动态写入的键扣掉）。
    ///
    /// 只有 `properties` 参与：`data` 是初始化值，页面/组件常用 `onLoad` 后的
    /// `setData` 把它改成运行时值，而 `dirty_data` 只能看到显式写过的键。
    pub fn constants(&self) -> Env {
        let mut env = Env::default();
        if self.dirty_all {
            return env;
        }
        for (name, value) in &self.literals {
            if self.dirty_data.contains(name) {
                continue;
            }
            env.insert(name.clone(), value.clone());
        }
        env
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
                body_start: f.body_start,
                body_end: f.body_end,
                body_directives: f.body_directives.iter().map(String::as_str).collect(),
            })
            .collect()
    }

    /// 所有调用表达式，按遍历顺序（外层调用先于它实参里的调用）。
    pub fn calls(&self) -> &[CallInfo] {
        &self.calls
    }

    /// 所有 `const { .. } = this.data` / `let { .. } = this.data` 形式的声明。
    ///
    /// 只暴露这一个形状，不做通用 AST 暴露：worklet 的数据冻结规则（#156）只需要
    /// 「解构了 `this.data`」与「在不在 worklet 里」，`calls()` 看不到解构。
    pub fn this_data_destructures(&self) -> &[ThisDataDestructure] {
        &self.this_data_destructures
    }

    /// 所有对象字面量里「值是函数、或指向函数的裸标识符」的属性，按遍历顺序。
    ///
    /// 路由动画的约束落在 builder **返回对象**的 `handle*Animation` 键上（#153），
    /// 而 `calls()` 只看得到 builder 的实参、看不到它 return 了什么。
    pub fn object_fns(&self) -> &[ObjectFn] {
        &self.object_fns
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
    /// 函数体的字节区间（表达式体箭头函数取表达式本身）。
    pub body_start: u32,
    pub body_end: u32,
    /// 函数体指令序言的全部指令原文，按出现顺序。
    pub body_directives: Vec<&'a str>,
}

/// 一次调用表达式。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallInfo {
    /// 点号链文本，如 `wx.createSelectorQuery`、`this.animate`、
    /// `scrollViewContext.scrollTo`、`foo`。
    ///
    /// 链的前缀是调用结果时（`createSelectorQuery().select('.x').ref(cb)`）没有静态
    /// 名字，用空串占位，于是 callee 是 `.ref`——链尾仍然可判定。计算属性、括号、
    /// 逻辑表达式等真正取不到结构的写法给空串，规则靠空串跳过。
    pub callee: String,
    /// 整个调用表达式的字节区间。
    pub start: u32,
    pub end: u32,
    pub args: Vec<ArgInfo>,
    /// 最近的外层函数是否 `has_worklet`；不在任何函数内为 `false`。
    pub worklet_scope: bool,
}

/// 一次调用的实参。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArgInfo {
    pub start: u32,
    pub end: u32,
    /// 实参是对象字面量时，静态属性名（字符串键与标识符键都算，计算键不算）；
    /// 不是对象字面量为空。
    pub object_keys: Vec<String>,
    /// `object_keys` 里每个键的字节区间，与它同序。
    pub object_key_spans: Vec<(u32, u32)>,
    /// `object_keys` 里每个键的值，与它同序：值是字符串字面量时给出内容，其它
    /// 形态（变量、模板字符串、调用结果等）为 `None`。
    pub object_string_values: Vec<Option<String>>,
    /// 实参是内联函数（箭头函数 / 函数表达式）时，该函数是否声明了 `'worklet'`；
    /// 其它写法为 `None`。
    pub fn_worklet: Option<bool>,
    /// 实参是裸标识符时的名字；成员访问、调用结果等写法为 `None`。
    pub ident: Option<String>,
}

/// 一次 `{ .. } = this.data` 解构声明。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThisDataDestructure {
    /// 整个声明器（`{ a } = this.data`）的字节区间。
    pub start: u32,
    pub end: u32,
    /// 是否位于 worklet 函数体内（同样取最近的外层函数）。
    pub worklet_scope: bool,
}

/// 对象字面量里「值是函数、或指向函数的裸标识符」的一个属性。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObjectFn {
    /// 静态键名；计算键不收录。
    pub key: String,
    /// 键的位置，诊断落点（字符串键含引号）。
    pub key_start: u32,
    pub key_end: u32,
    /// 值是内联函数时，该函数是否声明了 `'worklet'`。
    pub fn_worklet: Option<bool>,
    /// 值是裸标识符时的名字，指向同文件里的函数。
    pub ident: Option<String>,
}

impl ArgInfo {
    /// 某个静态键的位置；键不存在时返回 `None`。
    pub fn key_span(&self, key: &str) -> Option<(u32, u32)> {
        let index = self.object_keys.iter().position(|name| name == key)?;
        self.object_key_spans.get(index).copied()
    }
}

/// 对外数据用 owned 形态存，避开「Allocator + 借用它的 AST」的自引用。
#[derive(Debug, Clone)]
struct OwnedFunction {
    name: Option<String>,
    has_worklet: bool,
    start: u32,
    end: u32,
    body_start: u32,
    body_end: u32,
    body_directives: Vec<String>,
}

#[derive(Default)]
struct Collector {
    functions: Vec<OwnedFunction>,
    calls: Vec<CallInfo>,
    this_data_destructures: Vec<ThisDataDestructure>,
    object_fns: Vec<ObjectFn>,
    /// `Page({data})` / `Component({data, properties})` 里的字面量取值。
    literals: BTreeMap<String, Lit>,
    /// 被动态写入过的 data 键：`setData({k: 变量})` 或 `setData` 里出现动态键。
    dirty_data: BTreeSet<String>,
    /// 整份文件的 data 都被动态写入（`setData(变量)` / 展开语法），所有键都不再确定。
    dirty_all: bool,
    /// 下一个函数表达式 / 箭头函数该沿用的名字（变量名或方法名）。
    pending: Option<String>,
    /// 外层函数栈，值为该函数是否 `has_worklet`。取最近的一层。
    worklet_stack: Vec<bool>,
}

impl Collector {
    fn push_function(
        &mut self,
        name: Option<String>,
        span: Span,
        body: Option<&FunctionBody>,
        body_span: Span,
    ) {
        let body_directives: Vec<String> = body
            .map(|b| {
                b.directives
                    .iter()
                    .map(|d| d.directive.as_str().to_string())
                    .collect()
            })
            .unwrap_or_default();
        let has_worklet = body_directives.iter().any(|d| d == WORKLET);
        self.functions.push(OwnedFunction {
            has_worklet,
            name,
            start: span.start,
            end: span.end,
            body_start: body_span.start,
            body_end: body_span.end,
            body_directives,
        });
        self.worklet_stack.push(has_worklet);
    }

    fn worklet_scope(&self) -> bool {
        self.worklet_stack.last().copied().unwrap_or(false)
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
        let body = it.body.as_deref();
        // 无函数体（TS `declare function` / overload）不是有效作用域，占位到函数末尾。
        let body_span = body.map_or(it.span, |b| b.span);
        self.push_function(name, it.span, body, body_span);
        walk::walk_function(self, it, flags);
        self.worklet_stack.pop();
    }

    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        let name = self.pending.take();
        let (body, body_span) = match &it.body {
            ArrowFunctionBody::FunctionBody(body) => (Some(body.as_ref()), body.span),
            other => (
                None,
                other.as_expression().map_or(it.span, |expr| expr.span()),
            ),
        };
        self.push_function(name, it.span, body, body_span);
        walk::walk_arrow_function_expression(self, it);
        self.worklet_stack.pop();
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        // 只认「初始化值本身就是函数」的情况：`const f = wrap(() => {})` 里的箭头
        // 不该叫 f。
        if it.init.as_ref().is_some_and(is_function) {
            self.pending = binding_name(&it.id);
        }
        if matches!(it.id, BindingPattern::ObjectPattern(_))
            && it.init.as_ref().is_some_and(is_this_data)
        {
            self.this_data_destructures.push(ThisDataDestructure {
                start: it.span.start,
                end: it.span.end,
                worklet_scope: self.worklet_scope(),
            });
        }
        walk::walk_variable_declarator(self, it);
    }

    fn visit_object_property(&mut self, it: &ObjectProperty<'a>) {
        if is_function(&it.value) {
            self.pending = key_name(&it.key);
        }
        if let Some(item) = object_fn(it) {
            self.object_fns.push(item);
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

    fn visit_assignment_expression(&mut self, it: &AssignmentExpression<'a>) {
        // `this.data.x = ...` 也改写了绑定；`Object.assign(this.data, ...)` 之类整体写入
        // 拿不到键名，按「整份 data 不可信」处理。
        match &it.left {
            AssignmentTarget::StaticMemberExpression(member)
                if member.property.name == "data"
                    && matches!(&member.object, Expression::ThisExpression(_)) =>
            {
                self.dirty_all = true;
            }
            AssignmentTarget::StaticMemberExpression(member) if is_this_data(&member.object) => {
                let key = member.property.name.to_string();
                self.literals.remove(&key);
                self.dirty_data.insert(key);
            }
            // `this.data['x'] =`：键是字面量时当静态键，否则整份 data 不可信。
            AssignmentTarget::ComputedMemberExpression(member) if is_this_data(&member.object) => {
                match &member.expression {
                    Expression::StringLiteral(literal) => {
                        if let Some(key) = literal.value.as_str().map(str::to_string) {
                            self.literals.remove(&key);
                            self.dirty_data.insert(key);
                        } else {
                            self.dirty_all = true;
                        }
                    }
                    _ => self.dirty_all = true,
                }
            }
            _ => {}
        }
        walk::walk_assignment_expression(self, it);
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let callee = callee_name(&it.callee).unwrap_or_default();
        if matches!(callee.as_str(), "Page" | "Component") {
            if let Some(Expression::ObjectExpression(object)) =
                it.arguments.first().and_then(arg_expression)
            {
                self.collect_properties_of(object);
            }
        } else if callee.ends_with(".setData") {
            match it.arguments.first().and_then(arg_expression) {
                Some(Expression::ObjectExpression(object)) => self.collect_set_data(object),
                // `setData(this.data)` / `setData({...rest})` 这类整体写入：整份 data 不可信。
                Some(_) => self.dirty_all = true,
                None => {}
            }
        } else if callee == "Object.assign"
            && it
                .arguments
                .first()
                .and_then(arg_expression)
                .is_some_and(is_this_data)
        {
            // `Object.assign(this.data, …)` 拿不到键名，按整份 data 不可信处理。
            self.dirty_all = true;
        }
        self.calls.push(CallInfo {
            callee: callee_name(&it.callee).unwrap_or_default(),
            start: it.span.start,
            end: it.span.end,
            args: it.arguments.iter().map(arg_info).collect(),
            worklet_scope: self.worklet_scope(),
        });
        walk::walk_call_expression(self, it);
    }
}

impl Collector {
    /// `Component({ properties })`：只收默认值里的字面量。
    fn collect_properties_of<'b>(&mut self, object: &ObjectExpression<'b>) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if property.computed {
                continue;
            }
            // `data` 刻意不参与：它是初始化值，页面/组件随后会反复 `setData`，
            // 只有 `properties` 的默认值在没有被写入时才是稳定的。
            if key_name(&property.key).as_deref() == Some("properties")
                && let Expression::ObjectExpression(properties) = &property.value
            {
                self.collect_properties(properties);
            }
        }
    }

    /// `properties: { renderer: { type: String, value: 'skyline' } }`。
    fn collect_properties<'b>(&mut self, object: &ObjectExpression<'b>) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                continue;
            };
            if property.computed {
                continue;
            }
            let Some(key) = key_name(&property.key) else {
                continue;
            };
            let Expression::ObjectExpression(definition) = &property.value else {
                continue;
            };
            for field in &definition.properties {
                let ObjectPropertyKind::ObjectProperty(field) = field else {
                    continue;
                };
                if field.computed || key_name(&field.key).as_deref() != Some("value") {
                    continue;
                }
                if let Some(literal) = literal(&field.value) {
                    self.literals.insert(key.clone(), literal);
                }
            }
        }
    }

    /// `setData` 里出现过的键一律不再是常量：无论赋的是不是字面量，运行时都会被
    /// 改写，模板里也就不能再按初始值折叠。
    fn collect_set_data<'b>(&mut self, object: &ObjectExpression<'b>) {
        for property in &object.properties {
            let ObjectPropertyKind::ObjectProperty(property) = property else {
                self.dirty_all = true;
                continue;
            };
            match key_name(&property.key) {
                Some(key) if !property.computed => {
                    self.literals.remove(&key);
                    self.dirty_data.insert(key);
                }
                _ => self.dirty_all = true,
            }
        }
    }
}

/// 字面量：字符串 / 数字 / 布尔。其余（对象、数组、变量、调用）都算不可知。
fn literal(expression: &Expression) -> Option<Lit> {
    match expression {
        Expression::StringLiteral(value) => Some(Lit::Str(
            value.value.as_str().unwrap_or_default().to_string(),
        )),
        Expression::NumericLiteral(value) => Some(Lit::Num(value.value)),
        Expression::BooleanLiteral(value) => Some(Lit::Bool(value.value)),
        // 不含插值的模板串就是普通字符串。
        Expression::TemplateLiteral(template) if template.expressions.is_empty() => {
            let quasi = template.quasis.first()?;
            Some(Lit::Str(match &quasi.value.cooked {
                Some(cooked) => cooked.as_str().unwrap_or_default().to_string(),
                None => quasi.value.raw.to_string(),
            }))
        }
        _ => None,
    }
}

/// 调用实参的表达式（展开实参不算）。
fn arg_expression<'a>(argument: &'a Argument<'a>) -> Option<&'a Expression<'a>> {
    argument.as_expression()
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

/// `this.data` 成员访问（只认静态属性名，`this['data']` 不算）。
fn is_this_data(expr: &Expression) -> bool {
    matches!(
        expr,
        Expression::StaticMemberExpression(member)
            if member.property.name == "data"
                && matches!(&member.object, Expression::ThisExpression(_))
    )
}

/// 对象字面量里值可判定的属性：内联函数或指向函数的裸标识符。
fn object_fn(property: &ObjectProperty) -> Option<ObjectFn> {
    if property.computed {
        return None;
    }
    let key = property.key.static_name()?.as_str()?.to_string();
    let (fn_worklet, ident) = match &property.value {
        Expression::ArrowFunctionExpression(arrow) => (Some(arrow_has_worklet(arrow)), None),
        Expression::FunctionExpression(function) => (Some(function_has_worklet(function)), None),
        Expression::Identifier(id) => (None, Some(id.name.as_str().to_string())),
        _ => return None,
    };
    Some(ObjectFn {
        key_start: property.key.span().start,
        key_end: property.key.span().end,
        key,
        fn_worklet,
        ident,
    })
}

/// 属性名；计算属性（`[x]`、`[x + 1]`）与私有名拿不到静态值，返回 `None`。
fn key_name(key: &PropertyKey) -> Option<String> {
    key.static_name()
        .and_then(|name| name.as_js_str().as_str().map(str::to_string))
}

/// 调用的 callee 文本。只解析点号链；见 [`CallInfo::callee`] 的取舍说明。
fn callee_name(expr: &Expression) -> Option<String> {
    match expr {
        Expression::Identifier(id) => Some(id.name.as_str().to_string()),
        Expression::ThisExpression(_) => Some("this".to_string()),
        Expression::StaticMemberExpression(member) => {
            member_name(&member.object, &member.property.name)
        }
        // `a?.b()` 整条链包在 `ChainExpression` 里；`a?.b` 作为 callee 时同理。
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::StaticMemberExpression(member) => {
                member_name(&member.object, &member.property.name)
            }
            ChainElement::CallExpression(call) => callee_name(&call.callee),
            _ => None,
        },
        _ => None,
    }
}

/// 前缀取不到（计算属性 / 括号 / 逻辑表达式）时整条链放弃，`None` 一路上抛。
fn member_name(object: &Expression, property: &str) -> Option<String> {
    Some(format!("{}.{property}", callee_prefix(object)?))
}

/// 成员链前缀。调用结果没有静态名字，用空串占位，让链尾（`.ref`、`.scrollTo`）
/// 仍可判定；计算属性、括号、逻辑表达式等结构不产出一部分文本，整条链放弃。
fn callee_prefix(expr: &Expression) -> Option<String> {
    match expr {
        Expression::CallExpression(_) => Some(String::new()),
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::CallExpression(_) => Some(String::new()),
            ChainElement::StaticMemberExpression(member) => {
                member_name(&member.object, &member.property.name)
            }
            _ => None,
        },
        _ => callee_name(expr),
    }
}

fn arg_info(arg: &Argument) -> ArgInfo {
    let span = arg.span();
    let (object_keys, object_key_spans, object_string_values) = match arg {
        Argument::ObjectExpression(object) => object_members(object),
        _ => (Vec::new(), Vec::new(), Vec::new()),
    };
    let (fn_worklet, ident) = match arg {
        Argument::ArrowFunctionExpression(arrow) => (Some(arrow_has_worklet(arrow)), None),
        Argument::FunctionExpression(function) => (Some(function_has_worklet(function)), None),
        Argument::Identifier(id) => (None, Some(id.name.as_str().to_string())),
        _ => (None, None),
    };
    ArgInfo {
        start: span.start,
        end: span.end,
        object_keys,
        object_key_spans,
        object_string_values,
        fn_worklet,
        ident,
    }
}

fn arrow_has_worklet(arrow: &ArrowFunctionExpression) -> bool {
    match &arrow.body {
        ArrowFunctionBody::FunctionBody(body) => has_worklet_directive(body),
        // 表达式体没有指令序言，不可能是 worklet。
        _ => false,
    }
}

fn function_has_worklet(function: &Function) -> bool {
    function.body.as_deref().is_some_and(has_worklet_directive)
}

fn has_worklet_directive(body: &FunctionBody) -> bool {
    body.directives
        .iter()
        .any(|directive| directive.directive.as_str() == WORKLET)
}

/// 对象字面量的静态键、键位置与字符串值。计算键（`[x]`）跳过——它可能不是静态
/// 属性名；值只认字符串字面量，其余形态记 `None`。
type ObjectMembers = (Vec<String>, Vec<(u32, u32)>, Vec<Option<String>>);

fn object_members(object: &ObjectExpression) -> ObjectMembers {
    let mut keys = Vec::new();
    let mut spans = Vec::new();
    let mut values = Vec::new();
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };
        if property.computed {
            continue;
        }
        let Some(name) = property.key.static_name() else {
            continue;
        };
        let Some(name) = name.as_str() else {
            continue;
        };
        let span = property.key.span();
        keys.push(name.to_string());
        spans.push((span.start, span.end));
        values.push(match &property.value {
            Expression::StringLiteral(literal) => literal.value.as_str().map(str::to_string),
            _ => None,
        });
    }
    (keys, spans, values)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_js(src: &str) -> JsDoc {
        JsDoc::parse("a.js", src).expect("应当解析成功")
    }

    fn callees(doc: &JsDoc) -> Vec<&str> {
        doc.calls().iter().map(|c| c.callee.as_str()).collect()
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
        // 函数体是 `{ ... }`，不含参数表。
        assert_eq!(doc.position(fns[0].body_start as usize), (1, 14));
        assert_eq!(fns[0].body_end, src.len() as u32);
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
        assert_eq!(doc.position(fns[0].body_start as usize), (1, 17));
    }

    #[test]
    fn expression_body_arrow_has_the_expression_as_body() {
        let src = "const f = () => g();";
        let doc = parse_js(src);
        let fns = doc.functions();
        assert_eq!(fns.len(), 1);
        assert!(!fns[0].has_worklet);
        assert_eq!(doc.position(fns[0].body_start as usize), (1, 17));
        assert_eq!(fns[0].body_end, (src.len() - 1) as u32);
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
    fn this_animate_call_and_its_arguments() {
        let src = "this.animate('.box', [], 1000);";
        let doc = parse_js(src);
        let calls = doc.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].callee, "this.animate");
        assert_eq!((calls[0].start, calls[0].end), (0, src.len() as u32 - 1));
        assert_eq!(calls[0].args.len(), 3);
        assert_eq!(
            calls[0]
                .args
                .iter()
                .map(|arg| &src[arg.start as usize..arg.end as usize])
                .collect::<Vec<_>>(),
            ["'.box'", "[]", "1000"]
        );
        assert!(!calls[0].worklet_scope);
    }

    #[test]
    fn parenthesized_arguments_are_transparent() {
        // `Page(({...}))` / `wx.navigateTo(({...}))` 与不加括号的写法必须同构。
        let doc =
            parse_js("Page(({ onPageScroll() {} })); wx.navigateTo(({ routeType: 'slide' }));");
        let calls = doc.calls();
        assert_eq!(calls[0].callee, "Page");
        assert_eq!(calls[0].args[0].object_keys, ["onPageScroll"]);
        assert_eq!(calls[1].callee, "wx.navigateTo");
        assert_eq!(calls[1].args[0].object_keys, ["routeType"]);
    }

    #[test]
    fn object_argument_keys_and_positions() {
        let src = "scrollViewContext.scrollTo(ref, { top: 1, 'size': 0.5, [k]: 2, pixels: 100 });";
        let doc = parse_js(src);
        let calls = doc.calls();
        // 外层 scrollTo 先记录，实参里的调用（这里没有）随后。
        assert_eq!(calls[0].callee, "scrollViewContext.scrollTo");
        let second = &calls[0].args[1];
        assert_eq!(second.object_keys, ["top", "size", "pixels"]);
        // 字符串键与标识符键都算，计算键不算。
        let (start, end) = second.key_span("size").unwrap();
        assert_eq!(&src[start as usize..end as usize], "'size'");
        let (start, _) = second.key_span("pixels").unwrap();
        assert_eq!(&src[start as usize..start as usize + 6], "pixels");
        // 非对象实参没有键。
        assert!(calls[0].args[0].object_keys.is_empty());
    }

    #[test]
    fn object_argument_string_values_align_with_keys() {
        let src = r#"wx.navigateTo({ url: 'a', routeType: "slide", dynamic: value, tpl: `x`, nested: { routeType: 'x' } });"#;
        let doc = parse_js(src);
        let arg = &doc.calls()[0].args[0];
        assert_eq!(
            arg.object_keys,
            ["url", "routeType", "dynamic", "tpl", "nested"]
        );
        // 与 object_keys 同序；只有字符串字面量取得到内容。
        assert_eq!(
            arg.object_string_values,
            [
                Some("a".to_string()),
                Some("slide".to_string()),
                None,
                None,
                None
            ]
        );
        // 非对象实参没有值列表。
        assert!(
            parse_js("f('x', 1);").calls()[0]
                .args
                .iter()
                .all(|arg| arg.object_string_values.is_empty())
        );
    }

    #[test]
    fn worklet_scope_follows_the_nearest_enclosing_function() {
        let src = r#"
            wx.worklet.shared(0);
            function outer() {
              this.animate('.a', [], 1);
              const animated = () => {
                'worklet';
                this.animate('.b', [], 1);
                const inner = () => { this.animate('.c', [], 1); };
              };
              this.animate('.d', [], 1);
            }
        "#;
        let doc = parse_js(src);
        let by_callee = |name: &str| {
            doc.calls()
                .iter()
                .filter(|call| call.callee == name)
                .collect::<Vec<_>>()
        };
        let animate = by_callee("this.animate");
        // 外层普通函数 / worklet 箭头里 / worklet 箭头内嵌的普通箭头 / 回到外层。
        assert_eq!(
            animate
                .iter()
                .map(|call| call.worklet_scope)
                .collect::<Vec<_>>(),
            [false, true, false, false]
        );
        // 内层普通箭头自己的调用不算 worklet（取最近一层）。
        assert!(!animate[2].worklet_scope);
        assert!(!doc.calls()[0].worklet_scope);
        assert_eq!(doc.calls()[0].callee, "wx.worklet.shared");
    }

    #[test]
    fn worklet_arrow_function_body_is_marked() {
        let src = "const f = () => { 'worklet'; scrollViewContext.scrollTo(ref, { top: 1 }); };";
        let doc = parse_js(src);
        assert_eq!(doc.calls().len(), 1);
        assert_eq!(doc.calls()[0].callee, "scrollViewContext.scrollTo");
        assert!(doc.calls()[0].worklet_scope);
    }

    #[test]
    fn computed_and_call_chain_callees() {
        let doc = parse_js(
            "a[b].c(); f()(); (a || b).x(); obj?.y(); this.a.b(); \
             wx.createSelectorQuery().select('.x').ref(cb);",
        );
        assert_eq!(
            callees(&doc),
            [
                // 计算属性：整条链放弃。
                "",
                // `f()()`：外层 callee 是调用表达式，不是点号链；内层是裸标识符。
                "",
                "f",
                // 括号内是逻辑表达式。
                "",
                // 可选链的静态部分保留。
                "obj.y",
                "this.a.b",
                // 调用链从外到内遍历：链尾先记，前缀没有名字用空串占位，所以
                // callee 是 `.ref` / `.select`，根调用是最后一个。
                ".ref",
                ".select",
                "wx.createSelectorQuery",
            ]
        );
    }

    #[test]
    fn page_option_object_key() {
        let src = "Page({ onPageScroll() {}, data: {} });";
        let doc = parse_js(src);
        let call = &doc.calls()[0];
        assert_eq!(call.callee, "Page");
        assert_eq!(call.args[0].object_keys, ["onPageScroll", "data"]);
        let (start, end) = call.args[0].key_span("onPageScroll").unwrap();
        assert_eq!(&src[start as usize..end as usize], "onPageScroll");
        // `wx.Page` 不是裸 `Page`。
        let doc = parse_js("wx.Page({ onPageScroll() {} });");
        assert_eq!(doc.calls()[0].callee, "wx.Page");
    }

    #[test]
    fn nested_calls_are_recorded_outer_first() {
        let doc = parse_js("foo(bar(), baz(qux()));");
        assert_eq!(callees(&doc), ["foo", "bar", "baz", "qux"]);
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

    #[test]
    fn argument_function_and_identifier_shape() {
        let src = "f(() => { 'worklet'; }, () => {}, function () { 'worklet'; }, function () {}, method, this.fn);";
        let doc = parse_js(src);
        let args = &doc.calls()[0].args;
        assert_eq!(
            args.iter().map(|arg| arg.fn_worklet).collect::<Vec<_>>(),
            [Some(true), Some(false), Some(true), Some(false), None, None]
        );
        assert_eq!(
            args.iter()
                .map(|arg| arg.ident.as_deref())
                .collect::<Vec<_>>(),
            [None, None, None, None, Some("method"), None]
        );
    }

    #[test]
    fn expression_body_arrow_argument_is_not_worklet() {
        let doc = parse_js("applyAnimatedStyle('.x', () => offset.value);");
        let arg = &doc.calls()[0].args[1];
        assert_eq!(arg.fn_worklet, Some(false));
        assert_eq!(arg.ident, None);
    }

    #[test]
    fn destructured_this_data_is_collected_with_scope() {
        let src = r#"
            const { a } = this.data;
            function normal() { let { b } = this.data; const c = this.data.c; const { d } = this.props; }
            function animated() { 'worklet'; const { e } = this.data; }
            function outer() { 'worklet'; const inner = () => { const { f } = this.data; }; }
            const { g } = other.data;
            const [h] = this.data;
        "#;
        let doc = parse_js(src);
        let found: Vec<(u32, bool)> = doc
            .this_data_destructures()
            .iter()
            .map(|d| (d.start, d.worklet_scope))
            .collect();
        // 顶层、普通函数内、worklet 内、worklet 内嵌普通函数：作用域取最近一层。
        assert_eq!(found.len(), 4, "{found:?}");
        assert_eq!(
            found.iter().map(|(_, scope)| *scope).collect::<Vec<_>>(),
            [false, false, true, false]
        );
        assert_eq!(doc.position(found[2].0 as usize), (4, 52));
    }
}
