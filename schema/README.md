# Schema

JSON Schema 定义 `wanted` 插件 TOML 清单的合法结构，用于编辑器的校验与补全。

- [`plugin.schema.json`](plugin.schema.json) — 插件清单（manifest）的 schema。

## 自动生成

`plugin.schema.json` 由 [`build.rs`](../build.rs) 在每次 `cargo build` 时自动
从 [`src/plugin/raw.rs`](../src/plugin/raw.rs) 中的解析类型重新生成——库的
`Manifest::parse` 与 schema 生成共用同一份源文件，因此 schema 永远与解析逻辑
一致，不会漂移。改动插件清单结构后重新 `cargo build` 即可得到最新 schema。

## 关联到编辑器

仓库内的示例插件（`golang.toml`、`llvm.toml`）顶部都有 `#:schema` 指令，
支持 taplo 的工具（如 VS Code「Even Better TOML」、taplo CLI 的 LSP）会自动
加载本 schema 做校验与自动补全。

输入 `wanted add <your-plugin>.toml` 前，在你自己的清单里加同类指令：

```toml
#:schema ./schema/plugin.schema.json

[meta]
name = "go"
# ...
```

若清单在其它目录，请用相对该文件的路径，或改用 taplo 的 `$SHELF`/项目级
`taplo.toml` schema 映射（见 taplo 文档）。注意 schema 只用于编辑期校验，
`wanted` 运行时不做 JSON Schema 校验，以代码中的强类型解析为准。

## 修改清单结构

需要增删字段、改枚举或必填时，改 [`src/plugin/raw.rs`](../src/plugin/raw.rs) 中
对应的 `serde` 类型（并同步 `src/plugin/mod.rs` 的领域模型与仓库内示例），然后
`cargo build` 让 build.rs 重新生成 schema。不要直接手改 `plugin.schema.json`——
它会在下次构建被覆盖。