//! JS / TS 视图。
//!
//! 只做「解析 + 定位」：把源文件解析成 AST，抽出所有函数（含函数体的指令序言里
//! 有没有 `"worklet"`）与所有调用表达式（callee 的点号链文本、实参对象字面量的
//! 静态键、调用点是否位于 worklet 函数内）。不做作用域分析、不建调用图——那些
//! 属于规则层。
//!
//! 语法交给 `oxc_parser`，这里只遍历一遍 AST。`.js` / `.ts` 的差异由
//! `SourceType::from_path` 按后缀决定（脚本还是模块、要不要解析类型）；`.wxs` 是
//! 独立方言、后缀不在其白名单里，这里显式按 CommonJS 脚本解析。
//!
//! 容错策略与 `input/wxss.rs` 一致：不 panic，解析失败返回 `Err`，由调用方决定是否
//! 跳过。差别在于这里不接受可恢复的语法错误：oxc 能带着错误产出一棵「大致像样」的
//! AST，但错位之后的节点形状不可信，规则在它上面报的告警多半是误报，宁可整份跳过。

use std::fmt;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    Argument, ArrayExpressionElement, ArrowFunctionBody, ArrowFunctionExpression,
    AssignmentExpression, AssignmentTarget, BindingPattern, CallExpression, ChainElement,
    Expression, Function, FunctionBody, IfStatement, ImportDeclaration, ImportDeclarationSpecifier,
    MethodDefinition, ObjectExpression, ObjectProperty, ObjectPropertyKind, PropertyDefinition,
    PropertyKey, Statement, StringLiteral, TemplateLiteral, VariableDeclaration,
    VariableDeclarationKind, VariableDeclarator,
};
use oxc_ast_visit::{Visit, walk};
use std::collections::{BTreeMap, BTreeSet};

use oxc_parser::{ParseOptions, Parser};
use oxc_span::{GetSpan, SourceType, Span};
use oxc_syntax::operator::{BinaryOperator, UnaryOperator};
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
    strings: Vec<StringValue>,
    this_data_captures: Vec<ThisDataCapture>,
    object_fns: Vec<ObjectFn>,
    /// 单元根对象（`Page({..})` / `Component({..})`）与 `methods` 的属性绑定。
    members: Vec<MemberBinding>,
    /// 模块引用（`require` / `import`）的本地名。
    requires: Vec<RequireBinding>,
    /// 运行时挂到 `this` 上的属性（`this.onTap = ...`）。
    this_binds: Vec<MemberBinding>,
    literals: BTreeMap<String, Lit>,
    /// 声明在 `properties` 里的属性，以及默认值能不能静态判空。
    property_defaults: BTreeMap<String, PropertyDefault>,
    dirty_data: BTreeSet<String>,
    dirty_all: bool,
}

/// `Component({ properties })` 里一个属性的默认值形态。
///
/// 只区分「空 / 非空常量 / 判不了」三档：规则问的是「不传这个属性时背景是不是一定为空」，
/// 默认值缺省、`''`、`null` 都算空；变量、调用等一律算判不了（宁可漏报）。
#[derive(Debug, Clone, PartialEq)]
pub enum PropertyDefault {
    /// `value` 缺省、`''` 或 `null`。
    Empty,
    /// 非空字面量（`'#fff'`、`false` 等）。
    Fixed(Lit),
    /// 取值依赖运行时（变量 / 调用 / 对象字面量）。
    Unknown,
}

impl JsDoc {
    pub fn parse(path: &str, source: &str) -> Result<Self, ParseError> {
        // `.wxs` 不在 `SourceType::VALID_EXTENSIONS` 里，但它就是脚本方言：
        // 按 CommonJS 脚本解析（`require` 与 `module.exports` 都能落位）。
        let source_type = if path.ends_with(".wxs") {
            SourceType::cjs()
        } else {
            SourceType::from_path(path)
                .map_err(|_| ParseError::Syntax(format!("不支持的文件后缀：{path}")))?
        };

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

        let mut collector = Collector {
            source: owned.clone(),
            // 顶层也是一个作用域：模块级 `const isSkyline = ...` 同样要能解析。
            renderer_aliases: vec![BTreeMap::new()],
            ..Default::default()
        };
        collector.visit_program(&ret.program);

        Ok(Self {
            path: path.to_string(),
            source: owned,
            functions: collector.functions,
            calls: collector.calls,
            strings: collector.strings,
            this_data_captures: collector.this_data_captures,
            object_fns: collector.object_fns,
            members: collector.members,
            requires: collector.requires,
            this_binds: collector.this_binds,
            literals: collector.literals,
            property_defaults: collector.property_defaults,
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

    /// 声明在 `properties` 里的属性 → 默认值形态。
    ///
    /// 被 `setData` / `this.data.x = ...` 写过的键一律降为 [`PropertyDefault::Unknown`]：
    /// 运行时会被改写，默认值不再代表实际取值。
    pub fn declared_properties(&self) -> BTreeMap<String, PropertyDefault> {
        self.property_defaults
            .iter()
            .map(|(name, default)| {
                let default = if self.dirty_all || self.dirty_data.contains(name) {
                    PropertyDefault::Unknown
                } else {
                    default.clone()
                };
                (name.clone(), default)
            })
            .collect()
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

    /// 所有字符串字面量与无插值的模板串，按遍历顺序。
    ///
    /// 值是 cooked 形态（转义已解开），位置是含引号 / 反引号的原文区间，`context`
    /// 是它挂的名字（对象属性键 / 变量名 / 赋值目标）：规则据此判断字符串内容，以及
    /// 它是不是某个 `…Style` 的值，不重新扫源码文本。
    pub fn strings(&self) -> &[StringValue] {
        &self.strings
    }

    /// 所有以 `this.data` 为初始化值的声明（整体捕获或解构）。
    ///
    /// 只暴露这一个形状，不做通用 AST 暴露：worklet 的数据冻结规则（#156）只需要
    /// 「捕获了 `this.data` 对象」与「在不在 worklet 里」，`calls()` 看不到声明。
    pub fn this_data_captures(&self) -> &[ThisDataCapture] {
        &self.this_data_captures
    }

    /// `Page({..})` / `Component({..})` 顶层与 `methods` 里的属性绑定，按出现顺序。
    ///
    /// WXML 的 `bind:tap="onTap"` 是按**属性名**找到处理函数的，所以这里记的是
    /// 「键 → 值指向哪里」，而不是「函数叫什么名字」。
    pub fn members(&self) -> &[MemberBinding] {
        &self.members
    }

    /// 模块引用的本地名：`const m = require('./x')`、`import m from './x'`、
    /// `import { a as b } from './x'`、`const { a } = require('./x')`。
    pub fn requires(&self) -> &[RequireBinding] {
        &self.requires
    }

    /// 运行时挂到 `this` 上的属性（`this.onTap = common.onTap.bind(this)`）。
    pub fn this_binds(&self) -> &[MemberBinding] {
        &self.this_binds
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
    /// 与 `WxssDoc::position` 同一套换算（[`super::position`]）。
    pub fn position(&self, offset: usize) -> (u32, u32) {
        super::position(&self.source, offset)
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

/// 一个字符串字面量（或无插值的模板串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StringValue {
    /// cooked 取值。
    pub value: String,
    /// 字面量挂的名字：对象属性键、变量名、赋值目标名。`None` 表示结构上不是
    /// 「某个名字的值」（调用实参、数组元素、`return` 的值等）。
    ///
    /// 规则用 `…Style$` 判定它是不是内联样式：`style` / `wxLoginStyle` /
    /// `loginActiveStyle` 这类名字的值才是绑到 `style` 上的候选。
    pub context: Option<String>,
    /// 含引号的字节区间。
    pub start: u32,
    pub end: u32,
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
    /// 调用点是否落在「只在 WebView 走的分支」里：外层 `if` 的条件直接比较
    /// `renderer`（`=== 'webview'` 的 then 分支、`=== 'skyline'` 的 else 分支）。
    /// 这类调用在 Skyline 下根本不执行，按不支持 API 报就是误报。
    pub webview_only: bool,
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

/// 一次以 `this.data` 为初始化值的声明：`const { a } = this.data`、`const snapshot = this.data`。
///
/// 整体捕获与解构在运行时是同一条路径——`this.data` 对象成为 worklet 的外部引用，
/// 因此不区分绑定形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThisDataCapture {
    /// 整个声明器（`const snapshot = this.data`）的字节区间。
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

/// 属性值指向哪里。只保留静态可判的四种形态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueShape {
    /// 内联函数字面量 / 方法简写。
    Function,
    /// 裸标识符（同文件声明的函数、或 `require` / `import` 的本地名）。
    Ident(String),
    /// `module.fn` 形式的成员访问。
    Member { object: String, property: String },
    /// 其它写法（调用结果、条件表达式……）静态判不了。
    Other,
}

/// 单元方法表上的一个属性：键 → 值指向。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberBinding {
    pub key: String,
    pub value: ValueShape,
    /// 属性在源码里的范围（键到值），用于把诊断落在声明处。
    pub start: u32,
    pub end: u32,
}

/// 一次模块引用：本地名指向哪个文件的什么导出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequireBinding {
    /// 本地名（`const m = require('./x')` 里的 `m`）。
    pub local: String,
    /// 原始说明符（`./x`、`@/utils/y`）。
    pub specifier: String,
    /// 具名引用时的导出名（`import { noop } from './x'`）；命名空间引用为 `None`。
    pub imported: Option<String>,
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

/// 一条 `if` 语句的分支区间：命中区间时按「只在 WebView」处理。
#[derive(Debug, Clone, Copy)]
struct RendererGuard {
    cons: (u32, u32),
    alt: Option<(u32, u32)>,
    /// 条件直接判 `'webview'` 时 then 分支只在 WebView 走。
    cons_webview: bool,
    /// 条件判 `'skyline'` 时 else 分支只在 WebView 走。
    alt_webview: bool,
}

/// `if` 条件里两个分支各属哪个渲染器：`(then 是否只属于 WebView, else 是否只属于 WebView)`。
///
/// 认 `renderer` 与 `'webview'` / `'skyline'` 的字面量比较，并按运算符方向分：
/// `=== 'webview'` 的 then 是 WebView 分支，`!== 'skyline'` 的 then 同样是 WebView 分支。
/// 条件里同时比对两个取值（`a === 'skyline' || b === 'webview'`）时无法简单归属，按不加守卫处理。
fn renderer_branches(test: &str) -> (bool, bool) {
    let test = test.to_ascii_lowercase();
    // 取值前一个运算符是不是 `!=`：是则 then 分支不属这个取值。
    let negated = |value: &str| -> Option<bool> {
        let index = test
            .find(&format!("'{value}'"))
            .or_else(|| test.find(&format!("\"{value}\"")))?;
        let prefix = test[..index].trim_end();
        Some(prefix.ends_with("!==") || prefix.ends_with("!="))
    };
    match (negated("webview"), negated("skyline")) {
        (Some(negated), None) => (!negated, negated),
        (None, Some(negated)) => (negated, !negated),
        _ => (false, false),
    }
}

/// `X.renderer === 'webview'` 这类单个比较的两个分支归属，供常量别名使用。
///
/// 与 [`renderer_branches`] 同一套归属规则，但要求表达式本身就是一次比较、
/// 两侧至少一侧是 `renderer` 成员访问：`a === 'skyline' && b === 'webview'` 这种
/// 组合表达式不归属，宁可不加守卫。
fn renderer_comparison_branches(expr: &Expression) -> Option<(bool, bool)> {
    let Expression::BinaryExpression(binary) = expr else {
        return None;
    };
    let negated = match binary.operator {
        BinaryOperator::Equality | BinaryOperator::StrictEquality => false,
        BinaryOperator::Inequality | BinaryOperator::StrictInequality => true,
        _ => return None,
    };
    let (literal, reference) = match (&binary.left, &binary.right) {
        (Expression::StringLiteral(literal), reference) => (literal, reference),
        (reference, Expression::StringLiteral(literal)) => (literal, reference),
        _ => return None,
    };
    match literal.value.as_str()?.to_ascii_lowercase().as_str() {
        value @ ("webview" | "skyline") if is_renderer_reference(reference) => {
            let cons_webview = (value == "webview") != negated;
            Some((cons_webview, !cons_webview))
        }
        _ => None,
    }
}

/// `renderer` / `this.renderer` / `info.renderer`：比较的另一侧要是取渲染器。
fn is_renderer_reference(expr: &Expression) -> bool {
    match expr {
        Expression::StaticMemberExpression(member) => member.property.name == "renderer",
        Expression::Identifier(id) => id.name == "renderer",
        _ => false,
    }
}

#[derive(Default)]
struct Collector {
    functions: Vec<OwnedFunction>,
    calls: Vec<CallInfo>,
    strings: Vec<StringValue>,
    this_data_captures: Vec<ThisDataCapture>,
    object_fns: Vec<ObjectFn>,
    members: Vec<MemberBinding>,
    requires: Vec<RequireBinding>,
    this_binds: Vec<MemberBinding>,
    /// `Page({data})` / `Component({data, properties})` 里的字面量取值。
    literals: BTreeMap<String, Lit>,
    /// 声明在 `properties` 里的属性 → 默认值形态。
    property_defaults: BTreeMap<String, PropertyDefault>,
    /// 已经带上下文收录的字面量起点：父节点先收，通用字符串遍历后到时跳过。
    claimed_strings: BTreeSet<u32>,
    /// 被动态写入过的 data 键：`setData({k: 变量})` 或 `setData` 里出现动态键。
    dirty_data: BTreeSet<String>,
    /// 整份文件的 data 都被动态写入（`setData(变量)` / 展开语法），所有键都不再确定。
    dirty_all: bool,
    /// 下一个函数表达式 / 箭头函数该沿用的名字（变量名或方法名）。
    pending: Option<String>,
    /// 外层函数栈，值为该函数是否 `has_worklet`。取最近的一层。
    worklet_stack: Vec<bool>,
    /// 已经过的 `if` 语句分支区间与「该分支是否只属于 WebView」。
    renderer_guards: Vec<RendererGuard>,
    /// 每个函数作用域里的渲染器常量：
    /// `const isSkyline = this.renderer === 'skyline'` 的名字 →
    /// `(条件成立时是否只在 WebView, 条件不成立时是否只在 WebView)`。
    /// 函数进出时压栈/出栈，跨作用域的同名常量互不影响。
    renderer_aliases: Vec<BTreeMap<String, (bool, bool)>>,
    /// 原始源码：守卫判定要读 `if` 条件的原文。
    source: String,
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
        self.renderer_aliases.push(BTreeMap::new());
    }

    fn worklet_scope(&self) -> bool {
        self.worklet_stack.last().copied().unwrap_or(false)
    }

    /// 收录直接作为某名字取值的字符串，并记下它挂的名字。
    ///
    /// 只在 `expr` 本身就是字符串（无插值模板串）时收录：调用实参、数组元素里的
    /// 字符串不继承外层名字，否则会把 `style: format('…')` 里的参数当成样式值。
    fn push_context_string(&mut self, expr: &Expression<'_>, context: Option<String>) {
        let (value, span) = match expr {
            Expression::StringLiteral(literal) => match literal.value.as_str() {
                Some(value) => (value.to_string(), literal.span),
                None => return,
            },
            Expression::TemplateLiteral(template) => match template_string(template) {
                Some(value) => (value, template.span),
                None => return,
            },
            _ => return,
        };
        self.claimed_strings.insert(span.start);
        self.strings.push(StringValue {
            value,
            context,
            start: span.start,
            end: span.end,
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
        let body = it.body.as_deref();
        // 无函数体（TS `declare function` / overload）不是有效作用域，占位到函数末尾。
        let body_span = body.map_or(it.span, |b| b.span);
        self.push_function(name, it.span, body, body_span);
        walk::walk_function(self, it, flags);
        self.worklet_stack.pop();
        self.renderer_aliases.pop();
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
        self.renderer_aliases.pop();
    }

    fn visit_string_literal(&mut self, it: &StringLiteral<'a>) {
        if self.claimed_strings.contains(&it.span.start) {
            return;
        }
        if let Some(value) = it.value.as_str() {
            self.strings.push(StringValue {
                value: value.to_string(),
                context: None,
                start: it.span.start,
                end: it.span.end,
            });
        }
    }

    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        // 含插值的模板串拼不出来，静态判不了；不含插值的就是一个普通字符串。
        if !self.claimed_strings.contains(&it.span.start)
            && let Some(value) = template_string(it)
        {
            self.strings.push(StringValue {
                value,
                context: None,
                start: it.span.start,
                end: it.span.end,
            });
        }
        walk::walk_template_literal(self, it);
    }

    fn visit_variable_declaration(&mut self, it: &VariableDeclaration<'a>) {
        // 只看 `const`：`let` / `var` 可能被重新赋值，不能当稳定常量。
        if it.kind == VariableDeclarationKind::Const {
            for declarator in &it.declarations {
                let (Some(name), Some(init)) = (binding_name(&declarator.id), &declarator.init)
                else {
                    continue;
                };
                if let Some(branches) = renderer_comparison_branches(init)
                    && let Some(scope) = self.renderer_aliases.last_mut()
                {
                    scope.insert(name, branches);
                }
            }
        }
        walk::walk_variable_declaration(self, it);
    }

    fn visit_variable_declarator(&mut self, it: &VariableDeclarator<'a>) {
        // 模块引用的本地名：`const m = require('./x')` / `const { a } = require('./x')`。
        if let Some(init) = &it.init
            && let Some(specifier) = require_specifier(init)
        {
            match &it.id {
                BindingPattern::BindingIdentifier(id) => self.requires.push(RequireBinding {
                    local: id.name.as_str().to_string(),
                    specifier,
                    imported: None,
                }),
                BindingPattern::ObjectPattern(pattern) => {
                    for property in &pattern.properties {
                        let (Some(local), Some(imported)) =
                            (binding_name(&property.value), key_name(&property.key))
                        else {
                            continue;
                        };
                        self.requires.push(RequireBinding {
                            local,
                            specifier: specifier.clone(),
                            imported: Some(imported),
                        });
                    }
                }
                _ => {}
            }
        }
        // 只认「初始化值本身就是函数」，以及链式 API 的身份包装
        // `const f = method(() => {})` / `listener(...)`：这两个包装原样返回入参
        // （`miniprogram-chaining-api-polyfill` 只打 tag），运行时仍是那个函数，
        // 所以 `'worklet'` 指令照样生效、`bind:` 绑的也是它。
        // 其余 `wrap(fn)` 一律不认，避免把包装函数当成 worklet。
        if it.init.as_ref().is_some_and(is_function)
            || it
                .init
                .as_ref()
                .is_some_and(|init| is_identity_wrapper(init))
        {
            self.pending = binding_name(&it.id);
        }
        // 右侧是字面量 `this.data` 就捕获了整个 data 对象，与左侧是不是解构无关。
        if it.init.as_ref().is_some_and(is_this_data) {
            self.this_data_captures.push(ThisDataCapture {
                start: it.span.start,
                end: it.span.end,
                worklet_scope: self.worklet_scope(),
            });
        }
        if let Some(init) = &it.init {
            self.push_context_string(init, binding_name(&it.id));
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
        let context = (!it.computed).then(|| key_name(&it.key)).flatten();
        self.push_context_string(&it.value, context);
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

    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        let Some(specifier) = it.source.value.as_str() else {
            return;
        };
        let specifier = specifier.to_string();
        let Some(specifiers) = &it.specifiers else {
            return;
        };
        for item in specifiers.iter() {
            let (local, imported) = match item {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => (
                    specifier.local.name.as_str().to_string(),
                    Some(specifier.imported.name().as_str().to_string()),
                ),
                ImportDeclarationSpecifier::ImportDefaultSpecifier(specifier) => {
                    (specifier.local.name.as_str().to_string(), None)
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(specifier) => {
                    (specifier.local.name.as_str().to_string(), None)
                }
            };
            self.requires.push(RequireBinding {
                local,
                specifier: specifier.clone(),
                imported,
            });
        }
        walk::walk_import_declaration(self, it);
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
            // `this.onTap = ...`：运行时挂上去的处理函数。
            AssignmentTarget::StaticMemberExpression(member)
                if matches!(&member.object, Expression::ThisExpression(_))
                    && member.property.name != "data" =>
            {
                self.this_binds.push(MemberBinding {
                    key: member.property.name.to_string(),
                    value: value_shape(&it.right),
                    start: it.span.start,
                    end: it.span.end,
                });
            }
            _ => {}
        }
        self.push_context_string(&it.right, target_name(&it.left));
        walk::walk_assignment_expression(self, it);
    }

    fn visit_if_statement(&mut self, it: &IfStatement<'a>) {
        // 优先按本地常量别名解析：`const isSkyline = this.renderer === 'skyline'`
        // 之后的 `if (isSkyline)` / `if (!isSkyline)`。解析不出来再读条件原文。
        let (cons_webview, alt_webview) = self.alias_branches(&it.test).unwrap_or_else(|| {
            let span = it.test.span();
            renderer_branches(&self.source[span.start as usize..span.end as usize])
        });
        let cons = it.consequent.span();
        self.renderer_guards.push(RendererGuard {
            cons: (cons.start, cons.end),
            alt: it.alternate.as_ref().map(|alt| {
                let span = alt.span();
                (span.start, span.end)
            }),
            cons_webview,
            alt_webview,
        });
        walk::walk_if_statement(self, it);
        self.renderer_guards.pop();
    }

    fn visit_call_expression(&mut self, it: &CallExpression<'a>) {
        let callee = callee_name(&it.callee).unwrap_or_default();
        // 单元方法表三种写法：`Page/Component({ .. })`、链式 `.methods({ .. })`、
        // 链式 `.init(({..}) => { .. return { .. } })`。
        let tail = callee.trim_start_matches('.');
        if matches!(callee.as_str(), "Page" | "Component") {
            if let Some(Expression::ObjectExpression(object)) =
                it.arguments.first().and_then(arg_expression)
            {
                self.collect_properties_of(object);
                self.collect_members(object);
            }
        } else if matches!(tail, "methods" | "method") {
            if let Some(Expression::ObjectExpression(object)) =
                it.arguments.first().and_then(arg_expression)
            {
                self.collect_members(object);
            }
        } else if tail == "init"
            && let Some(object) = it
                .arguments
                .first()
                .and_then(arg_expression)
                .and_then(returned_object)
        {
            self.collect_members(object);
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
        let start = it.span.start;
        let end = it.span.end;
        self.calls.push(CallInfo {
            callee: callee_name(&it.callee).unwrap_or_default(),
            start,
            end,
            args: it.arguments.iter().map(arg_info).collect(),
            worklet_scope: self.worklet_scope(),
            webview_only: self.renderer_guarded(start),
        });
        walk::walk_call_expression(self, it);
    }
}

impl Collector {
    /// 测试表达式是本地常量别名时的分支归属：`isSkyline` / `!isSkyline`。
    ///
    /// 只查当前作用域，不向父作用域找：常量定义在哪个函数里就只在哪个函数里有效，
    /// 跨函数的同名绑定不能串味。
    fn alias_branches(&self, test: &Expression) -> Option<(bool, bool)> {
        let (name, negated) = match test {
            Expression::Identifier(id) => (id.name.as_str(), false),
            Expression::UnaryExpression(unary) if unary.operator == UnaryOperator::LogicalNot => {
                let Expression::Identifier(id) = &unary.argument else {
                    return None;
                };
                (id.name.as_str(), true)
            }
            _ => return None,
        };
        let &(cons, alt) = self.renderer_aliases.last()?.get(name)?;
        Some(if negated { (alt, cons) } else { (cons, alt) })
    }

    /// 调用点是否落在「只在 WebView 执行」的分支里。
    ///
    /// 条件认两种写法：直接比较 `renderer` 的原文，以及指向这种比较的本地
    /// `const` 别名（`if (isSkyline)` / `if (!isSkyline)`）。两者都解不出来时
    /// 不加守卫，保持「宁可漏报不要误报」。
    fn renderer_guarded(&self, offset: u32) -> bool {
        self.renderer_guards.iter().any(|guard| {
            let in_cons = offset >= guard.cons.0 && offset < guard.cons.1;
            if in_cons {
                return guard.cons_webview;
            }
            match guard.alt {
                Some(alt) if offset >= alt.0 && offset < alt.1 => guard.alt_webview,
                _ => false,
            }
        })
    }

    /// `Component({..})` 顶层与 `methods` 里的属性：键 → 值指向。
    ///
    /// 只从单元方法表收，不从任意对象字面量里造数据：同一文件里 `{ onTap: ... }`
    /// 这样的局部对象跟事件绑定无关。
    fn collect_members<'b>(&mut self, object: &ObjectExpression<'b>) {
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
            if key == "methods" {
                if let Expression::ObjectExpression(methods) = &property.value {
                    self.collect_members(methods);
                }
                continue;
            }
            self.members.push(MemberBinding {
                key,
                value: value_shape(&property.value),
                start: property.span.start,
                end: property.span.end,
            });
        }
    }

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
                // `properties: { background: String }`：只写了类型，没有默认值。
                if matches!(&property.value, Expression::Identifier(id)
                    if matches!(id.name.as_str(), "String" | "Number" | "Boolean" | "Array" | "Object"))
                {
                    self.property_defaults.insert(key, PropertyDefault::Empty);
                }
                continue;
            };
            let mut value = None;
            for field in &definition.properties {
                let ObjectPropertyKind::ObjectProperty(field) = field else {
                    continue;
                };
                if field.computed || key_name(&field.key).as_deref() != Some("value") {
                    continue;
                }
                value = Some(&field.value);
                if let Some(literal) = literal(&field.value) {
                    self.literals.insert(key.clone(), literal);
                }
            }
            let default = match value {
                // 没写 `value` 就取类型默认值（字符串为空）。
                None => PropertyDefault::Empty,
                Some(Expression::NullLiteral(_)) => PropertyDefault::Empty,
                Some(field) => match literal(field) {
                    Some(Lit::Str(text)) if text.is_empty() => PropertyDefault::Empty,
                    Some(literal) => PropertyDefault::Fixed(literal),
                    None => PropertyDefault::Unknown,
                },
            };
            self.property_defaults.insert(key, default);
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

/// 字面量：字符串 / 数字 / 布尔 / 无 spread 的数组（只记长度）。其余（对象、变量、调用）都算不可知。
fn literal(expression: &Expression) -> Option<Lit> {
    match expression {
        Expression::StringLiteral(value) => Some(Lit::Str(
            value.value.as_str().unwrap_or_default().to_string(),
        )),
        Expression::NumericLiteral(value) => Some(Lit::Num(value.value)),
        Expression::BooleanLiteral(value) => Some(Lit::Bool(value.value)),
        // `{{buttons && buttons.length}}` 这类条件靠数组默认值折叠；带 spread 时长度不可知。
        Expression::ArrayExpression(array) => {
            let spread = array
                .elements
                .iter()
                .any(|element| matches!(element, ArrayExpressionElement::SpreadElement(_)));
            (!spread).then_some(Lit::Array(array.elements.len()))
        }
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

/// 不含插值的模板串的 cooked 取值。
fn template_string(template: &TemplateLiteral<'_>) -> Option<String> {
    if !template.expressions.is_empty() {
        return None;
    }
    let quasi = template.quasis.first()?;
    let value = match &quasi.value.cooked {
        Some(cooked) => cooked.as_str(),
        None => Some(quasi.value.raw.as_str()),
    };
    Some(value?.to_string())
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

/// 链式 API 的身份包装：`method(fn)` / `listener(fn)`，实参必须是函数字面量。
fn is_identity_wrapper(expr: &Expression) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return false;
    };
    let name = callee.name.as_str();
    (name == "method" || name == "listener")
        && matches!(call.arguments.len(), 1)
        && call
            .arguments
            .first()
            .is_some_and(|arg| arg.as_expression().is_some_and(is_function))
}

/// 函数体里最后一个 `return { .. }` 的对象字面量。
///
/// 链式 API 的 `.init(({..}) => { ... return { 方法定义 } })` 把方法表放在返回值里。
fn returned_object<'a, 'b>(expr: &'b Expression<'a>) -> Option<&'b ObjectExpression<'a>> {
    let body = match expr {
        Expression::ArrowFunctionExpression(arrow) => match &arrow.body {
            ArrowFunctionBody::FunctionBody(body) => Some(body.as_ref()),
            _ => None,
        },
        Expression::FunctionExpression(function) => function.body.as_deref(),
        _ => None,
    }?;
    body.statements.iter().rev().find_map(|statement| {
        let Statement::ReturnStatement(ret) = statement else {
            return None;
        };
        match &ret.argument {
            Some(Expression::ObjectExpression(object)) => Some(object.as_ref()),
            _ => None,
        }
    })
}

/// `require('./x')` 的说明符。
fn require_specifier(expr: &Expression) -> Option<String> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    let Expression::Identifier(callee) = &call.callee else {
        return None;
    };
    if callee.name != "require" {
        return None;
    }
    let Expression::StringLiteral(literal) = call.arguments.first()?.as_expression()? else {
        return None;
    };
    literal.value.as_str().map(str::to_string)
}

/// 属性值指向哪里：函数字面量、裸标识符、`m.fn`（含 `.bind(this)`）、其它。
fn value_shape(expr: &Expression) -> ValueShape {
    let target = |expr: &Expression| -> Option<(String, String)> {
        let Expression::StaticMemberExpression(member) = expr else {
            return None;
        };
        let Expression::Identifier(object) = &member.object else {
            return None;
        };
        Some((
            object.name.as_str().to_string(),
            member.property.name.as_str().to_string(),
        ))
    };
    match expr {
        _ if is_function(expr) => ValueShape::Function,
        Expression::Identifier(id) => ValueShape::Ident(id.name.as_str().to_string()),
        // `m.fn.bind(this)`：`.bind` 只换 this，目标函数没变。
        Expression::CallExpression(call) => {
            let Expression::StaticMemberExpression(member) = &call.callee else {
                return ValueShape::Other;
            };
            if !matches!(member.property.name.as_str(), "bind" | "call") {
                return ValueShape::Other;
            }
            target(&member.object)
                .map(|(object, property)| ValueShape::Member { object, property })
                .unwrap_or(ValueShape::Other)
        }
        _ => target(expr)
            .map(|(object, property)| ValueShape::Member { object, property })
            .unwrap_or(ValueShape::Other),
    }
}

fn binding_name(pattern: &BindingPattern) -> Option<String> {
    match pattern {
        BindingPattern::BindingIdentifier(id) => Some(id.name.as_str().to_string()),
        _ => None,
    }
}

/// 赋值目标的名字：`x` / `this.x` / `a.b.x` 取最后一段；计算属性取不到。
fn target_name(target: &AssignmentTarget) -> Option<String> {
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(id) => Some(id.name.as_str().to_string()),
        AssignmentTarget::StaticMemberExpression(member) => {
            Some(member.property.name.as_str().to_string())
        }
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
            JsDoc::parse("a.coffee", "var a = 1;"),
            Err(ParseError::Syntax(_))
        ));
    }

    /// `.wxs` 不在 oxc 的后缀白名单里，但要按脚本解析：规则需要看 wxs 里的调用。
    #[test]
    fn wxs_is_parsed_as_a_script() {
        let src = "function f(owner) { return owner.getBoundingClientRect().width }\nmodule.exports = { f: f }";
        let doc = JsDoc::parse("a.wxs", src).expect("WXS 应当解析成功");
        assert_eq!(doc.functions().len(), 1);
        assert_eq!(doc.calls()[0].callee, "owner.getBoundingClientRect");
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
    fn this_data_captures_are_collected_with_scope() {
        let src = r#"
            const { a } = this.data;
            function normal() { let { b } = this.data; const c = this.data.c; const snapshot = this.data; const { d } = this.props; }
            function animated() { 'worklet'; const { e } = this.data; }
            function outer() { 'worklet'; const inner = () => { const { f } = this.data; }; }
            const { g } = other.data;
            const [h] = this.data;
            const { i: { j } } = this.data;
        "#;
        let doc = parse_js(src);
        let found: Vec<(u32, bool)> = doc
            .this_data_captures()
            .iter()
            .map(|d| (d.start, d.worklet_scope))
            .collect();
        // 整体捕获、嵌套解构、数组解构都算；属性访问（`this.data.c`）不算。
        // 顶层、普通函数内、worklet 内、worklet 内嵌普通函数：作用域取最近一层。
        assert_eq!(found.len(), 7, "{found:?}");
        assert_eq!(
            found.iter().map(|(_, scope)| *scope).collect::<Vec<_>>(),
            [false, false, false, true, false, false, false]
        );
    }
}

#[cfg(test)]
mod renderer_guard_tests {
    use super::renderer_branches;

    #[test]
    fn branches_follow_the_operator() {
        assert_eq!(
            renderer_branches("this.renderer === 'webview'"),
            (true, false),
            "eq webview"
        );
        assert_eq!(
            renderer_branches("this.renderer !== 'webview'"),
            (false, true),
            "neq webview"
        );
        assert_eq!(
            renderer_branches("this.renderer === 'skyline'"),
            (false, true),
            "eq skyline"
        );
        assert_eq!(
            renderer_branches("getApp().renderer !== \"skyline\""),
            (true, false)
        );
        assert_eq!(renderer_branches("a === 1"), (false, false));
    }
}
