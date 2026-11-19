//! 与渲染器无关的工程级规则。
//!
//! 这里的判据只说「小程序本身的写法有问题」（组件约定、资源引用一类），迁移到
//! Skyline 只是让它更容易暴露出来，WebView 下同样成立。

pub mod transparent_background;

pub use transparent_background::TransparentBackground;
