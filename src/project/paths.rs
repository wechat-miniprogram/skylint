//! posix 路径工具。
//!
//! 工程的路径一律是「项目内相对路径 + `/` 分隔」，不经过平台路径类型，避免 Windows
//! 反斜杠混进诊断、字典键和 `@import` 解析。语义与 node 的 `path.posix` 子集一致。

/// `path.posix.normalize`，另把反斜杠归一成正斜杠。
pub fn posix_normalize(path: &str) -> String {
    let path = path.replace('\\', "/");
    let absolute = path.starts_with('/');
    let mut out: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => match out.last() {
                Some(last) if *last != ".." => {
                    out.pop();
                }
                _ => {
                    // 绝对路径下 `..` 到达根即丢弃
                    if !absolute {
                        out.push("..");
                    }
                }
            },
            s => out.push(s),
        }
    }
    let joined = out.join("/");
    if absolute {
        format!("/{joined}")
    } else if joined.is_empty() {
        ".".to_string()
    } else {
        joined
    }
}

/// `path.posix.join`。
pub fn posix_join(base: &str, append: &str) -> String {
    if base.is_empty() {
        return posix_normalize(append);
    }
    if append.is_empty() {
        return posix_normalize(base);
    }
    posix_normalize(&format!("{base}/{append}"))
}

/// `path.posix.dirname`。
pub fn posix_dirname(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(i) => path[..i].to_string(),
        None => ".".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_normalizes_dots() {
        assert_eq!(posix_join("pages/a", "../b"), "pages/b");
        assert_eq!(posix_join("", "a/./b"), "a/b");
        assert_eq!(posix_join("pages", "/abs"), "pages/abs");
    }

    #[test]
    fn dirname_matches_posix() {
        assert_eq!(posix_dirname("a/b.wxml"), "a");
        assert_eq!(posix_dirname("a"), ".");
        assert_eq!(posix_dirname("/a"), "/");
    }
}
