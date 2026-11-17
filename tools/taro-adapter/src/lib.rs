//! Taro 适配器：把构建产物里的 WXML 换成 TSX 还原出来的结构，再交给 skylint。
//!
//! 分层：
//! - [`jsx`]：TSX/JSX → 渲染树（内置组件、`usingComponents` 组件、内联组件）；
//! - [`emit`]：渲染树 → WXML + 字节偏移映射；
//! - [`synth`]：合成工程（复制构建产物、替换 WXML）；
//! - [`remap`]：诊断从合成文件重定位回源文件。

pub mod emit;
pub mod jsx;
pub mod remap;
pub mod synth;
pub mod table;
