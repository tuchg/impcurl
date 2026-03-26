# impcurl

[![impcurl-sys](https://img.shields.io/crates/v/impcurl-sys?label=impcurl-sys)](https://crates.io/crates/impcurl-sys)
[![impcurl](https://img.shields.io/crates/v/impcurl?label=impcurl)](https://crates.io/crates/impcurl)
[![impcurl-ws](https://img.shields.io/crates/v/impcurl-ws?label=impcurl-ws)](https://crates.io/crates/impcurl-ws)
[![MIT](https://img.shields.io/crates/l/impcurl)](LICENSE)

Rust WebSocket client with TLS fingerprint impersonation, powered by [libcurl-impersonate](https://github.com/lexiforest/curl-impersonate).

Bypass TLS fingerprinting by impersonating real browser signatures (Chrome, Safari, Firefox, Edge, Tor).

## Crates

| Crate | Description |
|-------|-------------|
| `impcurl-sys` | Dynamic FFI bindings for `libcurl-impersonate` with auto-fetch |
| `impcurl` | Safe blocking wrapper — WebSocket handshake, send, recv |
| `impcurl-ws` | Async tokio `Stream + Sink` WebSocket connection |

## Quick Start

```toml
[dependencies]
impcurl-ws = "1.1.0"
futures-util = "0.3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }
```

```rust
use futures_util::{SinkExt, StreamExt};
use impcurl_ws::{Message, WsConnection};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut ws = WsConnection::connect("wss://echo.websocket.org").await?;

    ws.send(Message::Text("hello".to_owned())).await?;

    if let Some(message) = ws.next().await.transpose()? {
        match message {
            Message::Text(text) => println!("{text}"),
            Message::Binary(bytes) => println!("{} bytes", bytes.len()),
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) => {}
        }
    }

    Ok(())
}
```

## Builder API

```rust
use impcurl::ImpersonateTarget;
use impcurl_ws::{ControlFrameMode, WsConnection};
use std::time::Duration;

let ws = WsConnection::builder("wss://example.com/ws")
    .header("Origin", "https://example.com")
    .header("User-Agent", "Mozilla/5.0 ...")
    .proxy("socks5h://127.0.0.1:1080")
    .impersonate(ImpersonateTarget::Chrome136)
    .connect_timeout(Duration::from_secs(10))
    .control_frame_mode(ControlFrameMode::Manual)
    .read_buffer_messages(32)
    .write_buffer_messages(32)
    .verbose(true)
    .connect()
    .await?;
```

## Runtime Library

The `libcurl-impersonate` shared library is resolved at runtime in this order:

1. `CURL_IMPERSONATE_LIB` env var
2. Near executable (`../lib/` and side-by-side)
3. `IMPCURL_LIB_DIR` env var
4. `~/.impcurl/lib`, `~/.cuimp/binaries`
5. Auto-fetch from versioned runtime assets on this repo's GitHub Releases (enabled by default)
6. Fallback auto-fetch from [curl_cffi](https://github.com/lexiforest/curl_cffi) wheel

`impcurl-ws` does not expose a `lib_path(...)` builder escape hatch anymore. Runtime library resolution is treated as deployment/runtime configuration rather than a connection-level concern.

## TLS CA Bundle (Linux)

`impcurl` now auto-resolves a CA bundle and applies `CURLOPT_CAINFO` during websocket setup.
Resolution order:

1. `CURL_CA_BUNDLE`
2. `SSL_CERT_FILE`
3. Platform defaults (Linux: `/etc/ssl/certs/ca-certificates.crt`, `/etc/pki/tls/certs/ca-bundle.crt`, ...)

This removes the need for app-level distro-specific CA symlink hacks in most Linux deployments.

### Auto-fetch Controls

| Env Var | Description |
|---------|-------------|
| `IMPCURL_AUTO_FETCH=0` | Disable auto-download |
| `IMPCURL_RUNTIME_VERSION` | Runtime asset version (default current crate version) |
| `IMPCURL_RUNTIME_REPO` | GitHub repo for runtime assets (default `tuchg/impcurl`) |
| `IMPCURL_CURL_CFFI_VERSION` | curl_cffi release tag (default `0.11.3`) |
| `IMPCURL_AUTO_FETCH_CACHE_DIR` | Override fetch cache directory |
| `IMPCURL_DISABLE_AUTO_CAINFO=1` | Disable automatic `CURLOPT_CAINFO` injection |

## Architecture

```
impcurl-ws (async tokio client)
  └── impcurl (safe blocking wrapper)
       └── impcurl-sys (dynamic FFI + auto-fetch)
            └── libcurl-impersonate (runtime .so/.dylib/.dll)
```

On Unix, the async event loop uses `CURLMOPT_SOCKETFUNCTION` / `CURLMOPT_TIMERFUNCTION` with `tokio::io::unix::AsyncFd` for efficient socket-level readiness notification. Non-Unix falls back to `curl_multi_poll`.

## License

MIT

## Runtime Asset Release

This repository includes a workflow that publishes versioned runtime assets:

- GitHub Actions workflow: `.github/workflows/release-runtime-assets.yml`
- Local packaging helper: `scripts/package_runtime_assets.sh`

Asset naming:

- `impcurl-runtime-v<version>-x86_64-unknown-linux-gnu.tar.gz`
- `impcurl-runtime-v<version>-aarch64-unknown-linux-gnu.tar.gz`
- `impcurl-runtime-v<version>-x86_64-unknown-linux-gnu.sha256`
- `impcurl-runtime-v<version>-aarch64-unknown-linux-gnu.sha256`

Note: `aarch64` publishing depends on an available ARM64 source image for
`libcurl-impersonate`. The workflow has an explicit ARM64 image input so the
asset can be enabled once that source is available.
