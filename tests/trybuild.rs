//! 编译失败集成测试：设备树的「错误在编译期拦截」承诺。
//!
//! 用例文件在 `tests/ui/`，预期诊断在 `tests/ui/*.stderr`。
//! rustc 诊断格式跨版本可能微调，需要更新基线时：
//!
//! ```sh
//! TRYBUILD=overwrite cargo test -p embassy-dt --test trybuild
//! ```

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
