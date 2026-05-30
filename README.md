# Apache Doris CLI

A fast, scriptable CLI for the **Apache Doris kernel**. It connects over the
MySQL protocol (+ the FE HTTP API), executes, and returns **structured JSON** — a
"tool, not a brain": all the intelligence lives in the layer above (an agent, a
script, your shell). Every command prints machine-readable output and exits; there
are no interactive prompts.

The binary is `doriscli`.

```bash
doriscli sql "SELECT VERSION()"
doriscli profile get <query_id>
doriscli tablet db.orders --detail
```

---

## Features

- **SQL execution** — run a query (or a `.sql` file) and get back `query_id`,
  `exec_time_ms`, `columns`, and `rows` as JSON. Set session variables, toggle the
  query cache, or enable profiling inline.
- **Query profile analysis** — fetch and parse Doris query profiles:
  - a digest by default (summary + plan + operators + table context),
  - the full parsed tree (`Fragment → Pipeline → Operator` with all counters) with `--full`,
  - the raw profile text with `--raw`,
  - **diff** two runs (slow vs. fast) to find the regression,
  - **history** for a SQL pattern from `audit_log`,
  - **list** recent or currently-running queries.
- **Tablet & bucket analysis** — bucket/tablet distribution for a table, with a
  `--detail` mode that breaks down per-partition stats, per-tablet rows/size, and the
  backend mapping (find skew fast).
- **Multi-environment auth** — save several connections; `auth status` probes the
  connection and reports the Doris version, backends, and workload groups.
- **SOCKS5 tunneling** — reach a BYOC / bastioned cluster through a SOCKS5 proxy.
  Proxy credentials are never written to disk.
- **Flexible output** — JSON when piped, a pretty table on a TTY, or CSV. Override
  with `--format`.
- **Stateless mode** — drive it entirely from environment variables (no files
  touched), which is ideal for bastions, CI, and multi-tenant hosts.

---

## Install

The recommended way to install doris-cli is from **npm** — it ships prebuilt
binaries, so there is no Rust toolchain and no compile step:

```bash
npm install -g @apache-doris/doriscli
doriscli --version
```

On install, npm pulls **only** the binary that matches your OS + CPU (via
`optionalDependencies` + `os`/`cpu` constraints). Supported platforms: macOS
(arm64), Linux (x64, arm64). The published package is scoped
(`@apache-doris/doriscli`), but the installed command is just `doriscli`.

For any other platform — or to hack on doris-cli itself — build from source below.

---

## Build

Requires a recent stable Rust toolchain (verified on rustc 1.87; the crate uses the
MSRV-aware resolver and pins a few dependencies for older toolchains).

```bash
cargo build --release          # produces target/release/doriscli
cargo run --release -- --version
cargo test                     # unit tests
```

Put the binary on your `PATH`:

```bash
cp target/release/doriscli /usr/local/bin/
```

---

## Quick start

```bash
# 1. Save a connection ("prod" is just a name you choose)
doriscli auth add prod --host 127.0.0.1 --port 9030 --http-port 8030 \
  --user root --password 'secret'

# 2. Verify it (version, backends, workload groups, HTTP health)
doriscli --env prod auth status

# 3. Query
doriscli --env prod sql "SELECT COUNT(*) FROM db.orders"
```

The first environment you add becomes the default, so `--env` is optional until you
have more than one. Switch the default any time with `doriscli use <name>`.

---

## Commands

### `auth` — manage connections

```bash
doriscli auth add <name> --host <h> --user <u> --password <p> [--port 9030] [--http-port 8030]
doriscli auth add <name> --mysql "mysql://user:pass@host:9030"   # URI form
doriscli auth list
doriscli auth status            # test connection + cluster info
doriscli auth remove <name>
```

`auth add` probes the FE HTTP port and, if it doesn't answer, suggests common
alternatives (8080 for cloud FEs, 8030/8040 for self-hosted Apache Doris) so `profile`
commands work later.

### `sql` — execute queries

```bash
doriscli sql "SELECT * FROM db.t LIMIT 10"
doriscli sql -f migration.sql
doriscli sql "SELECT ..." --profile            # SET enable_profile=true first
doriscli sql "SELECT ..." --no-cache           # bypass the SQL cache (benchmarking)
doriscli sql "SELECT ..." --set "exec_mem_limit=8g" --set "parallel_pipeline_task_num=8"
```

Output:

```json
{
  "query_id": "f1e2...",
  "exec_time_ms": 42,
  "rows_returned": 10,
  "columns": ["id", "name"],
  "rows": [{ "id": 1, "name": "a" }]
}
```

### `profile` — analyze query profiles

```bash
doriscli profile list [--limit 20] [--active]      # recent, or running queries
doriscli profile get <query_id>                    # summary + plan + operators
doriscli profile get <query_id> --full             # full Fragment→Pipeline→Operator tree
doriscli profile get <query_id> --raw              # raw profile text
doriscli profile get <query_id> -f exported.txt    # parse a profile saved from the web UI
doriscli profile diff <slow_qid> <fast_qid>        # compare two runs
doriscli profile history "<sql substring>" [--days 7] [--limit 50]
```

### `tablet` — bucket / tablet distribution

```bash
doriscli tablet db.orders                          # overview
doriscli tablet db.orders --detail                 # per-partition + per-tablet + backends
doriscli tablet db.orders --detail --partition p20240101
```

### `use` — switch the default environment

```bash
doriscli use            # show current + list environments
doriscli use staging    # make "staging" the default
```

---

## Global options

| Option | Description |
|---|---|
| `--env <name>` | Which saved environment to use (default: `default`, or `$DORIS_ENV`). |
| `--format <json\|table\|csv>` | Force the output format. Default: table on a TTY, JSON when piped. |
| `--socks5 <user:pass@host:port>` | Route the connection through a SOCKS5 proxy (BYOC). |
| `--init-sql <sql>` | Run a statement right after connecting (e.g. `USE @<compute-group>`). Overrides `$DORIS_INIT_SQL`. |

---

## Configuration & environment variables

Saved environments live in `~/.doris/`:

- `~/.doris/config.toml` — host, ports, user, per-environment.
- `~/.doris/credentials.toml` — passwords (written `0600`).

Every setting can also come from the environment (prefix `DORIS_`), which takes
precedence over the config files:

| Variable | Meaning | Default |
|---|---|---|
| `DORIS_HOST` | FE host | — |
| `DORIS_USER` | MySQL user | — |
| `DORIS_PASSWORD` | MySQL password | empty |
| `DORIS_PORT` | MySQL/query port | `9030` |
| `DORIS_HTTP_PORT` | FE HTTP port | `8030` |
| `DORIS_ENV` | Which environment to use | `default` |
| `DORIS_INIT_SQL` | Statement run after connect | — |
| `DORIS_SOCKS5_HOST` / `_PORT` / `_USER` / `_PASS` | SOCKS5 proxy (BYOC) | user/pass default to `admin` |

**Stateless mode:** when both `DORIS_HOST` and `DORIS_USER` are set, doris-cli
connects purely from environment variables and never reads or writes any file —
designed for bastions, CI, and multi-tenant hosts where `$HOME` may not be writable.

```bash
DORIS_HOST=fe.internal DORIS_USER=admin DORIS_PASSWORD=*** \
  doriscli sql "SELECT 1"
```

---

## Connecting to a cloud-hosted kernel

doris-cli talks to any reachable Doris FE — including a cloud cluster — as long as
you give it the resolved host/port. Compute-group routing is handled
by `--init-sql` (or `DORIS_INIT_SQL`), which runs `USE @<compute-group>` right after
connecting:

```bash
DORIS_HOST=<resolved-host> DORIS_PORT=<port> DORIS_USER=admin DORIS_PASSWORD=*** \
DORIS_INIT_SQL='USE @my_compute_group' \
  doriscli sql "SELECT 1"
```

---

## Output formats

- **TTY** → a pretty table.
- **Piped / redirected** → JSON (so `| jq` and scripts just work).
- Force either, or CSV, with `--format json|table|csv`.

```bash
doriscli --env prod sql "SHOW BACKENDS" --format table
doriscli --env prod sql "SELECT * FROM db.t" | jq '.rows[] | .id'
```
