//! JSON 原文定位。
//!
//! `serde_json::Value` 只留结构、不留 span，而配置类规则的诊断必须落在用户写的
//! 那一行上。这里不写第二套 JSON 解析器：只按词法扫 `"key"`，把字节偏移换算成
//! 行列（`column` 按 Unicode 字符计，与 [`crate::input::wxss::WxssDoc::position`]
//! 同一语义）。取值一律仍由 `serde_json` 负责。
//!
//! 已知的取舍：只认第一个「后跟 `:`」的同名键。同一个文件里出现同名键时（例如
//! 页面 json 的 `rendererOptions` 顶层与 `rendererOptions.skyline` 都写了
//! `defaultDisplayBlock`），位置会落在源码里靠前的那个——诊断文案已经带上了完整
//! 路径，位置只用于指路。

/// 一段 JSON 原文的偏移 → 行列换算器。
pub struct JsonPositions<'a> {
    source: &'a str,
    /// 每行起始字节偏移；第 0 项恒为 0。
    line_starts: Vec<usize>,
}

impl<'a> JsonPositions<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        line_starts.extend(source.match_indices('\n').map(|(offset, _)| offset + 1));
        Self {
            source,
            line_starts,
        }
    }

    /// 第一个 `"key"` 的键位置（开引号处），1-based `(line, column)`。
    pub fn key(&self, key: &str) -> Option<(u32, u32)> {
        self.key_offset(key).map(|offset| self.position(offset))
    }

    /// 从 `key` 的位置往后找第一个非空白字符（用于指向值），1-based `(line, column)`。
    pub fn value_after(&self, key: &str) -> Option<(u32, u32)> {
        let key_end = self.string_end(self.key_offset(key)?)?;
        let colon = self.source[key_end + 1..].find(':')? + key_end + 1;
        let bytes = self.source.as_bytes();
        let value = colon
            + 1
            + bytes[colon + 1..]
                .iter()
                .take_while(|b| b.is_ascii_whitespace())
                .count();
        (value < bytes.len()).then(|| self.position(value))
    }

    /// 键名开引号的字节偏移。字符串内部的同名文本不会被命中：只有紧跟着 `:` 的
    /// 字符串字面量才算键。
    fn key_offset(&self, key: &str) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut from = 0;
        while let Some(quote) = self.source[from..].find('"').map(|at| from + at) {
            let end = self.string_end(quote)?;
            if &self.source[quote + 1..end] == key {
                let mut next = end + 1;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if bytes.get(next) == Some(&b':') {
                    return Some(quote);
                }
            }
            from = end + 1;
        }
        None
    }

    /// 从开引号起找字符串字面量的收引号下标（跳过 `\"` 这类转义）。
    fn string_end(&self, quote: usize) -> Option<usize> {
        let bytes = self.source.as_bytes();
        let mut at = quote + 1;
        while at < bytes.len() {
            match bytes[at] {
                b'\\' => at += 2,
                b'"' => return Some(at),
                _ => at += 1,
            }
        }
        None
    }

    /// 字节偏移 → 1-based 行列。偏移越界时钳到文末。
    fn position(&self, offset: usize) -> (u32, u32) {
        let offset = offset.min(self.source.len());
        let line = self
            .line_starts
            .partition_point(|&start| start <= offset)
            .saturating_sub(1);
        let column = self.source[self.line_starts[line]..offset].chars().count() as u32 + 1;
        (line as u32 + 1, column)
    }
}

#[cfg(test)]
mod tests {
    use super::JsonPositions;

    #[test]
    fn locates_key_and_value() {
        let positions = JsonPositions::new("{\n  \"a\": 1,\n  \"b\": \"x\"\n}");
        assert_eq!(positions.key("a"), Some((2, 3)));
        assert_eq!(positions.value_after("a"), Some((2, 8)));
        assert_eq!(positions.key("b"), Some((3, 3)));
        assert_eq!(positions.value_after("b"), Some((3, 8)));
    }

    #[test]
    fn ignores_key_text_inside_a_string_value() {
        let positions = JsonPositions::new(r#"{"desc": "libVersion", "other": 1}"#);
        assert_eq!(positions.key("libVersion"), None);
        assert_eq!(positions.key("desc"), Some((1, 2)));
    }

    #[test]
    fn duplicate_keys_take_the_first() {
        let positions = JsonPositions::new(r#"{"outer": {"k": 1}, "k": 2}"#);
        assert_eq!(positions.key("k"), Some((1, 12)));
        assert_eq!(positions.value_after("k"), Some((1, 17)));
    }

    #[test]
    fn missing_key_is_none() {
        let positions = JsonPositions::new(r#"{"a": 1}"#);
        assert_eq!(positions.key("b"), None);
        assert_eq!(positions.value_after("b"), None);
    }

    #[test]
    fn columns_count_unicode_chars() {
        let positions = JsonPositions::new(r#"{"名字": "x", "k": 1}"#);
        assert_eq!(positions.key("k"), Some((1, 13)));
        assert_eq!(positions.value_after("k"), Some((1, 18)));
    }
}
