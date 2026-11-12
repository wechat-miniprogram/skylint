//! 规则测试共用的临时工程夹具。
//!
//! 唯一性由 [`crate::fixture_root`] 保证：临时根带测试线程名，同一进程并发跑测试
//! 也不会互相删目录。

use std::fs;
use std::path::{Path, PathBuf};

use crate::Diagnostic;
use crate::fixture_root;
use crate::project::Project;
use crate::rules::Rule;

pub(crate) struct Fixture {
    root: PathBuf,
}

impl Fixture {
    pub(crate) fn new(name: &str) -> Self {
        let root = fixture_root("skylint", name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self { root }
    }

    /// 单页 Skyline 工程，页面 WXML 即 `wxml`。
    pub(crate) fn wxml(name: &str, wxml: &str) -> Self {
        let fixture = Self::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml);
        fixture
    }

    /// 单页 Skyline 工程，页面 WXML + WXSS 都由用例给。
    pub(crate) fn page(name: &str, wxml: &str, wxss: &str) -> Self {
        let fixture = Self::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        fixture
    }

    /// 单页 Skyline 工程，页面 JS 即 `js`。
    pub(crate) fn js(name: &str, js: &str) -> Self {
        let fixture = Self::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", js);
        fixture
    }

    /// 单页 Skyline 工程，附带一份 `app.js`。
    pub(crate) fn js_with_app(name: &str, js: &str, app_js: Option<&str>) -> Self {
        let fixture = Self::js(name, js);
        if let Some(app_js) = app_js {
            fixture.write("app.js", app_js);
        }
        fixture
    }

    /// 小程序插件工程：只有 `compileType` 不同。
    pub(crate) fn plugin(name: &str, js: &str) -> Self {
        let fixture = Self::new(name);
        fixture
            .write(
                "project.config.json",
                r#"{"miniprogramRoot":"","compileType":"plugin"}"#,
            )
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline"}"#,
            )
            .write("pages/index/index.json", "{}")
            .write("pages/index/index.wxml", "<view/>")
            .write("pages/index/index.js", js);
        fixture
    }

    pub(crate) fn write(&self, rel: &str, content: &str) -> &Self {
        self.write_bytes(rel, content.as_bytes())
    }

    pub(crate) fn write_bytes(&self, rel: &str, content: &[u8]) -> &Self {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self
    }

    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn project(&self) -> Project {
        Project::load(&self.root).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) fn messages(rule: &dyn Rule, project: &Project) -> Vec<String> {
    rule.check(project).into_iter().map(|d| d.message).collect()
}

pub(crate) fn check_rule(fixture: &Fixture, rule: &dyn Rule) -> Vec<Diagnostic> {
    rule.check(&Project::load(fixture.root()).unwrap())
}

/// WXML 规则的默认单页工程。
pub(crate) mod wxml {
    pub(crate) use super::{Fixture, check_rule};

    pub(crate) fn project(name: &str, wxml: &str) -> Fixture {
        Fixture::wxml(name, wxml)
    }
}

/// WXSS 规则的默认单页工程。
pub(crate) mod wxss {
    use super::Project;

    pub(crate) use super::{Fixture, messages};

    /// 单页工程上跑一条规则（WXML + WXSS 都由用例给）。
    pub(crate) fn check(
        rule: &dyn crate::rules::Rule,
        name: &str,
        wxml: &str,
        wxss: &str,
    ) -> Vec<crate::Diagnostic> {
        let fixture = Fixture::page(name, wxml, wxss);
        super::check_rule(&fixture, rule)
    }

    /// 带 WXML 的单页工程：渲染器分支类要验到节点上时用。
    pub(crate) fn page(name: &str, wxml: &str, wxss: &str) -> (Fixture, Project) {
        let fixture = Fixture::page(name, wxml, wxss);
        let project = fixture.project();
        (fixture, project)
    }

    /// app 级开了 `defaultDisplayBlock`、页面级又单独关掉：这是逐条报
    /// `flex-direction` 的唯一工程形态（app 级没开时整片收成一条）。
    pub(crate) fn page_opt_out(name: &str, wxml: &str, wxss: &str) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", r#"{"miniprogramRoot":""}"#)
            .write(
                "app.json",
                r#"{"pages":["pages/index/index"],"renderer":"skyline","rendererOptions":{"skyline":{"defaultDisplayBlock":true}}}"#,
            )
            .write(
                "pages/index/index.json",
                r#"{"rendererOptions":{"skyline":{"defaultDisplayBlock":false}}}"#,
            )
            .write("pages/index/index.wxml", wxml)
            .write("pages/index/index.wxss", wxss);
        let project = fixture.project();
        (fixture, project)
    }
}

/// 配置类规则的默认工程：app / 页面 json / WXML 都由用例给。
pub(crate) mod config {
    use super::Project;

    pub(crate) use super::Fixture;

    /// `project.config.json` 用默认 `miniprogramRoot`。
    pub(crate) fn project(name: &str, app: &str, page: &str) -> (Fixture, Project) {
        project_with_config(name, r#"{"miniprogramRoot":""}"#, app, page)
    }

    /// 要验 `project.config.json` 本身时用这个。
    pub(crate) fn project_with_config(
        name: &str,
        config: &str,
        app: &str,
        page: &str,
    ) -> (Fixture, Project) {
        let fixture = Fixture::new(name);
        fixture
            .write("project.config.json", config)
            .write("app.json", app)
            .write("pages/index/index.json", page)
            .write("pages/index/index.wxml", "<view/>");
        let loaded = fixture.project();
        (fixture, loaded)
    }
}

/// JS 规则的默认单页 / 插件工程。
pub(crate) mod js {
    use super::Project;

    pub(crate) use super::{Fixture, messages};

    pub(crate) fn single(name: &str, js: &str) -> (Fixture, Project) {
        single_with_app(name, js, None)
    }

    pub(crate) fn single_with_app(
        name: &str,
        js: &str,
        app_js: Option<&str>,
    ) -> (Fixture, Project) {
        let fixture = Fixture::js_with_app(name, js, app_js);
        let project = fixture.project();
        (fixture, project)
    }

    pub(crate) fn plugin(name: &str, js: &str) -> (Fixture, Project) {
        let fixture = Fixture::plugin(name, js);
        let project = fixture.project();
        (fixture, project)
    }
}
