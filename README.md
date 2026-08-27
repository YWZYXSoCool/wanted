# Wanted

**README Language:** <kbd>English</kbd> · [**简体中文**](README.zh-CN.md)

> Development environment installer. Backed by declarative manifests, automatic
> environment-variable wiring, receipts, and **rollback on every step**.

`wanted` is a plugin-driven installer for development tools: a tool ships as a
plain TOML "plugin" declaring how to get it (which archive, from where), where to
install it, and what environment variables to set. No package-manager dependency,
no platform daemon — just a single self-contained binary and a directory.

## Features

- **Declarative plugins** — a tool is described by a small TOML manifest: download
  method, asset URLs, install layout, env deltas. No custom build scripts.
- **Multi-source assets** — a plugin can offer several upstreams (e.g. an official
  mirror plus a blocked origin) and pick one per host at install time.
- **Automatic environment setup** — `PATH` is prepended/appended, other variables
  (e.g. `GOROOT`, `GOPATH`) are set relative to the install; `{version}`, `{user}`
  and `$VAR` references are expanded.
- **Transactional, rollback-safe install** — install is split into *staged* ops
  (download, unpack) then *commit* ops (write env). A failure anywhere rolls the
  partial work back, so a broken install never leaves a mess behind.
- **Parallel range downloads** — large archives are fetched with HTTP `Range`
  segmentation across multiple connections (clamped when the server doesn't
  support it), with a live spinner showing bytes downloaded.
- **Receipts & clean uninstall** — each install writes a receipt snapshotting the
  previous environment values; `uninstall` restores them and removes the app.
- **Pure, testable core** — planning is a pure function; all I/O runs behind
  small seams (`Fs`, `Downloader`, `EnvStore`, `Reporter`) so the whole engine is
  replayed against in-memory backends in tests.

## Installation

Requires a Rust toolchain.

```bash
cargo build --release
# binary at target/release/wanted
```

## Quick start

Register a plugin manifest, then install it:

```bash
# register the plugin so `wanted install go` knows it
wanted add golang.toml

# install a tool (optionally pinned to a version)
wanted install go@go1.27.0

# pick a named asset source at install time (defaults to the plugin's `default` source)
wanted install go --asset-source go.dev

# list what's installed
wanted list

# uninstall and restore the environment
wanted uninstall go
```

The first install creates a `.wanted/` directory in the current directory holding
installed apps and receipts.

## Plugin manifest

A plugin is a TOML file. The schema used above (`golang.toml`):

```toml
[meta]
name = "go"
version = "1.0.0"          # plugin version, not the tool version
url = "https://golang.org" # homepage

[install]
method = "download"        # download (extract archive) — system is not wired yet
base_dir = "golang"        # directory inside/at which the app lives

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
PATH   = "bin"          # prepended to PATH
GOROOT = "."
GOPATH = "{user}/go"
GOBIN  = "$GOPATH/bin"
```

**Assets** map a platform triple (`<arch>-<vendor>-<os>` style) to one or more
named sources. `{version}` is substituted with the version you ask to install;
the `--asset-source <name>` flag selects the source (default: `default`).

**Env** entries expand `{version}`, `{user}` (home directory) and `$VAR`
references to variables defined earlier in the list. Relative paths resolve
against the install directory. `PATH` is prepended by default, or appended via
`install.env_box`.

## CLI reference

| Command                        | Alias | Description                                            |
| ------------------------------ | ----- | ------------------------------------------------------ |
| `wanted add <plugin.toml>`     | `a`   | Register a plugin manifest, making a tool installable  |
| `wanted install <spec>...`     | `i`   | Install tools (`name@version`), `--source`, `--asset-source` |
| `wanted update <tool\|plugin>` | `u`   | Update an installed tool or plugin (*M0 stub*)         |
| `wanted remove <name>`         | `rm`  | Remove a registered plugin manifest                    |
| `wanted uninstall <name>`      | `un`  | Uninstall a tool and restore its environment           |
| `wanted upgrade`               | —     | Upgrade `wanted` itself (*M0 stub*)                    |
| `wanted list`                  | `ls`  | List installed tools                                   |

## Layout

```
<project>/
└── .wanted/                 # created at first install in the current directory
    ├── apps/<base_dir>/     # installed applications
    └── installed/<name>/    # per-tool install receipts (env snapshots)
```

## Status

The core pipeline (plan → download → unpack → commit → receipt) is implemented
and tested. `update` and `upgrade` are currently placeholders.

## License

MIT.