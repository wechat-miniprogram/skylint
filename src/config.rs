//! 工程级配置：`.skylint/config.json`。
//!
//! 目前只有一项——排除路径。存在的理由很具体：真实工程里常带第三方样式
//! （`weui.wxss` 这类 4000 行、多份拷贝、本来就不该手改的文件），它们在
//! `unit:shared` 里能占九成诊断，把「共享资源还需要改什么」完全淹掉。
//!
//! 放在文件里而不是命令行开关，是因为**每个入口都要看见它**：`plan` / `job` /
//! `baseline` / `verify` 如果各带一套开关，作业单里的验收命令就会和基线用不同的
//! 过滤集，门禁立刻失真。

use std::path::Path;

use serde::Deserialize;

/// 配置文件相对工程根的位置。
pub const CONFIG_PATH: &str = ".skylint/config.json";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    #[serde(default)]
    exclude: Vec<String>,
}

/// 被排除的路径模式。
#[derive(Debug, Clone, Default)]
pub struct Excludes {
    patterns: Vec<String>,
}

impl Excludes {
    /// 读 `<root>/.skylint/config.json`；文件不存在或解析失败时等于「不排除任何东西」。
    ///
    /// 解析失败不报错：配置是辅助信息，坏掉的配置不该让整轮检查失败。要发现它，
    /// 直接看 `skylint --json` 的 `excluded` 字段有没有生效。
    pub fn load(root: &Path) -> Self {
        let Ok(source) = std::fs::read_to_string(root.join(CONFIG_PATH)) else {
            return Self::default();
        };
        let Ok(raw) = serde_json::from_str::<Raw>(crate::input::strip_bom(&source)) else {
            return Self::default();
        };
        Self {
            patterns: raw.exclude,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.patterns.is_empty()
    }

    pub fn matches(&self, path: &str) -> bool {
        self.patterns
            .iter()
            .any(|pattern| glob_match(pattern, path))
    }
}

/// 极简 glob：`*` 匹配任意字符（**含 `/`**），`?` 匹配单个字符，其余按字面量。
///
/// 不引入 glob 依赖：这里只需要「按路径排除」这一种用法，而 `*` 跨 `/` 的语义
/// 让 `packages/*/styles/weui.wxss` 与 `**/weui.wxss` 写成同一个东西，少一个要记的规则。
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    // `star` 记住最近一个 `*` 的位置，回溯时从那里重新展开。
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    #[test]
    fn star_crosses_directories() {
        assert!(glob_match(
            "packages/*/styles/weui.wxss",
            "packages/a/styles/weui.wxss"
        ));
        assert!(glob_match("**/weui.wxss", "packages/a/styles/weui.wxss"));
        assert!(glob_match("*.wxss", "a/b/c.wxss"));
        assert!(!glob_match("*.wxss", "a/b/c.scss"));
    }

    #[test]
    fn question_matches_one_character() {
        assert!(glob_match("a?.wxss", "ab.wxss"));
        assert!(!glob_match("a?.wxss", "abc.wxss"));
    }

    #[test]
    fn trailing_star_matches_rest() {
        assert!(glob_match(
            "packages/quick-fill-info/*",
            "packages/quick-fill-info/styles/weui.wxss"
        ));
        assert!(glob_match("*", "anything"));
        assert!(!glob_match("packages/*", "common.scss"));
    }
}
