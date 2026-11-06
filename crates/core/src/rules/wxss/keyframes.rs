//! `@keyframes` 块的原始扫描。
//!
//! 输入层把关键帧块摊平成普通规则（选择器是 `from` / `to`），`at_rules` 为空，
//! 拿不到所属动画名，所以按源码扫。两种用法：
//!
//! - `transition-animation-conflict` 需要「名字 → 花括号内原文」；
//! - `vendor-prefix` 需要知道某个块带不带厂商前缀。

/// 源码里的一段 `@keyframes`（含带前缀的写法）。
pub(crate) struct Keyframe<'a> {
    /// 动画名（不含引号）。
    pub name: &'a str,
    /// 带前缀时的前缀（`"-webkit-"` / `"-moz-"` / `"-o-"`），无前缀为 `None`。
    pub prefix: Option<&'a str>,
    /// `@` 的字节偏移，用于把诊断落在正确位置。
    pub offset: usize,
    /// 花括号内的原文。
    pub body: &'a str,
}

/// 按源码顺序扫描全部关键帧块。花括号不配对（语法错误）时停止。
pub(crate) fn blocks(source: &str) -> Vec<Keyframe<'_>> {
    const KEYWORD: &str = "keyframes";
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find(KEYWORD) {
        let keyword_at = cursor + found;
        let Some(at) = source[..keyword_at].rfind('@') else {
            cursor = keyword_at + KEYWORD.len();
            continue;
        };
        let prefix = &source[at + 1..keyword_at];
        let prefix = match prefix {
            "" => None,
            "-webkit-" | "-moz-" | "-o-" => Some(prefix),
            _ => None,
        };
        if prefix.is_none() && !source[at + 1..keyword_at].is_empty() {
            // `@supports (animation-timeline: keyframes…)` 这类不是关键帧块。
            cursor = keyword_at + KEYWORD.len();
            continue;
        }
        let mut index = keyword_at + KEYWORD.len();
        while index < bytes.len() && (bytes[index] as char).is_whitespace() {
            index += 1;
        }
        let name_start = index;
        while index < bytes.len()
            && !matches!(bytes[index] as char, '{' | ' ' | '\n' | '\t' | '\r' | ';')
        {
            index += 1;
        }
        let name = source[name_start..index].trim();
        while index < bytes.len() && bytes[index] != b'{' {
            if bytes[index] == b';' {
                break;
            }
            index += 1;
        }
        if index >= bytes.len() || bytes[index] != b'{' || name.is_empty() {
            cursor = index.max(keyword_at + KEYWORD.len());
            continue;
        }
        let body_start = index + 1;
        let mut depth = 1;
        index = body_start;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        let body_end = index.saturating_sub(1);
        out.push(Keyframe {
            name,
            prefix,
            offset: at,
            body: &source[body_start..body_end],
        });
        cursor = index;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::blocks;

    #[test]
    fn plain_and_prefixed_blocks_are_found_in_order() {
        let source = "@keyframes a { from {} }\n@-webkit-keyframes b { to {} }";
        let found = blocks(source);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].name, found[0].prefix), ("a", None));
        assert_eq!((found[1].name, found[1].prefix), ("b", Some("-webkit-")));
        assert_eq!(found[1].body.trim(), "to {}");
        assert_eq!(found[1].offset, source.find("@-webkit-keyframes").unwrap());
    }

    #[test]
    fn unrelated_at_rules_are_ignored() {
        assert!(blocks("@supports (animation-name: keyframes) { .a {} }").is_empty());
        assert!(blocks("@media screen { .a { color: red } }").is_empty());
    }
}
