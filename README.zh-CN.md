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
# 从 crates.io 安装（把 `wanted` 装到系统的 PATH 上）
cargo install wanted

# 或从源码构建（二进制位于 target/release/wanted）
cargo build --release
```

## 快速开始

注册一份插件清单，然后安装它：

```bash
# 注册插件，让 `wanted install go` 认识它
# 传工具名会从默认仓库（GitHub wanted-registry）抓取 `<name>.toml`
wanted add golang
# 或者显式传一份本地清单路径
wanted add golang.toml

# 安装一个工具（可固定版本）
wanted install go@go1.27.0

# 列出插件各来源声明的可用版本（会标注 latest）
wanted versions go

# 裸名字安装时，latest 解析为声明的最新版本（见下）
wanted install go

# 安装时指定命名的资产来源（默认取插件的 `default` 来源）
wanted install go --asset-source go.dev

# 可选地在基础资产之上额外下载组件（若插件声明了组件）
wanted install <tool> --with <component>

# 列出已安装的工具
wanted list

# 卸载并还原环境变量
wanted uninstall go
```

首次安装会把应用安装到当前目录，并在旁边创建 `.wanted/` 记录目录存放安装收据。

## 插件清单

插件是 TOML 文件。上文使用的 `golang.toml` 的结构如下：

```toml
[meta]
name = "go"
version = "1.0.0"          # 插件版本，不是工具版本
url = "https://golang.org" # 主页

[install]
method = "download"        # download / installer / command —— system 尚未接入
base_dir = "golang"        # 应用所在的目录（位于运行目录之下）

[install.asset]
"x86_64-pc-windows-msvc" = {
  default = "https://golang.google.cn/dl/go{version}.windows-amd64.zip",
  go.dev  = "https://go.dev/dl/go{version}.windows-amd64.zip",
}
"aarch64-apple-darwin" = {
  default = "https://golang.google.cn/dl/go{version}.darwin-arm64.tar.gz",
  go.dev  = "https://go.dev/dl/go{version}.darwin-arm64.tar.gz",
}

[install.versions]                # 每个来源的可选版本列表
default = ["1.22.5", "1.23.4"]    # 列出真实版本号，不要带前缀
"go.dev" = ["1.22.5", "1.23.4"]

[env]
PATH   = "bin"          # 前置到 PATH
GOROOT = "."
GOPATH = "{user}/go"
GOBIN  = "$GOPATH/bin"
```

**资产（asset）** 将平台三元组（`<arch>-<vendor>-<os>` 风格）映射到一个或多个命名来源。`{version}` 会被替换为你要求安装的版本；`--asset-source <name>` 选择来源（默认 `default`）。

**版本（versions）**。`install` 内可选的 `[install.versions]` 表按来源名（`default` 以及 `asset` 里用到的其它来源）列出可用版本。裸的 `wanted install <tool>`（不带 `@version`）会把 `latest` 解析为所选来源声明的**最新**版本——用真实版本号替换 `{version}`，而不是字面量 `latest`。`wanted versions <tool>` 打印这些列表（`--source <name>` 只显示某来源），并把最新版标注为 `latest`。每个条目必须是可比较的规范 semver（如 `1.23.4`；`go` 这类前缀请写进 URL 模板 `go{version}`）；出现不可比较的条目会报错，而不是静默选错版本。未声明 `versions` 的来源保持旧行为：`latest` 仍被字面替换（适合自己处理 URL 魔力的高级用户）。

**组件（component）** 以与资产完全相同的「平台 → 来源 → URL」结构声明可选附加项。默认不会下载；传入可重复的 `--with <name>` 标志才抓取，并解压到基础资产目录之下（`apps/<base_dir>/<name>`），与核心隔离。环境变量条目可用 `../<name>/...` 形式的相对路径（相对 `<base_dir>` 解析）指向组件目录。

**环境变量（env）** 条目会展开 `{version}`、`{user}`（家目录）以及对先前定义过变量的 `$VAR` 引用。相对路径相对安装目录解析。`PATH` 默认前置追加，或通过 `install.env_box` 改为后置。

**安装方式（method）**：`method = "download"`（默认）直接解压归档；`method = "installer"` 则下载可执行文件并作为**静默安装器**运行、装到 `<base_dir>`，适用于以 `.exe` 发行的工具（如 Windows 上的 LLVM）。静默参数放在 `args` 中，可用 `{base}`（即 `<base_dir>` 目录）、`{version}`、`{user}` 占位：

```toml
[install]
method = "installer"
base_dir = "llvm"
asset = { "x86_64-pc-windows-msvc" = { default = "https://example/LLVM-{version}-win64.exe" } }
args = ["/VERYSILENT", "/NORESTART", "/DIR={base}"]
```

有的工具在部分平台发行归档、在另一些平台发行安装器。用 `install.strategy` 按平台三元组指定各自的方式与参数；未列出的平台回退到 install 级的 `method`/`args`：

```toml
[install]
method = "download"
base_dir = "llvm"
asset = {
  "aarch64-apple-darwin" = { default = "...tar.xz" },
  "x86_64-pc-windows-msvc" = { default = "...LLVM-{version}-win64.exe" },
}

[install.strategy]
"x86_64-pc-windows-msvc" = { method = "installer", args = ["/VERYSILENT", "/DIR={base}"] }
```

有些工具不以归档或安装器分发，而是由包管理器拉取（`cargo install`、`npm i -g`、`pip install`…）。用 `method = "command"` 按**回退顺序**运行一条或多条外部命令：某个工具不在 `PATH`、或命令返回非零退出码时，就尝试下一条命令。每条命令写入 `<base_dir>`（用 `--root {base}` / `--prefix {base}` 这类参数，或 `CARGO_INSTALL_ROOT` 这类环境变量引导到该目录）。`install.command` 与 `asset` 一样按平台三元组键控：

```toml
[install]
method = "command"
base_dir = "rust"

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "cargo", args = ["install", "--root", "{base}", "bat@1.0.0"], env = { CARGO_INSTALL_ROOT = "{base}" } },
  { tool = "npm",   args = ["install", "--prefix", "{base}", "--global", "bat"] },
]
```

`tool` 是可执行文件名（经 `PATH` 解析）。`args` 与每条命令的 `env` 映射的值会展开 `{base}`（即 `<base_dir>` 目录）、`{version}`、`{user}` 占位。当所有命令都失败时，安装会汇总各错误并中止——**不会删除** `<base_dir>` 中上一份安装。此方式不支持组件，也不需要 `asset`。

**回退链（Fallback chain）**。`method` 只选一种机制。若想按顺序多试几种——先
试系统包管理器，最后才回退到链接下载——就在 `install.fallback` 里声明额外
的 method，只有当主 `method` 失败后才依次尝试。执行顺序为：

```
[主 method] → fallback[0] → fallback[1] → …
```

**首次成功即停**；只有当**所有**尝试都失败时 `wanted install` 才报错。每个回退
条目复用本区已声明的数据（download/installer 用 `install.asset`，command 用
`install.command`），无需重复声明。某 method 在当台平台上数据不可用（如 Linux
没有 command 条目、macOS 没有 asset 条目）时，会跳过该尝试而不是中断整个链：

```toml
[install]
method = "command"           # 1) 先试用户机器上的包管理器
base_dir = "golang"
fallback = ["download"]      # 2) 命令都失败时回退到链接下载

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "winget", args = ["install", "GoLang.Go", "-h", "--override", "/TARGETDIR={base}"] },
]

[install.asset]
"x86_64-pc-windows-msvc" = { default = "https://go.dev/dl/go{version}.windows-amd64.zip" }
```

由于 wanted 把 `apps/<base_dir>/bin` 前置到 `PATH`，托管的副本总会遮住系统已装
的东西——「安装并覆盖」才是我们想要的语义，所以没有「已装就跳过」这类开关。

`install.command` 里只放能把工具装进 wanted 托管目录 `{base}` 的包管理器；像
apt、brew 这类装到系统位置的管理器就不写进去，这类平台会直接落到下载尝试。

## CLI 参考

| 命令                           | 别名 | 说明                                              |
| ------------------------------ | ---- | ------------------------------------------------- |
| `wanted add <name\|plugin.toml>` | `a`  | 注册插件清单并使其可安装（传工具名会从默认仓库抓取 `<name>.toml`，可用 `--registry <base>` 覆盖） |
| `wanted install <spec>...`     | `i`  | 安装工具（`name@version`）；裸 `name` 把 `latest` 解析为声明的最新版本；支持 `--source`、`--asset-source`、`--with <component>` |
| `wanted versions <name>`       | `avail` | 列出插件各来源声明的可用版本（标注 `latest`）；`--source <name>` 只显示某来源 |
| `wanted remove <name>`         | `rm` | 移除已注册的插件清单                                |
| `wanted uninstall <name>`      | `un` | 卸载工具并还原其环境变量                             |
| `wanted upgrade`               | —    | 从最新 GitHub release 升级 `wanted` 自身（校验和验证 + 可回滚的热替换） |
| `wanted list`                  | `ls` | 列出已安装的工具                                    |
| `wanted env`                   | `use` | 把 wanted 所在目录加入 PATH，使 `wanted` 可直接调用（Windows 写注册表；POSIX 写 `~/.wanted/env.sh`） |

## 目录布局

```
<project>/                   # 运行 `wanted install` 的目录
├── <base_dir>/              # 已安装应用（直接位于运行目录下）
└── .wanted/                 # 记录目录，首次安装时创建
    ├── installed/<name>/    # 各工具的安装收据（环境变量快照）
    └── .staging/            # 临时下载/解压区（安装后被清理）
```

## 状态

核心管线（规划 → 下载 → 解压 → 提交 → 收据）已实现并通过测试。自升级（`upgrade`）已实现校验和验证与可回滚替换。

## 许可证

MIT。