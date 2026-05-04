# HTTP retry

`RetryableHttpClient` and `secure_reqwest_client` for production HTTP in CLI commands. See [`skill/examples/http_retry_demo`](../examples/http_retry_demo/).

## `secure_reqwest_client`

Creates a `reqwest::Client` with production-safe defaults:
- TLS required; plaintext HTTP blocked
- Redirect policy: no cross-scheme redirects
- Timeout enforced

```rust
use cli_framework::http_retry::secure_reqwest_client;

let client = secure_reqwest_client()?;
let resp = client.get("https://api.example.com/health").send().await?;
```

## `RetryableHttpClient` construction

Wraps `reqwest::Client` with automatic retry and circuit-breaker logic:

```rust
use cli_framework::http_retry::{RetryableHttpClient, secure_reqwest_client};

let client = RetryableHttpClient::builder()
    .max_retries(3)
    .base_delay_ms(100)
    .build(secure_reqwest_client()?)?;
```

## Circuit breaker pattern

After a configurable number of consecutive failures, the client opens the circuit and returns errors immediately without attempting network calls. The circuit closes again after a cooldown period:

```rust
// Retries automatically on transient errors (5xx, timeout, connection reset)
let resp = client.get("https://api.example.com/data").send().await?;
// Returns Err immediately when circuit is open
```

## Error classification

| Category | Behavior |
|----------|---------|
| 5xx server errors | Retried up to `max_retries` |
| Network timeout | Retried |
| Connection reset / refused | Retried |
| 4xx client errors | Not retried (caller error) |
| 2xx / 3xx | Success; no retry |

## In `AppContext`

Store the client on context to share across commands:

```rust
struct AppCtx {
    http: RetryableHttpClient,
}
impl AppContext for AppCtx {}
```
