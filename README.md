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
# a tool name fetches `<name>.toml` from the default registry (GitHub wanted-registry)
wanted add golang
# or pass an existing local manifest path explicitly
wanted add golang.toml

# install a tool (optionally pinned to a version)
wanted install go@go1.27.0

# pick a named asset source at install time (defaults to the plugin's `default` source)
wanted install go --asset-source go.dev

# optionally download components on top of the base asset (if the plugin declares them)
wanted install <tool> --with <component>

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
method = "download"        # download / installer / command — system is not wired yet
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

**Components** declare optional add-ons with the exact same platform → source →
URL shape as assets. They are never downloaded by default; pass the repeatable
`--with <name>` flag to fetch one and unpack it under the base asset's directory
(`apps/<base_dir>/<name>`), separate from the core. Env entries address them with
`../<name>/...`-style relative paths (resolved from `apps/<base_dir>`).

**Env** entries expand `{version}`, `{user}` (home directory) and `$VAR`
references to variables defined earlier in the list. Relative paths resolve
against the install directory. `PATH` is prepended by default, or appended via
`install.env_box`.

**Install method.** `method = "download"` (default) unpacks a vendored archive.
`method = "installer"` instead downloads an executable and runs it as a silent
installer into `apps/<base_dir>`, for tools shipped as `.exe` (e.g. LLVM on
Windows). The silent flags live in `args` and may reference `{base}` (the
`apps/<base_dir>` directory), `{version}` and `{user}`:

```toml
[install]
method = "installer"
base_dir = "llvm"
asset = { "x86_64-pc-windows-msvc" = { default = "https://example/LLVM-{version}-win64.exe" } }
args = ["/VERYSILENT", "/NORESTART", "/DIR={base}"]
```

Some tools ship archives on some platforms but an installer on others. Use
`install.strategy` to pick the method (and args) per platform; anything not
listed falls back to the install-level `method`/`args`:

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

Some tools aren't distributed as archives or installers at all — they are pulled
in by a package manager (`cargo install`, `npm i -g`, `pip install`, …). Use
`method = "command"` to run one or more external commands in **fallback order**:
if a tool is missing from `PATH`, or a command exits non-zero, the next command is
tried. Each command writes into `apps/<base_dir>` (point it there via a flag like
`--root {base}` / `--prefix {base}`, or an env var like `CARGO_INSTALL_ROOT`).
`install.command` is keyed by platform triple, mirroring `asset`:

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

`tool` is the executable name (resolved via `PATH`). `args` and the values of the
per-command `env` map expand `{base}` (the `apps/<base_dir>` directory),
`{version}` and `{user}`. When every command fails, the install reports the
combined errors and aborts — a previous good install in `apps/<base_dir>` is
never deleted. This method supports no components and needs no `asset`.

**Fallback chain.** A `method` picks one mechanism. To try several in order —
a system package manager first, then a plain link download — declare
`install.fallback` with the extra methods, tried only after the primary
`method` fails. Execution order is:

```
[primary method] → fallback[0] → fallback[1] → …
```

The first attempt that succeeds wins; only when **every** attempt fails does
`wanted install` error. Each fallback entry reuses this section's already
declared data (`install.asset` for download/installer, `install.command` for
command), so nothing needs redeclaring. A method whose data is unavailable for
the current platform (e.g. no `command` entry for Linux, or no `asset` for a
macOS fallback) is skipped rather than aborting the chain:

```toml
[install]
method = "command"           # 1) try the user's package managers first
base_dir = "golang"
fallback = ["download"]      # 2) fall back to a link download if all commands fail

[install.command]
"x86_64-pc-windows-msvc" = [
  { tool = "winget", args = ["install", "GoLang.Go", "-h", "--override", "/TARGETDIR={base}"] },
]

[install.asset]
"x86_64-pc-windows-msvc" = { default = "https://go.dev/dl/go{version}.windows-amd64.zip" }
```

Because wanted prepends `apps/<base_dir>/bin` to `PATH`, the managed copy always
shadows anything already on the system — install-and-override is the intended
semantics, so there is no "skip if already installed" knob.

Only the package managers that install into wanted's managed `{base}` directory
belong in `install.command`; a manager that plants tools system-wide (apt, brew)
is left out, and that platform simply falls through to the download attempt.

## CLI reference

| Command                        | Alias | Description                                            |
| ------------------------------ | ----- | ------------------------------------------------------ |
| `wanted add <name\|plugin.toml>` | `a`   | Register a plugin manifest (a tool name fetches `<name>.toml` from the default registry; `--registry <base>` overrides it), making a tool installable |
| `wanted install <spec>...`     | `i`   | Install tools (`name@version`); `--source`, `--asset-source`, `--with <component>` |
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