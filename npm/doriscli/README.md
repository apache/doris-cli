# doriscli

A fast, scriptable CLI for the **Apache Doris kernel**. It connects over the MySQL
protocol (+ the FE HTTP API), executes, and returns **structured JSON**.

## Install

```bash
npm install -g doriscli
doriscli --version
```

This package ships a prebuilt native binary. On install, npm automatically pulls
**only** the platform package that matches your OS + CPU (via `optionalDependencies`
+ `os`/`cpu` constraints), so there is no compile step and no Rust toolchain needed.

Supported platforms: macOS (arm64, x64), Linux (x64, arm64), Windows (x64). On any
other platform, [build from source](https://github.com/morningman/doris-cli#build).

## Quick start

```bash
# Save a connection ("prod" is a name you choose)
doriscli auth add prod --host 127.0.0.1 --port 9030 --http-port 8030 --user root --password 'secret'

# Verify it (version, backends, workload groups)
doriscli --env prod auth status

# Query
doriscli --env prod sql "SELECT COUNT(*) FROM db.orders"
```

See the [full documentation](https://github.com/morningman/doris-cli#readme) for
`sql`, `profile`, `tablet`, `auth`, stateless mode, and SOCKS5 tunneling.

## License

[Apache-2.0](./LICENSE.txt)
