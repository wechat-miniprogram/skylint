//! TSX/JSX → 渲染树。
//!
//! Taro 默认模式下，页面 TSX 里 `import` 进来的自定义组件不会变成小程序自定义
//! 组件，而是由 React 渲染器把它的输出**内联**进页面节点树；只有写在
//! `usingComponents` 里的组件才是真正的组件边界。所以：
//! 1. 内置组件 → 标签；
//! 2. `usingComponents` 里声明的组件 → 组件标签（结构在它自己的 wxml 里）；
//! 3. 其余本地组件 → 解析默认导出函数，把返回的 JSX 内联进来（含 children 代入）。
//!
//! 静态可证范围：字面量、三元 / `&&` / `||` 的每个分支、`.map` 回调体。
//! 其余一律动态（`{{_}}`），与原生 WXML 的插值形态一致，规则层按 Unknown 处理。

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::*;
use oxc_ast_visit::{Visit, walk};
use oxc_parser::{Parser, ParserReturn};
use oxc_span::{GetSpan, SourceType, Span};

use crate::table;

/// 参与合成的源文件。`file` 就是它在 `sources` 里的下标。
#[derive(Debug, Clone)]
pub struct SourceFile {
    /// 绝对路径；`synth` 再换成相对项目根的写法。
    pub path: String,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub tag: String,
    /// 定义这段结构的源文件下标。
    pub file: u32,
    /// 源码里 `<` 的字节偏移。
    pub span: u32,
    pub attrs: Vec<Attr>,
    pub children: Vec<Child>,
}

#[derive(Debug, Clone)]
pub struct Attr {
    pub name: String,
    pub file: u32,
    /// 属性名在源码里的字节偏移。
    pub span: u32,
    pub value: AttrValue,
}

#[derive(Debug, Clone)]
pub enum AttrValue {
    Static(String),
    Bool,
    Dynamic,
}

#[derive(Debug, Clone)]
pub enum Child {
    Element(Element),
    Text { text: String, file: u32, span: u32 },
    Dynamic { file: u32, span: u32 },
    /// `{props.children}` / `{children}`：内联时替换为调用点的子节点。
    Slot,
}

pub struct Parsed {
    pub roots: Vec<Element>,
    pub notices: Vec<String>,
    /// 页面自己 + 被内联进来的组件文件。
    pub sources: Vec<SourceFile>,
}

/// 解析一个页面/组件的源文件。
pub fn parse_page(
    src_root: &Path,
    path: &Path,
    source: &str,
    declared: &HashSet<String>,
) -> Result<Parsed, String> {
    let mut loader = Loader {
        src_root: src_root.to_path_buf(),
        declared: declared.clone(),
        cache: HashMap::new(),
        sources: Vec::new(),
        ids: HashMap::new(),
        stack: Vec::new(),
        notices: Vec::new(),
    };
    let roots = loader.load(path, source)?;
    Ok(Parsed {
        roots,
        notices: loader.notices,
        sources: loader.sources,
    })
}

/// 本地名 → (模块路径, 导入名；默认导入为 "default")。
type Imports = HashMap<String, (String, String)>;

/// 一份源文件的 JSX 索引：元素按 span 取回、文件级顶层根、文件内局部组件。
struct Indexed<'a> {
    elements: HashMap<Span, &'a JSXElement<'a>>,
    roots: Vec<Span>,
    locals: HashMap<String, Vec<Span>>,
}

fn index_program<'a>(program: &'a Program<'a>, scope: Option<Span>) -> Indexed<'a> {
    struct Collector<'a> {
        elements: HashMap<Span, &'a JSXElement<'a>>,
        spans: Vec<Span>,
    }
    impl<'a> Visit<'a> for Collector<'a> {
        fn visit_jsx_element(&mut self, it: &JSXElement<'a>) {
            // `alloc` 把引用提升到 arena 的生命周期，转换阶段才能按 span 取回。
            let el: &'a JSXElement<'a> = self.alloc(it);
            self.elements.insert(el.span, el);
            self.spans.push(el.span);
            walk::walk_jsx_element(self, el);
        }
    }

    let mut collector = Collector {
        elements: HashMap::new(),
        spans: Vec::new(),
    };
    collector.visit_program(program);

    let roots = roots_within(&collector.spans, scope);
    let locals = locals_of(program, &collector.spans);
    Indexed {
        elements: collector.elements,
        roots,
        locals,
    }
}

/// 只有 `export ... from` 形式——本文件定义的导出已经在 `locals` 里。
fn reexports_of(program: &Program<'_>) -> (HashMap<String, (String, String)>, Vec<String>) {
    let mut reexports = HashMap::new();
    let mut stars = Vec::new();
    for statement in &program.body {
        match statement {
            Statement::ExportFromDeclaration(decl) => {
                let spec = decl.source.value.as_str().unwrap_or("").to_string();
                for item in &decl.specifiers {
                    reexports.insert(
                        item.exported.name().as_str().to_string(),
                        (spec.clone(), item.local.name().as_str().to_string()),
                    );
                }
            }
            Statement::ExportAllDeclaration(decl) if decl.exported.is_none() => {
                stars.push(decl.source.value.as_str().unwrap_or("").to_string());
            }
            Statement::ExportAllDeclaration(_) => {}
            _ => {}
        }
    }
    (reexports, stars)
}

/// `scope` 内、不被其它元素包含的 span。条件分支里的每个元素各自成为一棵树。
fn roots_within(all: &[Span], scope: Option<Span>) -> Vec<Span> {
    let inside: Vec<Span> = all
        .iter()
        .copied()
        .filter(|span| scope.is_none_or(|scope| span.start >= scope.start && span.end <= scope.end))
        .collect();
    inside
        .iter()
        .copied()
        .filter(|span| {
            !inside
                .iter()
                .any(|outer| spans_strictly_contains(*outer, *span))
        })
        .collect()
}

/// 文件内定义的组件（`const Card = () => ...` / `function Card() {}`）。
fn locals_of(program: &Program<'_>, all: &[Span]) -> HashMap<String, Vec<Span>> {
    let mut locals = HashMap::new();
    for statement in &program.body {
        match statement {
            Statement::FunctionDeclaration(function) => {
                add_function(&mut locals, function, function.span, all);
            }
            Statement::VariableDeclaration(decl) => add_variables(&mut locals, decl, all),
            Statement::ExportDeclaration(decl) => match &decl.declaration {
                Declaration::FunctionDeclaration(function) => {
                    add_function(&mut locals, function, function.span, all);
                }
                Declaration::VariableDeclaration(decl) => add_variables(&mut locals, decl, all),
                _ => {}
            },
            _ => {}
        }
    }
    locals
}

fn add_function(
    locals: &mut HashMap<String, Vec<Span>>,
    function: &Function<'_>,
    span: Span,
    all: &[Span],
) {
    if let Some(id) = &function.id {
        locals.insert(id.name.as_str().to_string(), roots_within(all, Some(span)));
    }
}

fn add_variables(
    locals: &mut HashMap<String, Vec<Span>>,
    decl: &VariableDeclaration<'_>,
    all: &[Span],
) {
    for declarator in &decl.declarations {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            continue;
        };
        let Some(init) = &declarator.init else {
            continue;
        };
        if let Some(span) = function_span(init) {
            locals.insert(id.name.as_str().to_string(), roots_within(all, Some(span)));
        }
    }
}

/// 函数本身，或 `memo(...)` / `forwardRef(...)` / `observer(...)` 这类包装里的函数。
fn function_span(expr: &Expression<'_>) -> Option<Span> {
    match expr {
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            Some(expr.span())
        }
        Expression::ParenthesizedExpression(paren) => function_span(&paren.expression),
        Expression::CallExpression(call) => call
            .arguments
            .iter()
            .filter_map(|arg| arg.as_expression())
            .find_map(function_span),
        _ => None,
    }
}

struct Loader {
    src_root: PathBuf,
    /// 页面 / app 级 `usingComponents` 的标签名（小写）。
    declared: HashSet<String>,
    cache: HashMap<PathBuf, Vec<Element>>,
    /// 参与合成的源文件，下标即 `Element::file`。
    sources: Vec<SourceFile>,
    ids: HashMap<PathBuf, u32>,
    stack: Vec<PathBuf>,
    notices: Vec<String>,
}

impl Loader {
    fn source_id(&mut self, path: &Path, text: &str) -> u32 {
        if let Some(id) = self.ids.get(path) {
            return *id;
        }
        let id = self.sources.len() as u32;
        self.sources.push(SourceFile {
            path: path.to_string_lossy().to_string(),
            text: text.to_string(),
        });
        self.ids.insert(path.to_path_buf(), id);
        id
    }
}

impl Loader {
    fn load(&mut self, path: &Path, source: &str) -> Result<Vec<Element>, String> {
        let source_type = SourceType::from_path(path)
            .map_err(|_| format!("不支持的文件后缀：{}", path.display()))?
            // Taro 允许 `.js` 里直接写 JSX（Babel 转换）。
            .with_jsx(true);
        let owned = source.to_string();
        let allocator = Allocator::default();
        let ret = Parser::new(&allocator, &owned, source_type).parse();
        if let Some(error) = ret.diagnostics.errors().next() {
            return Err(error.to_string());
        }

        let imports = collect_imports(&ret.program);
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let index = index_program(&ret.program, exported_function_span(&ret.program));
        let file = self.source_id(path, &owned);

        let mut out = Vec::new();
        self.stack.push(path.to_path_buf());
        {
            let mut converter = Converter {
                loader: self,
                dir,
                imports,
                source: &owned,
                file,
                file_path: path.to_path_buf(),
                index: &index,
                locals: Vec::new(),
            };
            for span in &index.roots {
                if let Some(el) = index.elements.get(span).copied() {
                    converter.convert_element(el, &mut out);
                }
            }
        }
        self.stack.pop();
        Ok(out)
    }

    /// 把 `<Name ...>` 展开成渲染树；`None` 表示无法静态还原。
    fn inline(
        &mut self,
        name: &str,
        dir: &Path,
        imports: &Imports,
        children: Vec<Child>,
    ) -> Option<Vec<Element>> {
        // 组件互相引用的极端情况下别把栈递归穿。
        if self.stack.len() > 32 {
            return None;
        }
        let (spec, imported) = imports.get(name)?;
        let path = resolve_specifier(&self.src_root, dir, spec)?;
        let roots = self.expand(&path, imported, 0)?;
        Some(
            roots
                .into_iter()
                .map(|root| substitute(root, &children))
                .collect(),
        )
    }

    /// 顺着 barrel（`export { X } from './x'` / `export * from './x'`）找到组件。
    fn expand(&mut self, path: &Path, imported: &str, depth: usize) -> Option<Vec<Element>> {
        if depth > 8 {
            return None;
        }
        if imported == "default" {
            let roots = match self.cache.get(path) {
                Some(cached) => cached.clone(),
                None => {
                    let roots = self.load(path, &std::fs::read_to_string(path).ok()?).ok()?;
                    self.cache.insert(path.to_path_buf(), roots.clone());
                    roots
                }
            };
            return (!roots.is_empty()).then_some(roots);
        }
        // 同文件具名导出：`export function Item` / `export const Item`。
        if let Some(roots) = self.load_named(path, imported) {
            return Some(roots);
        }
        let (names, stars) = self.exports_of(path)?;
        if let Some((spec, name)) = names.get(imported) {
            let next = resolve_specifier(&self.src_root, path.parent()?, spec)?;
            return self.expand(&next, name, depth + 1);
        }
        for spec in stars {
            let Some(next) = resolve_specifier(&self.src_root, path.parent()?, &spec) else {
                continue;
            };
            if let Some(roots) = self.expand(&next, imported, depth + 1) {
                return Some(roots);
            }
        }
        None
    }

    /// 只转换文件里某个具名导出（函数 / 箭头函数 / `memo(...)` 包装）。
    fn load_named(&mut self, path: &Path, name: &str) -> Option<Vec<Element>> {
        if self.stack.iter().any(|current| current == path) {
            return None;
        }
        let source = std::fs::read_to_string(path).ok()?;
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(path).ok()?.with_jsx(true);
        let ret = Parser::new(&allocator, &source, source_type).parse();
        if !parses_clean(&ret) {
            return None;
        }
        let index = index_program(&ret.program, None);
        let spans = index.locals.get(name)?.clone();
        if spans.is_empty() {
            return None;
        }
        let imports = collect_imports(&ret.program);
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();

        let file = self.source_id(path, &source);
        let mut out = Vec::new();
        self.stack.push(path.to_path_buf());
        {
            let mut converter = Converter {
                loader: self,
                dir,
                imports,
                source: &source,
                file,
                file_path: path.to_path_buf(),
                index: &index,
                locals: Vec::new(),
            };
            for span in &spans {
                if let Some(el) = index.elements.get(span).copied() {
                    converter.convert_element(el, &mut out);
                }
            }
        }
        self.stack.pop();
        Some(out)
    }

    fn exports_of(&self, path: &Path) -> Option<(HashMap<String, (String, String)>, Vec<String>)> {
        let source = std::fs::read_to_string(path).ok()?;
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(path).ok()?.with_jsx(true);
        let ret = Parser::new(&allocator, &source, source_type).parse();
        if !parses_clean(&ret) {
            return None;
        }
        Some(reexports_of(&ret.program))
    }
}

struct Converter<'s, 'a> {
    loader: &'s mut Loader,
    dir: PathBuf,
    imports: Imports,
    source: &'s str,
    file: u32,
    file_path: PathBuf,
    index: &'s Indexed<'a>,
    /// 正在展开的文件内组件名，用于打断自引用。
    locals: Vec<String>,
}

impl<'s, 'a> Converter<'s, 'a> {
    fn convert_element(&mut self, el: &JSXElement<'_>, out: &mut Vec<Element>) {
        match self.kind(el) {
            Kind::Tag(tag) => out.push(self.build(el, tag)),
            Kind::Inline => {
                let name = element_name(&el.opening_element.name);
                let children = self.use_children(el);
                match self.loader.inline(&name, &self.dir, &self.imports, children) {
                    Some(roots) => out.extend(roots),
                    None => self.splice_unknown(el, out),
                }
            }
            Kind::Local(name) => {
                let children = self.use_children(el);
                self.inline_local(&name, children, out);
            }
            Kind::Unknown => self.splice_unknown(el, out),
        }
    }

    fn convert_child_element(&mut self, el: &JSXElement<'_>, out: &mut Vec<Child>) {
        match self.kind(el) {
            Kind::Tag(tag) => out.push(Child::Element(self.build(el, tag))),
            Kind::Inline => {
                let name = element_name(&el.opening_element.name);
                let children = self.use_children(el);
                match self.loader.inline(&name, &self.dir, &self.imports, children) {
                    Some(roots) => out.extend(roots.into_iter().map(Child::Element)),
                    None => {
                        let mut roots = Vec::new();
                        self.splice_unknown(el, &mut roots);
                        out.extend(roots.into_iter().map(Child::Element));
                    }
                }
            }
            Kind::Local(name) => {
                let children = self.use_children(el);
                let mut roots = Vec::new();
                self.inline_local(&name, children, &mut roots);
                out.extend(roots.into_iter().map(Child::Element));
            }
            Kind::Unknown => {
                let mut roots = Vec::new();
                self.splice_unknown(el, &mut roots);
                out.extend(roots.into_iter().map(Child::Element));
            }
        }
    }

    fn inline_local(&mut self, name: &str, children: Vec<Child>, out: &mut Vec<Element>) {
        if self.locals.iter().any(|current| current == name) {
            return;
        }
        let spans = self.index.locals.get(name).cloned().unwrap_or_default();
        self.locals.push(name.to_string());
        for span in spans {
            let Some(el) = self.index.elements.get(&span).copied() else {
                continue;
            };
            let mut roots = Vec::new();
            self.convert_element(el, &mut roots);
            out.extend(roots.into_iter().map(|root| substitute(root, &children)));
        }
        self.locals.pop();
    }

    fn kind(&self, el: &JSXElement<'_>) -> Kind {
        let raw = element_name(&el.opening_element.name);
        if let Some(tag) = table::tag_for(&raw) {
            return Kind::Tag(tag.to_string());
        }
        let tag = table::kebab(&raw);
        if self.loader.declared.contains(&tag.to_ascii_lowercase()) {
            return Kind::Tag(tag);
        }
        if self.index.locals.contains_key(&raw) {
            return Kind::Local(raw);
        }
        if self.imports.contains_key(&raw) {
            return Kind::Inline;
        }
        Kind::Unknown
    }

    fn build(&mut self, el: &JSXElement<'_>, tag: String) -> Element {
        let span = el.span.start;
        let mut attrs = Vec::new();
        for item in &el.opening_element.attributes {
            let JSXAttributeItem::Attribute(attr) = item else {
                continue;
            };
            let name = match &attr.name {
                JSXAttributeName::Identifier(id) => id.name.as_str().to_string(),
                JSXAttributeName::NamespacedName(ns) => {
                    format!("{}:{}", ns.namespace.name.as_str(), ns.name.name.as_str())
                }
            };
            if name == "dataSet" || name == "dataset" {
                expand_dataset(attr, self.file, &mut attrs);
                continue;
            }
            // 事件名按 Taro 的规则改写：属性名成 `bind*` / `worklet:*` 才能被规则读到。
            // 值写运行时的统一处理函数 `eh`——规则只读名字，写作者的方法名反而指向
            // 合成工程里不存在的函数。
            let event = match name.as_str() {
                // `catchMove` 不是事件，语义是「阻断 touchmove 冒泡」，运行时等价
                // `catchtouchmove`（Taro 的 catch-view 模板就是这么产出的）。
                "catchMove" => Some("catchtouchmove".to_string()),
                _ => table::event_name(&name),
            };
            if let Some(event) = event {
                attrs.push(Attr {
                    name: event,
                    value: AttrValue::Static("eh".to_string()),
                    file: self.file,
                    span: attr.span.start,
                });
                continue;
            }
            let Some(wxml_name) = table::attr_name(&name) else {
                continue;
            };
            attrs.push(Attr {
                value: attr_value(&name, attr),
                name: wxml_name,
                file: self.file,
                span: attr.span.start,
            });
        }
        for (name, value) in table::defaults_for(&tag) {
            if !attrs.iter().any(|attr| attr.name == *name) {
                attrs.push(Attr {
                    name: (*name).to_string(),
                    value: AttrValue::Static((*value).to_string()),
                    file: self.file,
                    span,
                });
            }
        }

        let mut children = Vec::new();
        for child in &el.children {
            self.convert_child(child, &mut children);
        }
        Element {
            tag,
            file: self.file,
            span,
            attrs,
            children,
        }
    }

    fn use_children(&mut self, el: &JSXElement<'_>) -> Vec<Child> {
        let mut children = Vec::new();
        for child in &el.children {
            self.convert_child(child, &mut children);
        }
        children
    }

    fn convert_child(&mut self, child: &JSXChild<'_>, out: &mut Vec<Child>) {
        match child {
            JSXChild::Text(text) => {
                let value = normalize_text(text.value.as_str());
                if !value.is_empty() {
                    out.push(Child::Text {
                        text: value,
                        file: self.file,
                        span: text.span.start,
                    });
                }
            }
            JSXChild::Element(el) => self.convert_child_element(el, out),
            JSXChild::Fragment(fragment) => {
                for child in &fragment.children {
                    self.convert_child(child, out);
                }
            }
            JSXChild::ExpressionContainer(container) => {
                self.convert_expr_child(&container.expression, out);
            }
            JSXChild::Spread(_) => {}
        }
    }

    fn convert_expr_child(&mut self, jsx: &JSXExpression<'_>, out: &mut Vec<Child>) {
        let Some(expr) = jsx.as_expression() else {
            return;
        };
        match expr {
            Expression::JSXElement(el) => self.convert_child_element(el, out),
            Expression::JSXFragment(fragment) => {
                for child in &fragment.children {
                    self.convert_child(child, out);
                }
            }
            Expression::StringLiteral(lit) => {
                let value = normalize_text(lit.value.as_str().unwrap_or(""));
                if !value.is_empty() {
                    out.push(Child::Text {
                        text: value,
                        file: self.file,
                        span: lit.span.start,
                    });
                }
            }
            Expression::NumericLiteral(lit) => out.push(Child::Text {
                text: lit.value.to_string(),
                file: self.file,
                span: lit.span.start,
            }),
            Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => {
                out.push(Child::Text {
                    text: cooked(tpl),
                    file: self.file,
                    span: tpl.span.start,
                });
            }
            // 三元、`&&`、`||` 在 WXML 里就是条件分支：`wx:if` + `wx:else`。
            // 两个分支都保留（规则判「可能出现的结构」），但标成互斥——`branch.rs`
            // 靠 `wx:if` 的分组判同一条分支链的两个候选不会同时渲染。
            Expression::ConditionalExpression(cond) => {
                self.convert_branch(&cond.consequent, "wx:if", out);
                self.convert_branch(&cond.alternate, "wx:else", out);
            }
            Expression::LogicalExpression(logic) if logic.operator == LogicalOperator::And => {
                self.convert_branch(&logic.right, "wx:if", out);
            }
            Expression::LogicalExpression(logic) => {
                self.convert_branch(&logic.left, "wx:if", out);
                self.convert_branch(&logic.right, "wx:else", out);
            }
            Expression::ParenthesizedExpression(paren) => {
                self.convert_expr_child_expr(&paren.expression, out);
            }
            // `{props.children}` / `{children}`：内联到调用点后补上实际子节点。
            Expression::StaticMemberExpression(_) | Expression::Identifier(_)
                if self.is_slot(expr) =>
            {
                out.push(Child::Slot);
            }
            // `.map(...)` 的回调体整体重复，对应 `wx:for`；其它调用（`arr.filter(...)`、
            // `useMemo(...)`）的返回值静态不可知。
            Expression::CallExpression(call) if is_map_call(call) => {
                let arrow = call.arguments.iter().find_map(|arg| match arg.as_expression() {
                    Some(Expression::ArrowFunctionExpression(arrow)) => Some(arrow),
                    _ => None,
                });
                let Some(arrow) = arrow else {
                    out.push(Child::Dynamic {
                        file: self.file,
                        span: expr.span().start,
                    });
                    return;
                };
                let mut body = Vec::new();
                match &arrow.body {
                    ArrowFunctionBody::FunctionBody(statements) => {
                        for statement in &statements.statements {
                            if let Statement::ReturnStatement(ret) = statement
                                && let Some(returned) = &ret.argument
                            {
                                self.convert_expr_child_expr(returned, &mut body);
                            }
                        }
                    }
                    arrow_body => {
                        if let Some(returned) = arrow_body.as_expression() {
                            self.convert_expr_child_expr(returned, &mut body);
                        }
                    }
                }
                self.push_marked(body, "wx:for", expr.span().start, out);
            }
            Expression::ArrayExpression(array) => {
                for element in &array.elements {
                    if let Some(expr) = element.as_expression() {
                        self.convert_expr_child_expr(expr, out);
                    }
                }
            }
            _ => out.push(Child::Dynamic {
                file: self.file,
                span: expr.span().start,
            }),
        }
    }

    fn convert_expr_child_expr(&mut self, expr: &Expression<'_>, out: &mut Vec<Child>) {
        self.convert_expr_child(expr.as_jsx_expression(), out);
    }

    /// 一个条件 / 循环分支：先照常展开，再打上 `wx:if` / `wx:else` / `wx:for`。
    fn convert_branch(&mut self, expr: &Expression<'_>, marker: &str, out: &mut Vec<Child>) {
        let mut nodes = Vec::new();
        self.convert_expr_child_expr(expr, &mut nodes);
        self.push_marked(nodes, marker, expr.span().start, out);
    }

    /// 单个元素直接挂属性；文本、动态节点或展开出的多个元素包一层 `<block>`。
    ///
    /// `<block>` 在渲染上不产生自己的盒子，skylint 也会把它摊平（`input/wxml.rs`），
    /// 所以包与不包不改变兄弟结构，只在需要承载条件 / 循环属性时使用。
    fn push_marked(&mut self, mut nodes: Vec<Child>, marker: &str, span: u32, out: &mut Vec<Child>) {
        if nodes.is_empty() {
            return;
        }
        let attr = Attr {
            name: marker.to_string(),
            // `wx:else` 在 WXML 里本身不带值。
            value: match marker {
                "wx:else" => AttrValue::Bool,
                _ => AttrValue::Static("{{_}}".to_string()),
            },
            file: self.file,
            span,
        };
        if let [Child::Element(element)] = nodes.as_mut_slice() {
            // 已经带条件 / 循环标记时不叠在同一个元素上：`wx:for` 与 `wx:if` 同挂
            // 一个节点优先级有坑，包一层 `<block>` 两者的作用域才清楚。
            if !element.attrs.iter().any(|attr| attr.name.starts_with("wx:")) {
                element.attrs.push(attr);
                out.append(&mut nodes);
                return;
            }
        }
        out.push(Child::Element(Element {
            tag: "block".to_string(),
            file: self.file,
            span,
            attrs: vec![attr],
            children: nodes,
        }));
    }

    fn is_slot(&self, expr: &Expression<'_>) -> bool {
        let text = self
            .source
            .get(expr.span().start as usize..expr.span().end as usize)
            .unwrap_or("")
            .trim();
        matches!(text, "props.children" | "children" | "this.props.children")
    }

    /// 无法还原的组件：把作者写的子节点**上提**一级。
    ///
    /// Taro 里这类组件由 React 渲染器内联，真实节点树里没有它自己的盒子；
    /// 丢掉整棵子树会让整个页面结构消失（`AuthWrapper`、`BoxLink` 这类包装组件
    /// 很常见），上提至少保住了页面自己的结构，代价是少一层不确定的父节点。
    fn splice_unknown(&mut self, el: &JSXElement<'_>, out: &mut Vec<Element>) {
        if el.children.is_empty() {
            let raw = element_name(&el.opening_element.name);
            let spec = self
                .imports
                .get(&raw)
                .map(|(spec, _)| spec.as_str())
                .unwrap_or("未导入的组件");
            let notice = format!(
                "{} 里的 <{raw}>（{spec}）无法静态还原，已跳过",
                self.file_path.display()
            );
            if !self.loader.notices.contains(&notice) {
                self.loader.notices.push(notice);
            }
            return;
        }
        let mut children = Vec::new();
        for child in &el.children {
            self.convert_child(child, &mut children);
        }
        for child in children {
            match child {
                Child::Element(element) => out.push(element),
                // 裸文本/未知文本位留在未知组件的 slot 里，没有父节点可挂，丢弃。
                Child::Text { .. } | Child::Dynamic { .. } | Child::Slot => {}
            }
        }
    }
}

enum Kind {
    Tag(String),
    Inline,
    Local(String),
    Unknown,
}

/// 把组件输出里的 `<Slot>` 换成使用处的 children。
///
/// 代入的 children 本身已经是成品树，不能再代入一次——否则嵌套 slot 会
/// 每层都把同一批 children 重新套进去，树深指数增长。
fn substitute(mut root: Element, children: &[Child]) -> Element {
    let mut out = Vec::with_capacity(root.children.len());
    for child in std::mem::take(&mut root.children) {
        match child {
            Child::Slot => out.extend(children.iter().cloned()),
            Child::Element(element) => out.push(Child::Element(substitute(element, children))),
            other => out.push(other),
        }
    }
    root.children = out;
    root
}

fn collect_imports(program: &Program<'_>) -> Imports {
    let mut imports = Imports::new();
    for statement in &program.body {
        let Statement::ImportDeclaration(decl) = statement else {
            continue;
        };
        let spec = decl.source.value.as_str().unwrap_or("");
        for specifier in decl.specifiers.iter().flatten() {
            match specifier {
                ImportDeclarationSpecifier::ImportDefaultSpecifier(item) => {
                    imports.insert(
                        item.local.name.as_str().to_string(),
                        (spec.to_string(), "default".to_string()),
                    );
                }
                ImportDeclarationSpecifier::ImportSpecifier(item) => {
                    let imported = item.imported.name().as_str().to_string();
                    imports.insert(
                        item.local.name.as_str().to_string(),
                        (spec.to_string(), imported),
                    );
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => {}
            }
        }
    }
    imports
}

/// 语法错误 → 这个文件里的组件无法静态还原。
fn parses_clean(ret: &ParserReturn<'_>) -> bool {
    ret.diagnostics.errors().next().is_none()
}

/// 解析 import 说明符。
///
/// `@/x` 既可能指向 `src/x`（Taro 模板默认），也可能指向项目根（`@/src/x`），
/// 取决于工程的 `alias` 配置；两种都试，存在即用。
fn resolve_specifier(src_root: &Path, dir: &Path, spec: &str) -> Option<PathBuf> {
    let mut bases = Vec::new();
    if let Some(rest) = spec.strip_prefix("@/") {
        bases.push(src_root.join(rest));
        if let Some(project) = src_root.parent() {
            bases.push(project.join(rest));
        }
    } else if spec.starts_with('.') {
        bases.push(dir.join(spec));
    } else {
        // npm 组件无法从源码还原。
        return None;
    }

    for base in bases {
        if base.is_file() {
            return Some(clean(&base));
        }
        for ext in ["tsx", "jsx", "ts", "js"] {
            let candidate = base.with_extension(ext);
            if candidate.is_file() {
                return Some(clean(&candidate));
            }
        }
        for ext in ["tsx", "jsx", "ts", "js"] {
            let candidate = base.join("index").with_extension(ext);
            if candidate.is_file() {
                return Some(clean(&candidate));
            }
        }
    }
    None
}

/// 词法归一：`a/./b`、`a/x/../b` 与 `a/b` 是同一个文件，环检测与诊断里的
/// 路径都必须按同一条写法比较。
fn clean(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir if out.components().next_back().is_some_and(|last| matches!(last, Component::Normal(_))) => {
                out.pop();
            }
            component => out.push(component),
        }
    }
    out
}

/// 默认导出的函数（页面函数、组件函数）的 span；没有默认导出时 `None`（整份文件）。
fn exported_function_span(program: &Program<'_>) -> Option<Span> {
    for statement in &program.body {
        let Statement::ExportDefaultDeclaration(decl) = statement else {
            continue;
        };
        match &decl.declaration {
            ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                return Some(function.span);
            }
            ExportDefaultDeclarationKind::Identifier(id) => {
                return variable_initializer_span(program, id.name.as_str());
            }
            ExportDefaultDeclarationKind::ArrowFunctionExpression(function) => {
                return Some(function.span);
            }
            _ => return None,
        }
    }
    None
}

fn variable_initializer_span(program: &Program<'_>, name: &str) -> Option<Span> {
    for statement in &program.body {
        let Statement::VariableDeclaration(decl) = statement else {
            continue;
        };
        for declarator in &decl.declarations {
            if let BindingPattern::BindingIdentifier(id) = &declarator.id
                && id.name.as_str() == name
            {
                return declarator.init.as_ref().map(|expr| expr.span());
            }
        }
    }
    None
}

fn spans_strictly_contains(outer: Span, inner: Span) -> bool {
    outer.start <= inner.start && outer.end >= inner.end && outer != inner
}

fn element_name(name: &JSXElementName<'_>) -> String {
    match name {
        JSXElementName::Identifier(id) => id.name.as_str().to_string(),
        JSXElementName::IdentifierReference(id) => id.name.as_str().to_string(),
        JSXElementName::MemberExpression(member) => member.property.name.as_str().to_string(),
        JSXElementName::NamespacedName(ns) => ns.name.name.as_str().to_string(),
        JSXElementName::ThisExpression(_) => "this".to_string(),
    }
}

fn attr_value(name: &str, attr: &JSXAttribute<'_>) -> AttrValue {
    match name {
        "className" => return class_value(attr),
        "style" => return style_value(attr),
        _ => {}
    }
    match &attr.value {
        None => AttrValue::Bool,
        Some(JSXAttributeValue::StringLiteral(lit)) => {
            AttrValue::Static(lit.value.as_str().unwrap_or("").to_string())
        }
        Some(JSXAttributeValue::ExpressionContainer(container)) => {
            match container.expression.as_expression() {
                Some(Expression::StringLiteral(lit)) => {
                    AttrValue::Static(lit.value.as_str().unwrap_or("").to_string())
                }
                Some(Expression::NumericLiteral(lit)) => AttrValue::Static(lit.value.to_string()),
                Some(Expression::TemplateLiteral(tpl)) if tpl.expressions.is_empty() => {
                    AttrValue::Static(cooked(tpl))
                }
                Some(Expression::BooleanLiteral(lit)) => {
                    AttrValue::Static(lit.value.to_string())
                }
                _ => AttrValue::Dynamic,
            }
        }
        _ => AttrValue::Dynamic,
    }
}

fn class_value(attr: &JSXAttribute<'_>) -> AttrValue {
    let mut tokens: Vec<String> = Vec::new();
    let dynamic = match &attr.value {
        Some(JSXAttributeValue::StringLiteral(lit)) => {
            tokens.extend(lit.value.as_str().unwrap_or("").split_whitespace().map(str::to_string));
            false
        }
        Some(JSXAttributeValue::ExpressionContainer(container)) => {
            match container.expression.as_expression() {
                Some(expr) => class_expr(expr, &mut tokens),
                None => true,
            }
        }
        _ => true,
    };
    // 原生 `class="a {{b}}"` 也是动态属性（matcher 只能得到 Unknown），
    // 这里同样不保留静态前缀，保持与原生同口径。
    if dynamic || tokens.is_empty() {
        AttrValue::Dynamic
    } else {
        AttrValue::Static(tokens.join(" "))
    }
}

fn class_expr(expr: &Expression<'_>, tokens: &mut Vec<String>) -> bool {
    match expr {
        Expression::StringLiteral(lit) => {
            tokens.extend(lit.value.as_str().unwrap_or("").split_whitespace().map(str::to_string));
            false
        }
        Expression::TemplateLiteral(tpl) => {
            for quasi in &tpl.quasis {
                if let Some(cooked) = quasi.value.cooked.and_then(|value| value.as_str()) {
                    tokens.extend(cooked.split_whitespace().map(str::to_string));
                }
            }
            !tpl.expressions.is_empty()
        }
        Expression::ConditionalExpression(cond) => {
            let left = class_expr(&cond.consequent, tokens);
            let right = class_expr(&cond.alternate, tokens);
            left || right
        }
        Expression::LogicalExpression(logic) => {
            // `cond && 'x'`：结果集含静态 token，但条件不可证。
            let _ = class_expr(&logic.right, tokens);
            true
        }
        Expression::ParenthesizedExpression(paren) => class_expr(&paren.expression, tokens),
        Expression::ArrayExpression(array) => {
            let mut dynamic = false;
            for element in &array.elements {
                match element {
                    ArrayExpressionElement::StringLiteral(lit) => {
                        tokens.extend(
                            lit.value.as_str().unwrap_or("").split_whitespace().map(str::to_string),
                        );
                    }
                    ArrayExpressionElement::ConditionalExpression(cond) => {
                        dynamic |= class_expr(&cond.consequent, tokens);
                        dynamic |= class_expr(&cond.alternate, tokens);
                    }
                    _ => dynamic = true,
                }
            }
            dynamic
        }
        Expression::ObjectExpression(object) => {
            let mut dynamic = false;
            for property in &object.properties {
                let ObjectPropertyKind::ObjectProperty(property) = property else {
                    dynamic = true;
                    continue;
                };
                let PropertyKey::StaticIdentifier(key) = &property.key else {
                    dynamic = true;
                    continue;
                };
                if matches!(
                    property.value,
                    Expression::StringLiteral(_)
                        | Expression::NumericLiteral(_)
                        | Expression::BooleanLiteral(_)
                ) {
                    tokens.push(key.name.as_str().to_string());
                } else {
                    dynamic = true;
                }
            }
            dynamic
        }
        // `clsx(...)` / `classNames(...)`：静态字符串参数可用。
        Expression::CallExpression(call) => {
            let mut dynamic = false;
            for arg in &call.arguments {
                match arg.as_expression() {
                    Some(Expression::StringLiteral(lit)) => {
                        tokens.extend(
                            lit.value.as_str().unwrap_or("").split_whitespace().map(str::to_string),
                        );
                    }
                    _ => dynamic = true,
                }
            }
            dynamic
        }
        _ => true,
    }
}

fn style_value(attr: &JSXAttribute<'_>) -> AttrValue {
    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attr.value else {
        return match &attr.value {
            Some(JSXAttributeValue::StringLiteral(lit)) => {
                AttrValue::Static(lit.value.as_str().unwrap_or("").to_string())
            }
            None => AttrValue::Bool,
            _ => AttrValue::Dynamic,
        };
    };
    let Some(Expression::ObjectExpression(object)) = container.expression.as_expression() else {
        return AttrValue::Dynamic;
    };
    let mut out = String::new();
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            return AttrValue::Dynamic;
        };
        let PropertyKey::StaticIdentifier(key) = &property.key else {
            return AttrValue::Dynamic;
        };
        let value = match &property.value {
            Expression::StringLiteral(lit) => lit.value.as_str().unwrap_or("").to_string(),
            Expression::NumericLiteral(lit) => format!("{}px", lit.value),
            Expression::TemplateLiteral(tpl) if tpl.expressions.is_empty() => cooked(tpl),
            _ => return AttrValue::Dynamic,
        };
        out.push_str(&table::kebab(key.name.as_str()));
        out.push(':');
        out.push_str(&value);
        out.push(';');
    }
    if out.is_empty() {
        AttrValue::Dynamic
    } else {
        AttrValue::Static(out)
    }
}

/// `dataSet={{ {foo: 1} }}` → `data-foo="1"`（属性选择器要按 data-* 看）。
fn expand_dataset(attr: &JSXAttribute<'_>, file: u32, attrs: &mut Vec<Attr>) {
    let Some(JSXAttributeValue::ExpressionContainer(container)) = &attr.value else {
        return;
    };
    let Some(Expression::ObjectExpression(object)) = container.expression.as_expression() else {
        return;
    };
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else {
            continue;
        };
        let PropertyKey::StaticIdentifier(key) = &property.key else {
            continue;
        };
        let value = match &property.value {
            Expression::StringLiteral(lit) => {
                AttrValue::Static(lit.value.as_str().unwrap_or("").to_string())
            }
            Expression::NumericLiteral(lit) => AttrValue::Static(lit.value.to_string()),
            _ => AttrValue::Dynamic,
        };
        attrs.push(Attr {
            name: format!("data-{}", table::kebab(key.name.as_str())),
            value,
            file,
            span: property.span.start,
        });
    }
}

/// `.map(...)`：只有它产出「一组同类元素」，对应 `wx:for`。
fn is_map_call(call: &CallExpression<'_>) -> bool {
    matches!(&call.callee, Expression::StaticMemberExpression(member) if member.property.name == "map")
}

fn cooked(tpl: &TemplateLiteral<'_>) -> String {
    tpl.quasis
        .iter()
        .map(|quasi| {
            quasi
                .value
                .cooked
                .and_then(|value| value.as_str())
                .map(str::to_string)
                .unwrap_or_default()
        })
        .collect()
}

fn normalize_text(raw: &str) -> String {
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    lines.join(" ")
}

