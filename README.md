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
# from crates.io (installs `wanted` onto your system's PATH)
cargo install wanted

# or build from source (binary at target/release/wanted)
cargo build --release
```

On Windows you can also install it with winget (new releases are published
automatically; the initial version was submitted manually):

```bash
winget install -e -i YWZYXSoCool.Wanted
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

# list the versions a plugin's sources declare (latest is marked)
wanted versions go

# a bare name resolves `latest` to the newest declared version (see below)
wanted install go

# pick a named asset source at install time (defaults to the plugin's `default` source)
wanted install go --asset-source go.dev

# optionally download components on top of the base asset (if the plugin declares them)
wanted install <tool> --with <component>

# list what's installed
wanted list

# uninstall and restore the environment
wanted uninstall go
```

The first install installs the app into the current directory and creates a
`.wanted/` record directory next to it holding install receipts.

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

[install.versions]                # available versions per source (optional)
default = ["1.22.5", "1.23.4"]    # list real versions, not prefixed ones
"go.dev" = ["1.22.5", "1.23.4"]

[env]
PATH   = "bin"          # prepended to PATH
GOROOT = "."
GOPATH = "{user}/go"
GOBIN  = "$GOPATH/bin"
```

**Assets** map a platform triple (`<arch>-<vendor>-<os>` style) to one or more
named sources. `{version}` is substituted with the version you ask to install;
the `--asset-source <name>` flag selects the source (default: `default`).

**Versions.** Inside `install`, the optional `[install.versions]` table lists the
available versions per source name (`default` and the others used in `asset`).
Installing a bare `wanted install <tool>` (no `@version`) resolves `latest` to
the **newest** version the selected source declares — the real version is
substituted into `{version}`, not the literal string `latest`. `wanted versions
<tool>` prints these lists (`--source <name>` to show one), marking the newest
as `latest`. Entries must be canonical, comparable SemVer (e.g. `1.23.4`, with
any prefix like `go` written into the URL template as `go{version}`); a
non-comparable entry errors rather than silently picking the wrong version. A
source without a `versions` entry keeps the old behaviour: `latest` is
substituted literally (advanced users who manage their own URL magic).

**Components** declare optional add-ons with the exact same platform → source →
URL shape as assets. They are never downloaded by default; pass the repeatable
`--with <name>` flag to fetch one and unpack it under the base asset's directory
(`apps/<base_dir>/<name>`), separate from the core. Env entries address them with
`../<name>/...`-style relative paths (resolved from `<base_dir>`).

**Env** entries expand `{version}`, `{user}` (home directory) and `$VAR`
references to variables defined earlier in the list. Relative paths resolve
against the install directory. `PATH` is prepended by default, or appended via
`install.env_box`.

**Install method.** `method = "download"` (default) unpacks a vendored archive.
`method = "installer"` instead downloads an executable and runs it as a silent
installer into `<base_dir>`, for tools shipped as `.exe` (e.g. LLVM on
Windows). The silent flags live in `args` and may reference `{base}` (the
`<base_dir>` directory), `{version}` and `{user}`:

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
tried. Each command writes into `<base_dir>` (point it there via a flag like
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
per-command `env` map expand `{base}` (the `<base_dir>` directory),
`{version}` and `{user}`. When every command fails, the install reports the
combined errors and aborts — a previous good install in `<base_dir>` is
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
| `wanted install <spec>...`     | `i`   | Install tools (`name@version`); bare `name` resolves `latest` to the newest declared version; `--source`, `--asset-source`, `--with <component>` |
| `wanted versions <name>`       | `avail` | List the versions a plugin's sources declare (marking `latest`); `--source <name>` shows one source |
| `wanted remove <name>`         | `rm`  | Remove a registered plugin manifest                    |
| `wanted uninstall <name>`      | `un`  | Uninstall a tool and restore its environment           |
| `wanted upgrade`               | —     | Upgrade `wanted` itself from the latest GitHub release (checksum-verified, rollback-safe swap) |
| `wanted list`                  | `ls`  | List installed tools                                   |
| `wanted env`                   | `use` | Add wanted's own directory to PATH so `wanted` is callable directly (registry on Windows; `~/.wanted/env.sh` on POSIX) |

## Layout

```
<project>/                   # the directory where `wanted install` runs
├── <base_dir>/              # installed applications (directly in the run directory)
└── .wanted/                 # record directory, created at first install
    ├── installed/<name>/    # per-tool install receipts (env snapshots)
    └── .staging/            # transient downloads/extraction (cleaned up)
```

## Status

The core pipeline (plan → download → unpack → commit → receipt) is implemented
and tested. Self-upgrade (`upgrade`) is checksum-verified and rollback-safe.

## License

MIT.