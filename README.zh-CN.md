# Wanted

**README 语言：** [**English**](README.md) · <kbd>简体中文</kbd>

> 开发工具安装器。声明式清单、自动配置环境变量、安装收据、并且 **每一步可回滚**。

`wanted` 是一个插件驱动的开发工具安装器：每个工具以一个纯 TOML「插件」来描述如何获取（下载哪个归档、从哪里下载）、装到哪里、需要设置哪些环境变量。不依赖包管理器、没有平台守护进程 —— 只有一个自包含的二进制和一个目录。

## 特性

- **声明式插件** —— 工具由一份小型 TOML 清单描述：下载方式、资产 URL、安装布局、环境变量增量。不需要自定义构建脚本。
- **多来源资产** —— 插件可以提供多个上游（例如官方镜像 + 被墙的源），安装时按平台逐来源选择。
- **自动配置环境** —— `PATH` 前置/后置追加，其余变量（如 `GOROOT`、`GOPATH`）相对安装目录设置；支持展开 `{version}`、`{user}` 和 `$VAR` 引用。
- **事务式、可回滚的安装** —— 安装分为 *暂存* 操作（下载、解压）与 *提交* 操作（写环境变量）。任何一步失败都会回滚已完成的部分，损坏的安装不会留下烂摊子。
- **并行分段下载** —— 大归档用 HTTP `Range` 分段多连接抓取（服务器不支持时自动回退），并带实时字节数 spinner。
- **收据与干净卸载** —— 每次安装写入收据，快照环境变量旧值；`uninstall` 按收据还原环境并清除应用。
- **纯函数、可测试的核心** —— 规划是纯函数；所有 I/O 走小粒度接缝（`Fs`、`Downloader`、`EnvStore`、`Reporter`），整个引擎可在测试中于内存后端上重放。

## 安装

需要 Rust 工具链。

```bash
cargo build --release
# 二进制位于 target/release/wanted
```

## 快速开始

注册一份插件清单，然后安装它：

```bash
# 注册插件，让 `wanted install go` 认识它
wanted add golang.toml

# 安装一个工具（可固定版本）
wanted install go@go1.27.0

# 安装时指定命名的资产来源（默认取插件的 `default` 来源）
wanted install go --asset-source go.dev

# 列出已安装的工具
wanted list

# 卸载并还原环境变量
wanted uninstall go
```

首次安装会在当前目录创建 `.wanted/` 目录，存放已装应用与收据。

## 插件清单

插件是 TOML 文件。上文使用的 `golang.toml` 的结构如下：

```toml
[meta]
name = "go"
version = "1.0.0"          # 插件版本，不是工具版本
url = "https://golang.org" # 主页

[install]
method = "download"        # download（解压归档）—— system 尚未接入
base_dir = "golang"        # 应用所在的目录（位于 apps/ 之下）

[install.asset]
"x86_64-pc-windows-msvc" = {
  default = "https://golang.google.cn/dl/go{version}.windows-amd64.zip",
  go.dev  = "https://go.dev/dl/go{version}.windows-amd64.zip",
}
"aarch64-apple-darwin" = {
  default = "https://golang.google.cn/dl/go{version}.darwin-arm64.tar.gz",
  go.dev  = "https://go.dev/dl/go{version}.darwin-arm64.tar.gz",
}

[env]
PATH   = "bin"          # 前置到 PATH
GOROOT = "."
GOPATH = "{user}/go"
GOBIN  = "$GOPATH/bin"
```

**资产（asset）** 将平台三元组（`<arch>-<vendor>-<os>` 风格）映射到一个或多个命名来源。`{version}` 会被替换为你要求安装的版本；`--asset-source <name>` 选择来源（默认 `default`）。

**环境变量（env）** 条目会展开 `{version}`、`{user}`（家目录）以及对先前定义过变量的 `$VAR` 引用。相对路径相对安装目录解析。`PATH` 默认前置追加，或通过 `install.env_box` 改为后置。

## CLI 参考

| 命令                           | 别名 | 说明                                              |
| ------------------------------ | ---- | ------------------------------------------------- |
| `wanted add <plugin.toml>`     | `a`  | 注册插件清单，使工具可安装                          |
| `wanted install <spec>...`     | `i`  | 安装工具（`name@version`），支持 `--source`、`--asset-source` |
| `wanted update <tool\|plugin>` | `u`  | 更新已安装的工具或插件（*M0 占位*）                 |
| `wanted remove <name>`         | `rm` | 移除已注册的插件清单                                |
| `wanted uninstall <name>`      | `un` | 卸载工具并还原其环境变量                             |
| `wanted upgrade`               | —    | 升级 `wanted` 自身（*M0 占位*）                     |
| `wanted list`                  | `ls` | 列出已安装的工具                                    |

## 目录布局

```
<project>/
└── .wanted/                 # 首次安装时在当前目录创建
    ├── apps/<base_dir>/     # 已安装应用
    └── installed/<name>/    # 各工具的安装收据（环境变量快照）
```

## 状态

核心管线（规划 → 下载 → 解压 → 提交 → 收据）已实现并通过测试。`update` 与 `upgrade` 目前为占位。

## 许可证

MIT。