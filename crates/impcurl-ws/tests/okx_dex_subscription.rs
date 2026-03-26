use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use impcurl_ws::{Message, WsConnection};
use serde_json::Value;
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

const IMPCURL_RUN_PUBLIC_WS_TESTS: &str = "IMPCURL_RUN_PUBLIC_WS_TESTS";
const IMPCURL_OKX_WS_LISTEN_SECS: &str = "IMPCURL_OKX_WS_LISTEN_SECS";
const OKX_DEX_PUBLIC_WS_URL: &str = "wss://wsdexpri.okx.com/ws/v5/ipublic";
const EXPECTED_CHANNEL: &str = "dex-token-candle5s";
const EXPECTED_CHAIN_ID: u64 = 501;
const EXPECTED_TOKEN_ADDRESSES: [&str; 4] = [
    "AVF9F4C4j8b1Kh4BmNHqybDaHgnZpJ7W7yLvL7hUpump",
    "NV2RYH954cTJ3ckFUpvfqaQXU4ARqqDH3562nFSpump",
    "BKQucpTXB2d67jSNXMznSTj2iNLVtyga9JW86QoWpump",
    "9HrdTe7wh6qrqQnVGHGKeUvvumiwDAj1B2vsrc7TmwqY",
];
const DEFAULT_LISTEN_SECS: u64 = 8;
const SUBSCRIBE_PAYLOAD: &str = r#"{
    "op": "subscribe",
    "args": [
        {
            "channel": "dex-token-candle5s",
            "chainId": 501,
            "tokenAddress": "AVF9F4C4j8b1Kh4BmNHqybDaHgnZpJ7W7yLvL7hUpump"
        },
        {
            "channel": "dex-token-candle5s",
            "chainId": 501,
            "tokenAddress": "NV2RYH954cTJ3ckFUpvfqaQXU4ARqqDH3562nFSpump"
        },
        {
            "channel": "dex-token-candle5s",
            "chainId": 501,
            "tokenAddress": "BKQucpTXB2d67jSNXMznSTj2iNLVtyga9JW86QoWpump"
        },
        {
            "channel": "dex-token-candle5s",
            "chainId": 501,
            "tokenAddress": "9HrdTe7wh6qrqQnVGHGKeUvvumiwDAj1B2vsrc7TmwqY"
        }
    ]
}"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn okx_dex_candle5s_subscription_receives_ack_or_data() -> Result<()> {
    if std::env::var(IMPCURL_RUN_PUBLIC_WS_TESTS).ok().as_deref() != Some("1") {
        eprintln!(
            "skipping OKX websocket integration test; set {IMPCURL_RUN_PUBLIC_WS_TESTS}=1 to enable"
        );
        return Ok(());
    }

    let _ = impcurl_sys::resolve_impersonate_lib_path(&[]).with_context(|| {
        "failed to resolve libcurl-impersonate; set CURL_IMPERSONATE_LIB to a valid shared library"
    })?;

    let mut client = WsConnection::builder(OKX_DEX_PUBLIC_WS_URL)
        .connect_timeout(Duration::from_secs(15))
        .connect()
        .await
        .context("failed to connect to OKX dex public websocket")?;

    client
        .send(Message::Text(SUBSCRIBE_PAYLOAD.to_owned()))
        .await
        .context("failed to send OKX dex subscribe payload")?;

    let listen_window = parse_listen_window()?;
    eprintln!(
        "listening for OKX websocket frames for {}s (override with {IMPCURL_OKX_WS_LISTEN_SECS})",
        listen_window.as_secs()
    );
    let observed = observe_okx_messages(&mut client, listen_window).await?;

    if observed.matching_tokens.is_empty() {
        bail!(
            "did not receive any frame matching channel {EXPECTED_CHANNEL} within {}s",
            listen_window.as_secs()
        );
    }

    let mut missing_matching = Vec::new();
    let mut missing_ack_or_data = Vec::new();
    for token in EXPECTED_TOKEN_ADDRESSES {
        if !observed.matching_tokens.contains(token) {
            missing_matching.push(token);
        }
        if !observed.subscribe_tokens.contains(token) && !observed.data_tokens.contains(token) {
            missing_ack_or_data.push(token);
        }
    }

    if !missing_matching.is_empty() {
        bail!("missing matching frames for tokens: {missing_matching:?}; observed={observed:?}");
    }
    if !missing_ack_or_data.is_empty() {
        bail!("missing subscribe/data for tokens: {missing_ack_or_data:?}; observed={observed:?}");
    }
    eprintln!("okx ws summary: {observed:?}");

    Ok(())
}

fn parse_listen_window() -> Result<Duration> {
    let secs = std::env::var(IMPCURL_OKX_WS_LISTEN_SECS)
        .ok()
        .map(|raw| raw.parse::<u64>())
        .transpose()
        .with_context(|| format!("{IMPCURL_OKX_WS_LISTEN_SECS} must be a positive integer"))?
        .unwrap_or(DEFAULT_LISTEN_SECS);
    if secs == 0 {
        bail!("{IMPCURL_OKX_WS_LISTEN_SECS} must be >= 1");
    }
    Ok(Duration::from_secs(secs))
}

#[derive(Debug, Default)]
struct ObservedFrames {
    total_frames: usize,
    matching_tokens: BTreeSet<String>,
    subscribe_tokens: BTreeSet<String>,
    data_tokens: BTreeSet<String>,
}

async fn observe_okx_messages(
    client: &mut WsConnection,
    listen_window: Duration,
) -> Result<ObservedFrames> {
    let deadline = Instant::now() + listen_window;
    let mut observed = ObservedFrames::default();

    loop {
        let now = Instant::now();
        if now >= deadline {
            return Ok(observed);
        }

        let wait = (deadline - now).min(Duration::from_secs(5));
        let Some(message) = (match tokio::time::timeout(wait, client.next()).await {
            Ok(item) => item,
            Err(_) => continue,
        }) else {
            continue;
        };

        let message = message.context("websocket stream returned error")?;
        let frame = match message {
            Message::Text(text) => text.into_bytes(),
            Message::Binary(bytes) => bytes.to_vec(),
            Message::Ping(_) | Message::Pong(_) | Message::Close(_) => continue,
        };

        observed.total_frames += 1;
        eprintln!("okx ws frame: {}", String::from_utf8_lossy(&frame));

        let Ok(value) = serde_json::from_slice::<Value>(&frame) else {
            continue;
        };
        if let Some(event) = value.get("event").and_then(Value::as_str) {
            if event.eq_ignore_ascii_case("error") {
                bail!("OKX websocket returned error event: {value}");
            }
        }

        let Some(arg) = value.get("arg") else {
            continue;
        };
        let Some(token) = parse_matching_token_address(arg) else {
            continue;
        };
        if !EXPECTED_TOKEN_ADDRESSES.contains(&token) {
            continue;
        }

        observed.matching_tokens.insert(token.to_owned());
        if value
            .get("event")
            .and_then(Value::as_str)
            .map(|event| event.eq_ignore_ascii_case("subscribe"))
            .unwrap_or(false)
        {
            observed.subscribe_tokens.insert(token.to_owned());
        }
        if value
            .get("data")
            .and_then(Value::as_array)
            .map(|entries| !entries.is_empty())
            .unwrap_or(false)
        {
            observed.data_tokens.insert(token.to_owned());
        }
    }
}

fn parse_matching_token_address(arg: &Value) -> Option<&str> {
    let channel = arg.get("channel").and_then(Value::as_str);
    if channel != Some(EXPECTED_CHANNEL) {
        return None;
    }

    let Some(chain_field) = arg.get("chainId").or_else(|| arg.get("chainIndex")) else {
        return None;
    };
    let chain_matches = match chain_field {
        Value::Number(raw) => raw.as_u64() == Some(EXPECTED_CHAIN_ID),
        Value::String(raw) => raw.parse::<u64>().ok() == Some(EXPECTED_CHAIN_ID),
        _ => false,
    };
    if !chain_matches {
        return None;
    }

    arg.get("tokenAddress")
        .or_else(|| arg.get("tokenContractAddress"))
        .and_then(Value::as_str)
}
