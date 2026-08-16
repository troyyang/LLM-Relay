# Rust LLM HTTP Relay
## Lightweight Large Language Model Request Forwarding Service

**Version:** 1.0
**Status:** Proposed
**Language:** Rust
**Primary Purpose:** Transparent HTTP/HTTPS forwarding for LLM APIs
**Target environment:** A single mainstream Linux distribution host (Ubuntu, Debian, RHEL/CentOS/Rocky/Alma, Fedora, openSUSE), no container runtime, 512MB RAM, 10GB disk, expected to handle high concurrency (many simultaneous long-lived streaming LLM connections).

---

## 1. Overview

This project provides a lightweight Rust-based HTTP relay service for forwarding requests to Large Language Model (LLM) providers.

The service is intentionally designed as a **thin forwarding layer**, not as a full-featured LLM Gateway.

The core principle is:

> **The client decides what to request; the relay only forwards the request and returns the provider's response.**

The relay should not understand or manage:

- LLM models
- Prompt structures
- Messages
- Token usage
- Model capabilities
- Provider-specific request schemas
- Provider-specific response schemas
- Business-level routing
- Conversation state

The relay should primarily perform:

1. Request reception
2. Relay access-key validation (§11.2) — reject before any forwarding if missing or incorrect
3. Provider/path routing
4. HTTP header forwarding
5. Request body forwarding
6. Upstream request execution
7. Streaming response forwarding
8. Response status/header forwarding
9. Basic logging and request identification
10. Target-host security validation

The service is also built around a tight resource envelope from the outset: it must run comfortably on a 512MB RAM / 10GB disk host with no container runtime, while remaining able to serve high concurrency (many simultaneous streaming connections) without exhausting memory.

---

# 2. Design Goals

## 2.1 Primary Goals

The service must be:

- Lightweight
- Stateless
- Fast
- Provider-agnostic
- Protocol-transparent
- Streaming-compatible
- Easy to deploy
- Easy to configure
- Easy to extend
- Easy to debug
- Predictable and bounded in memory and disk usage under high concurrency

The service should introduce as little processing overhead as possible.

---

## 2.2 Core Principle

The relay follows the following model:

```text
Client
  │
  │ HTTP Request
  ▼
┌──────────────────────┐
│    Rust LLM Relay    │
│                      │
│  Receive             │
│  Validate            │
│  Forward             │
│  Stream              │
│  Return              │
└──────────┬───────────┘
           │
           │ HTTP Request
           ▼
┌──────────────────────┐
│    LLM Provider      │
│                      │
│ OpenAI               │
│ OpenRouter            │
│ Gemini               │
│ Anthropic            │
│ Other HTTP APIs      │
└──────────────────────┘
```

The relay does not transform the LLM request unless explicitly required.

---

# 3. Non-Goals

The first version should explicitly avoid becoming a complete AI management platform.

The following capabilities are **out of scope**:

- User management
- Database-backed API key management
- Model management
- Model capability discovery
- Token accounting
- Billing
- Cost calculation
- Provider health scoring
- Load balancing
- Intelligent model routing
- Prompt management
- Conversation management
- RAG
- Agent orchestration
- Tool management
- Provider-specific schema conversion
- Automatic fallback
- Business-level authorization
- Persistent request history
- Container orchestration or image management

These capabilities can be added later if required, but they should not influence the initial architecture.

---

# 4. High-Level Architecture

```text
                         ┌─────────────────────┐
                         │       Client        │
                         │                     │
                         │ OpenAI SDK           │
                         │ CLI                  │
                         │ Application          │
                         │ Custom Service       │
                         └──────────┬──────────┘
                                    │
                                    │ HTTP
                                    ▼
                         ┌─────────────────────┐
                         │    Rust LLM Relay   │
                         │                     │
                         │  1. Receive         │
                         │  2. Validate        │
                         │  3. Resolve Target  │
                         │  4. Forward         │
                         │  5. Stream          │
                         │  6. Return          │
                         └──────────┬──────────┘
                                    │
                   ┌────────────────┼────────────────┐
                   │                │                │
                   ▼                ▼                ▼
              ┌─────────┐     ┌────────────┐    ┌─────────┐
              │ OpenAI  │     │ OpenRouter │    │ Gemini  │
              └─────────┘     └────────────┘    └─────────┘
                   │                │                │
                   └────────────────┼────────────────┘
                                    ▼
                              LLM Provider
```

---

# 5. Request Flow

A normal request follows this flow:

```text
Client
  │
  │ POST /proxy/openai/v1/chat/completions
  │
  ▼
Rust Relay
  │
  ├── Validate provider
  │
  ├── Build upstream URL
  │
  ├── Forward headers
  │
  ├── Forward body
  │
  ▼
OpenAI
  │
  │ Response
  ▼
Rust Relay
  │
  ├── Forward status
  ├── Forward headers
  └── Forward body/stream
  │
  ▼
Client
```

The relay should not deserialize the JSON request body unless absolutely necessary.

---

# 6. API Design

## 6.1 Recommended Endpoint

The relay access key is carried as a path segment, not a header, so that adopting the relay requires only pointing a client's existing `base_url` configuration at the relay — no header injection, no SDK modification, no application code changes:

```text
POST /{relay_api_key}/proxy/{provider}/{path...}
```

Examples:

```text
POST /a1b2c3d4e5f6.../proxy/openai/v1/chat/completions
```

```text
POST /a1b2c3d4e5f6.../proxy/openrouter/api/v1/chat/completions
```

```text
POST /a1b2c3d4e5f6.../proxy/gemini/v1beta/models/gemini-2.5-flash:generateContent
```

The provider name is used only to resolve the upstream base URL. The key segment is validated before that resolution happens (§11.2); the rest of the path is unaffected and behaves exactly as before.

In practice, a client only changes one setting — its `base_url` — to `http://host:5017/{relay_api_key}/proxy/openai` (or the equivalent for another provider), and everything the SDK already does (setting `Authorization: Bearer <provider-key>`, appending `/v1/chat/completions`, streaming, etc.) continues to work unmodified.

---

# 7. Provider Configuration

Provider configuration should remain minimal.

Example:

```yaml
server:
  host: "0.0.0.0"
  port: 5017

auth:
  keys_file: "/etc/llm-relay/keys.yaml"   # relay access keys (hashed); default key auto-generated on first run if absent

providers:
  openai:
    base_url: "https://api.openai.com"

  openrouter:
    base_url: "https://openrouter.ai"

  gemini:
    base_url: "https://generativelanguage.googleapis.com"

  anthropic:
    base_url: "https://api.anthropic.com"
```

The relay does not need to know which model belongs to which provider.

For example:

```json
{
  "model": "google/gemini-2.5-flash"
}
```

is simply forwarded to OpenRouter.

The relay does not interpret the `model` field.

---

# 8. Example: OpenAI

Client request:

```http
POST /a1b2c3d4e5f6.../proxy/openai/v1/chat/completions
Authorization: Bearer sk-xxxxxxxx
Content-Type: application/json
```

Request body:

```json
{
  "model": "gpt-4o",
  "messages": [
    {
      "role": "user",
      "content": "Hello"
    }
  ],
  "stream": false
}
```

The relay constructs:

```text
https://api.openai.com/v1/chat/completions
```

and forwards the request.

The relay does not need to know what `gpt-4o` means.

---

# 9. Example: OpenRouter

Client request:

```http
POST /a1b2c3d4e5f6.../proxy/openrouter/api/v1/chat/completions
Authorization: Bearer sk-or-xxxxxxxx
Content-Type: application/json
```

Body:

```json
{
  "model": "google/gemini-2.5-flash",
  "messages": [
    {
      "role": "user",
      "content": "Hello"
    }
  ],
  "stream": false
}
```

The relay forwards it to:

```text
https://openrouter.ai/api/v1/chat/completions
```

Again, the relay does not interpret the model.

---

# 10. Header Forwarding

The relay should forward client headers to the upstream provider whenever practical.

Typical headers include:

```text
Authorization
Content-Type
Accept
User-Agent
X-Request-ID
X-OpenRouter-*
```

The relay may remove or overwrite only headers that are unsafe or inappropriate to forward.

The following principle should be followed:

> **Forward by default, modify only when necessary.**

---

# 11. Authentication

There are two distinct, unrelated credentials in play, and the relay must not confuse them:

1. **Provider credentials** — the client's own API key for OpenAI/OpenRouter/Gemini/Anthropic/etc. The relay never generates, stores, or validates these; it only forwards them upstream unchanged.
2. **Relay access key** — a key that gates access to the relay itself, generated and stored by the relay, checked on every incoming request before any forwarding occurs.

## 11.1 Provider Credentials (forwarded, not managed)

The client supplies its own provider credentials in the standard `Authorization` header:

```http
Authorization: Bearer sk-xxxx
```

The relay forwards this header to the provider unchanged:

```text
Client
  │
  │ Provider API Key
  ▼
Rust Relay
  │
  │ Same API Key
  ▼
Provider
```

The relay does not need to store:

```text
OpenAI API Key
OpenRouter API Key
Gemini API Key
Anthropic API Key
```

This keeps the service stateless with respect to provider credentials and substantially reduces security complexity.

## 11.2 Relay Access Key (generated, stored, and enforced by the relay)

The relay itself requires its own key before it will forward anything at all. This protects the relay endpoint from being used by anyone who can merely reach the port — otherwise the service would be an open, unauthenticated proxy to whatever providers are configured.

### Key generation

On first startup, if no relay API key exists yet, the relay generates one automatically:

- Generated from an OS-level cryptographically secure random source (for example, Rust's `OsRng` / the `getrandom` crate — never a non-cryptographic RNG).
- At least 256 bits of entropy (e.g. 32 random bytes), encoded as a hex or base64url string for easy copy/paste into client configuration.
- Generated once and persisted; subsequent restarts reuse the existing key rather than rotating it silently.

### Key storage

Relay access keys are written to a local file, not to the main YAML config, so they can be kept outside of version control and given tighter file permissions than the general config:

```text
/etc/llm-relay/keys.yaml
```

The relay supports **more than one active key** (for example, one per calling application or per environment), so that a single key can be handed to one integration and rotated later without affecting the others. Each entry stores metadata and a salted hash of the key — never the plaintext:

```yaml
keys:
  - id: "3f9a2b7c"
    label: "default"
    created_at: "2026-08-16T09:12:03Z"
    hash: "sha256:8f2a1c...e91d"   # salted hash of the key; plaintext is never persisted

  - id: "a4e710d2"
    label: "billing-service"
    created_at: "2026-08-20T14:03:47Z"
    hash: "sha256:1b77fe...52ac"
```

- File permissions restricted to the service's own user (`chmod 600`), owner-readable only.
- On first startup, if `keys.yaml` does not exist, the relay generates a single key labeled `default`, persists its hash, and prints the plaintext value once to stdout/log — the same one-time-disclosure behavior as before, just backed by a file that can hold more than one entry.
- A newly generated key's plaintext is **shown exactly once**, at generation time (whether that's the automatic first-run key or one created later via the CLI, §32.3). It is not recoverable afterward, by design — only its hash is kept, mirroring how a service should treat any credential. Losing a key means generating a replacement, not retrieving the old value.
- Every request's presented key is checked against the stored hashes; a match against **any** entry authenticates the request. This is what allows independent generation and (manual) removal of individual keys without disturbing the others.
- An operator may instead hand-edit `keys.yaml` to add or remove entries directly (each `hash` value can be produced with any SHA-256 tool) if the CLI is not desired — the relay only auto-generates the initial `default` key when the file is absent.

### Request validation

Every incoming request carries a relay access key as the **first path segment**, ahead of `/proxy/...`:

```text
POST /{relay_api_key}/proxy/{provider}/{path...}
```

This is a deliberate choice over a custom header: a header requires the caller to modify how their HTTP client or SDK constructs requests, which many LLM SDKs make awkward (some don't expose a simple way to inject an arbitrary default header). Putting the key in the URL means the *only* configuration change a caller makes is their `base_url` — everything else (the SDK's own `Authorization` header, path construction, streaming handling) works unmodified.

The check happens in a dedicated authentication step that runs **before** provider resolution, URL construction, or any upstream connection attempt — a rejected request should do essentially no work beyond comparing the key:

```text
Client
  │
  │ POST /{key}/proxy/{provider}/{path...}
  ▼
Rust Relay
  │
  ├── Key segment missing/malformed?     → 401, forwarding interrupted
  ├── Key hash matches no stored entry   → 401, forwarding interrupted
  └── Key hash matches a stored entry    → strip the key segment, continue to provider resolution
  │
  ▼
Provider
```

Implementation notes:

- Hash the presented key the same way stored keys are hashed, then compare against each stored hash using a constant-time comparison (for example, `subtle::ConstantTimeEq`) rather than a plain `==`, to avoid leaking timing information about how much of any candidate matched.
- A missing/malformed segment and an incorrect key should both simply produce `401 Unauthorized` with no further detail (avoid distinguishing "missing" from "wrong" in the response body, which would give an attacker a probing oracle).
- No part of the request — headers, body, or provider path — should be forwarded upstream when this check fails; forwarding is interrupted immediately.
- This check is independent of, and runs in addition to, the provider allowlist (§20) and concurrency admission control (§15.2).
- **The key must never be echoed into an error body, a redirect `Location` header, or forwarded upstream as part of the path** — it is stripped from the path before the remaining `/proxy/{provider}/{path...}` segment is used to build the upstream request.
- With a small number of keys (single digits to low tens, the expected scale for this use case), checking against every stored hash on each request is negligible overhead; this is not designed to scale to a large multi-tenant key store (that would push toward the database-backed key management explicitly excluded in §3).

### Trade-off: a URL-embedded secret

Putting a secret in the URL path is deliberately convenient, but it comes with a real trade-off worth calling out rather than glossing over: URLs tend to end up in more places than headers do — web/reverse-proxy access logs, browser history if ever opened directly, and shell history if invoked via `curl` on a shared machine. This design accepts that trade-off for the sake of zero-code-change adoption, and compensates for it directly:

- **Always deploy behind TLS** (§33) — this is what prevents the key from being visible on the wire; without it, the key is no better protected than a plaintext password.
- **Do not log the full request path** at the relay or at any reverse proxy in front of it; log the method, provider, and downstream path with the key segment redacted (see §26).
- **Treat each key like a password**: rotate a compromised or retiring key by running `llm-relay keys generate` (§32.3) to issue a replacement, updating the caller to use it, then removing the old entry from `/etc/llm-relay/keys.yaml` — no restart is required for either half of that swap.
- If a deployment's threat model makes URL-based logging exposure unacceptable, a header-based alternative can be layered in later as an opt-in — but it should not be the default, since it reintroduces the per-caller code-change burden this design is explicitly avoiding.

---

# 12. Request Body Handling

The request body should be treated as opaque data.

For example:

```json
{
  "model": "some-model",
  "messages": [],
  "temperature": 0.7,
  "stream": true,
  "tools": [],
  "response_format": {}
}
```

The relay does not need to parse any of these fields.

The body should be forwarded directly, as a stream of bytes rather than a fully materialized value. Anywhere the implementation would be tempted to read the whole incoming body into memory (for example, calling something like `to_bytes()` on it) before sending it upstream, that is incorrect at this memory budget — the request body must be piped chunk-by-chunk into the outbound request, exactly as the response body is streamed back to the client (see §14).

Conceptually:

```text
Incoming Body
     │
     │ bytes
     ▼
Rust Relay
     │
     │ bytes
     ▼
Upstream Provider
```

This has several advantages:

- No provider-specific schema dependency
- No unnecessary JSON parsing
- Lower CPU overhead
- Lower memory usage, independent of concurrency level
- Support for future provider APIs
- Easier maintenance

---

# 13. Resource Envelope

The service is expected to run on a 512MB RAM / 10GB disk host, which is very likely the **entire machine's** budget rather than an allowance dedicated solely to this process. The design accounts for that from the start.

## 13.1 Memory

| Consumer | Estimated RAM |
|---|---|
| Kernel + base OS daemons (sshd, systemd, cron, journald) | ~60–100MB |
| llm-relay idle (static binary, tokio runtime spun up) | ~8–15MB |
| llm-relay under load (per-connection overhead) | scales with concurrency — bounded by admission control, see §15 |
| Free headroom / page cache / safety margin | remainder |

A small swap file or zram device (e.g. 256–512MB) is recommended as a safety net against a traffic spike causing an OOM kill. Both are available out of the box on mainstream kernels (`zram` module, or a plain swapfile via `fallocate`) and need no extra software beyond what the distro already ships. A 256MB swapfile costs roughly 2.5% of the 10GB disk budget — cheap insurance.

## 13.2 Disk

| Consumer | Estimated Disk |
|---|---|
| Binary | ~4–10MB (see §21) |
| Config | <1MB |
| Logs (capped, see §27) | configurable ceiling, recommend ≤200MB |
| Remainder | available to the OS and other software on the host |

---

# 14. Streaming Support

Streaming is a core requirement, in both directions.

For example, on the response path:

```text
Client
  │
  │ stream=true
  ▼
Rust Relay
  │
  │ streaming request
  ▼
Provider
  │
  ├── chunk 1
  ├── chunk 2
  ├── chunk 3
  ├── chunk 4
  └── [DONE]
  │
  ▼
Rust Relay
  │
  ├── chunk 1
  ├── chunk 2
  ├── chunk 3
  ├── chunk 4
  └── [DONE]
  │
  ▼
Client
```

The relay should not buffer the entire response.

Instead, it should use a streaming HTTP body.

With `reqwest`:

```rust
let stream = response.bytes_stream();
```

The stream can then be passed directly into an Axum response body.

Conceptually:

```rust
let body = Body::from_stream(response.bytes_stream());
```

This allows the relay to support:

- SSE
- Chunked responses
- Long-running LLM responses
- Streaming token generation

without understanding the provider's response format.

The same discipline applies to the request path (client → relay → upstream): the incoming body must be forwarded as a stream, not collected into a buffer first (see §12). Streaming without full buffering bounds the *peak* memory used by any single request; it does not, by itself, bound the *total* memory used under high concurrency, since that is `(per-request footprint) × (number of concurrent requests)`. The second factor is handled explicitly by admission control (§15).

---

# 15. Concurrency & Memory Control

At 512MB, the number of simultaneous in-flight requests has to be actively managed rather than left unbounded, even with strict streaming in place.

## 15.1 Explicit worker-thread count

Do not let the async runtime auto-detect the number of CPU cores on a cloud VM that might report several vCPUs despite a small RAM allocation — each worker thread carries its own stack and scheduler overhead. Pin it low and make it configurable:

```yaml
runtime:
  worker_threads: 2
```

## 15.2 Admission control (bounded concurrency)

A global concurrency limiter sits in front of the proxy handler (for example, `tower::limit::ConcurrencyLimitLayer`, or an equivalent semaphore-based middleware). When the limit is reached, the relay returns `503 Service Unavailable` with `Retry-After`, rather than accepting unbounded additional work.

```yaml
server:
  host: "0.0.0.0"
  port: 5017
  max_body_size: "8MB"
  max_concurrent_requests: 800
```

```rust
let app = Router::new()
    .route("/proxy/{provider}/{*path}", any(proxy_handler))
    .layer(ConcurrencyLimitLayer::new(config.server.max_concurrent_requests));
```

This is the mechanism that lets the relay handle high concurrency safely within a fixed memory budget: rather than refusing to be concurrent, it caps concurrency at a level the budget can sustain and rejects fast (cheap) instead of degrading into swapping or an OOM kill (expensive, and takes the whole process down along with every in-flight request).

## 15.3 Outbound connection pool tuning

`reqwest`'s default connection-pool settings are tuned for general-purpose use, not a 512MB host. Cap them explicitly:

```rust
let client = reqwest::Client::builder()
    .pool_max_idle_per_host(8)
    .pool_idle_timeout(Duration::from_secs(30))
    .connect_timeout(Duration::from_secs(10))
    .build()?;
```

## 15.4 Request size limit as a secondary control

`max_body_size` (see §7 and §35) is set at 8MB by default — generous for typical chat/completion payloads including moderate multimodal attachments, while bounding the in-flight buffer for any single connection and discouraging abusive large payloads from amplifying memory pressure under high concurrency. Raise it in config if a specific provider's payloads require more.

---

# 16. Capacity Planning

A rough per-connection overhead for a streaming proxied request (task stack, read/write buffers, header allocations) is on the order of **50–150KB** per active connection.

| Usable RAM for connections | Est. overhead/connection | Approx. max concurrent streaming requests |
|---|---|---|
| ~250MB (conservative, after OS + idle binary + margin) | 150KB | ~1,700 |
| ~250MB | 50KB (optimistic) | ~5,000 |

`max_concurrent_requests: 800` (§15.2) is a sensible starting default given the uncertainty in that estimate — comfortably below the worst-case ceiling. Treat it as a dial to tune after measuring actual RSS-per-connection on real traffic (`smem`, or `/proc/<pid>/status`), not a constant to trust blindly.

---

# 17. Response Handling

The response should be returned as transparently as possible.

The relay should preserve:

```text
HTTP status
Response headers
Response body
Streaming behavior
```

For example, if the provider returns:

```http
HTTP/2 200
Content-Type: text/event-stream
```

the relay should return:

```http
HTTP/2 200
Content-Type: text/event-stream
```

with the response body streamed directly.

---

# 18. Error Handling

The relay should distinguish between:

### Upstream HTTP errors

For example:

```text
401 Unauthorized
403 Forbidden
404 Not Found
429 Too Many Requests
500 Internal Server Error
```

These should generally be returned to the client without rewriting them.

For example:

```text
Provider
   │
   │ 429
   ▼
Rust Relay
   │
   │ 429
   ▼
Client
```

The relay should not convert every provider error into:

```text
500 Internal Server Error
```

because doing so would hide useful provider information.

---

# 19. Relay-Level Errors

The relay itself may generate errors for situations such as:

```text
401 Unauthorized
```

when:

- The relay access key path segment is missing or malformed
- The relay access key does not match the relay's stored key

This check (§11.2) runs first, before any other relay-level or upstream processing, so a `401` means forwarding was interrupted before the provider or path were even resolved.

or:

```text
400 Bad Request
```

when:

- Provider is missing
- Path is invalid

or:

```text
403 Forbidden
```

when:

- Provider is not allowed

or:

```text
502 Bad Gateway
```

when:

- Upstream connection fails

or:

```text
503 Service Unavailable
```

when:

- The concurrency admission-control limit has been reached (see §15.2)

or:

```text
504 Gateway Timeout
```

when:

- Upstream request times out

Example:

```text
Client
  │
  ▼
Rust Relay
  │
  │ connection failed
  ▼
502 Bad Gateway
```

---

# 20. SSRF Protection

Because the service forwards requests to external systems, SSRF protection is important.

The relay should **not** allow arbitrary target URLs in the first version.

Instead, providers should be resolved from a predefined configuration:

```yaml
providers:
  openai:
    base_url: "https://api.openai.com"

  openrouter:
    base_url: "https://openrouter.ai"

  gemini:
    base_url: "https://generativelanguage.googleapis.com"
```

The client specifies:

```text
/proxy/openai/...
```

but cannot specify:

```text
http://127.0.0.1
```

or:

```text
http://169.254.169.254
```

This prevents the relay from becoming an unrestricted HTTP proxy.

---

# 21. Provider Resolution

The routing logic should remain extremely simple.

Conceptually:

```rust
let provider = providers
    .get(provider_name)
    .ok_or(UnknownProvider)?;

let target_url = format!(
    "{}{}",
    provider.base_url,
    request_path
);
```

For example:

```text
provider = openai

base_url =
https://api.openai.com

path =
/v1/chat/completions

result =
https://api.openai.com/v1/chat/completions
```

No model routing is involved.

---

# 22. HTTP Client and Build Configuration

The recommended HTTP client is `reqwest`, on top of a `tokio` runtime trimmed to only the features actually used, and built for a minimal, fully static binary.

## 22.1 Dependencies

```toml
[dependencies]
axum = "0.8"
tokio = { version = "1", default-features = false, features = ["rt-multi-thread", "net", "macros", "signal", "time"] }

reqwest = {
    version = "0.12",
    default-features = false,
    features = ["stream", "rustls-tls", "socks"]
}

serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"

tracing = "0.1"
tracing-subscriber = { version = "0.3", default-features = false, features = ["fmt"] }

thiserror = "2"

clap = { version = "4", features = ["derive"] }
sha2 = "0.10"
```

`clap` backs the small CLI surface described in §32.3 (`providers`, `keys`, `config`, and `serve`); `sha2` is used only to hash relay access keys for storage (§11.2) — it is not involved in request forwarding.

Two choices are worth calling out:

- **`default-features = false` on `tokio`, keeping only what's used.** The full feature set pulls in modules (process, UDS, filesystem, etc.) the relay never touches — trimming them reduces both binary size and background bookkeeping.
- **`rustls-tls` instead of the default OpenSSL/native-tls backend.** This removes any runtime dependency on the system's `libssl`, which both keeps the binary fully static (see §22.2) and avoids "works on one distro's OpenSSL version but not another's" problems — this is what actually makes "runs unmodified across mainstream Linux distributions" true, more so than any packaging format would.

The `socks` feature is retained for the optional SOCKS5 outbound path described in §24.

## 22.2 Release profile and static build

```toml
[profile.release]
opt-level = "z"      # optimize for size; "s" if a speed/size middle ground is preferred
lto = true
codegen-units = 1
panic = "abort"       # no unwinding tables — smaller binary; panics aren't caught anyway
strip = true
```

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

A musl-static binary runs unmodified on Ubuntu, Debian, RHEL/CentOS/Rocky/Alma, Fedora, and openSUSE without depending on the host's glibc version, OpenSSL version, or any shared library at all. Combined with rustls (§22.1), a stripped, LTO'd build typically lands in the **4–10MB** range with **no dynamic library dependencies** — verify with `ldd target/.../llm-relay`, which should report "not a dynamic executable."

---

# 23. SOCKS5 Support

The architecture should allow the outbound HTTP client to use a proxy without changing the application layer.

For example:

```text
Client
  │
  ▼
Rust Relay
  │
  │ Reqwest
  ▼
SOCKS5 Proxy
  │
  ▼
LLM Provider
```

This is particularly useful when the server's direct network access to an LLM provider is restricted.

The application logic remains:

```text
request
    ↓
relay
    ↓
provider
```

Only the transport layer changes:

```text
Reqwest
   ↓
SOCKS5
   ↓
Internet
```

---

# 24. Timeout Configuration

The relay should provide basic timeout configuration.

Example:

```yaml
timeout:
  connect: 10
  request: 300
```

Recommended initial behavior:

```text
Connect timeout:
10 seconds

Request timeout:
300 seconds
```

The timeout should be long enough for LLM requests, especially reasoning models.

Streaming requests should not be treated like ordinary short HTTP requests.

---

# 25. Request ID

Each request should have a request ID.

If the client provides:

```http
X-Request-ID: abc123
```

the relay may preserve it.

Otherwise, the relay should generate one.

Example:

```text
req_01KXYZ123456
```

The request ID should be included in logs.

Example:

```text
request_id=req_01KXYZ123456
provider=openai
path=/v1/chat/completions
status=200
latency=1832ms
```

---

# 26. Logging and Disk Management

Logging should remain lightweight and disk usage bounded.

Recommended fields:

```text
timestamp
request_id
provider
path
method
status
latency
error
```

Example:

```text
2026-08-16T10:32:21Z
request_id=req_01KXYZ
provider=openai
method=POST
path=/v1/chat/completions
status=200
latency=1832ms
```

The relay should **not log request bodies by default**.

This prevents accidental exposure of:

- Passwords
- API keys
- Personal data
- Business information
- Source code
- Private prompts

**The `path` field must never include the relay access key segment.** Because the key now travels in the URL (§11.2) rather than a header, the logging layer must capture the path *after* the auth middleware has stripped the leading key segment — never `request.uri().path()` taken raw at the point of receipt. The same applies to any reverse proxy placed in front of the relay (nginx/Caddy/Traefik access logs): configure it to log the path with the key segment masked or omitted, not the full raw request line.

It also matters directly for the disk budget: avoid per-chunk streaming logs, which would scale with token count multiplied by concurrency.

Preferred approach: log to stdout/stderr and let systemd's journald capture it — no extra log-rotation dependency to install. Cap journald's own storage at the OS level in `/etc/systemd/journald.conf`:

```ini
[Journal]
SystemMaxUse=200M
```

---

# 27. Project Structure

The initial project can remain very small.

```text
llm-relay/
│
├── Cargo.toml
│
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── proxy.rs
│   ├── auth.rs
│   ├── keys.rs
│   ├── config.rs
│   └── error.rs
│
├── config/
│   └── config.yaml
│
└── README.md
```

There is no need to introduce a large module hierarchy.

---

# 28. Module Responsibilities

## `main.rs`

Responsible for:

- Parsing CLI arguments (delegates the argument schema to `cli.rs`)
- Dispatching to either `serve` (application startup: configuration loading, relay access key loading/generation, HTTP client creation, Axum server creation, router initialization including the authentication middleware layer) or one of the administrative subcommands (`providers list`, `keys list`, `keys generate`, `config ...`, all described in §32.3)
- Administrative subcommands run to completion and exit; they never start the HTTP server

---

## `cli.rs`

Responsible for:

- Defining the CLI's subcommands and flags (§32.3) using `clap`'s derive API
- Rendering `--help` output, including the configuration-editing guidance shown in §32.3
- Nothing in this module touches the network or the request-forwarding path — it is pure argument parsing and help text

---

## `config.rs`

Responsible for:

- Server configuration
- Provider configuration
- Timeout configuration
- Optional proxy configuration

---

## `auth.rs`

Responsible for:

- Extracting the key segment from the request path
- Hashing the presented key and validating it against the stored entries (delegates storage to `keys.rs`) using constant-time comparison
- Stripping the key segment before the remaining path is passed on to `proxy.rs`
- Rejecting unauthenticated requests with `401` before any other processing occurs

---

## `keys.rs`

Responsible for:

- Reading and writing `/etc/llm-relay/keys.yaml` (§11.2)
- Generating a new relay access key using a CSPRNG, hashing it, and persisting the hash with restrictive file permissions
- Auto-generating the initial `default` key on first run if `keys.yaml` does not exist yet
- Listing stored key metadata (`id`, `label`, `created_at`) for the `keys list` CLI command — this module never re-exposes a stored key's plaintext, since only its hash is retained
- Shared by both the HTTP request path (`auth.rs`, read-only lookups) and the CLI (`cli.rs` / `main.rs`, read and write)

---

## `proxy.rs`

Responsible for:

- Extracting provider
- Building upstream URL
- Forwarding request
- Forwarding headers
- Forwarding body
- Streaming response

This is the core of the application. It is only reached after `auth.rs` has approved the request.

---

## `error.rs`

Responsible for:

- Relay-specific errors, including authentication failures
- Upstream connection errors
- Configuration errors
- HTTP error conversion

---

# 29. Core Processing Model

The complete request handler can conceptually be reduced to:

```text
1. Receive request
2. Extract the relay access key from the first path segment; validate against the stored key — reject with 401 and stop if missing/incorrect
3. Strip the key segment, leaving `/proxy/{provider}/{path...}`
4. Extract provider
5. Resolve provider configuration
6. Construct target URL
7. Copy HTTP method
8. Copy HTTP headers
9. Stream request body to upstream
10. Send upstream request
11. Copy upstream status
12. Copy upstream headers
13. Stream upstream body to client
```

There should be no LLM-specific processing in this pipeline.

---

# 30. Pseudo-Code

The authentication check runs as middleware, ahead of the proxy handler, so a rejected request never reaches provider resolution. It reads the relay access key from the leading path segment rather than a header, then rewrites the request path to drop that segment before passing it on:

```rust
async fn require_relay_api_key(
    mut request: Request<Body>,
    next: Next,
) -> Result<Response<Body>, AuthError> {

    // Path shape: /{relay_api_key}/proxy/{provider}/{path...}
    let mut segments = request.uri().path().splitn(2, '/').skip(1);
    let presented_key = segments.next();
    let remaining_path = segments.next(); // "proxy/{provider}/{path...}"

    match (presented_key, remaining_path) {
        (Some(key), Some(rest)) if stored_keys.iter().any(|stored| {
            constant_time_eq(hash(key.as_bytes()).as_ref(), stored.hash.as_ref())
        }) => {
            *request.uri_mut() = rewrite_path(request.uri(), rest)?;
            Ok(next.run(request).await)
        }
        _ => Err(AuthError::Unauthorized),
        // No forwarding, no provider resolution, no upstream
        // connection is attempted beyond this point.
        // `stored_keys` is loaded from keys.yaml (§11.2) and reflects
        // whatever `llm-relay keys generate` has most recently written.
    }
}
```

```rust
async fn proxy(
    provider: String,
    path: String,
    request: Request<Body>,
) -> Result<Response<Body>, ProxyError> {

    let provider_config =
        config.provider(&provider)?;

    let target_url =
        format!("{}{}", provider_config.base_url, path);

    let method =
        request.method().clone();

    let headers =
        request.headers().clone();

    let body =
        request.into_body();

    let upstream_request =
        client
            .request(method, target_url)
            .headers(headers)
            .body(body)
            .send()
            .await?;

    let status =
        upstream_request.status();

    let headers =
        upstream_request.headers().clone();

    let body =
        Body::from_stream(
            upstream_request.bytes_stream()
        );

    build_response(
        status,
        headers,
        body
    )
}
```

The actual implementation will need appropriate conversions between Axum and Reqwest body types, but the architecture should remain this simple.

---

# 31. Data Flow

The relay should be fundamentally stateless.

```text
                 Request N
                    │
                    ▼
              Rust Relay
                    │
                    ▼
               Provider
                    │
                    ▼
                 Response
```

There is no requirement for:

```text
Request N
    ↓
Database
    ↓
Request N+1
```

Every request is independent.

---

# 32. Deployment

The service is deployed as a single static binary supervised by systemd — no container runtime is required or used.

```text
/usr/local/bin/llm-relay        # static binary, ~4–10MB
/etc/llm-relay/config.yaml      # config
/etc/llm-relay/keys.yaml        # relay access keys (hashed), auto-generated on first run, chmod 600
/etc/systemd/system/llm-relay.service
```

systemd provides process supervision, restart policy, resource limits via cgroups, and log capture via journald — everything needed for production operation on any mainstream distribution, without adding a container daemon that would itself consume a meaningful share of a 512MB budget.

On the very first start, if `/etc/llm-relay/keys.yaml` does not exist, the relay generates a `default` key (§11.2) and prints it once to the service log so the operator can retrieve it with `journalctl -u llm-relay`. On every subsequent start, the existing keys are loaded and reused. Additional keys can be generated at any time — with the service running or stopped — via `llm-relay keys generate` (§32.3).

## 32.1 Install script

```bash
#!/usr/bin/env bash
set -euo pipefail

VERSION="v1.0.0"
BIN_URL="https://github.com/yourorg/llm-relay/releases/download/${VERSION}/llm-relay-x86_64-unknown-linux-musl"

install -d -m 700 /etc/llm-relay
curl -fsSL "$BIN_URL" -o /usr/local/bin/llm-relay
chmod +x /usr/local/bin/llm-relay

if [ ! -f /etc/llm-relay/config.yaml ]; then
  cp config.example.yaml /etc/llm-relay/config.yaml
fi

cp llm-relay.service /etc/systemd/system/llm-relay.service
systemctl daemon-reload
systemctl enable --now llm-relay

# The relay generates /etc/llm-relay/keys.yaml on first boot if it
# doesn't already exist, and prints the first key's plaintext once. Retrieve it with:
#   journalctl -u llm-relay | grep "relay access key"
#
# If that one-time output was missed, the key itself cannot be recovered
# (only its hash is stored) — generate a new one instead:
#   llm-relay keys generate --label default-replacement
```

## 32.2 systemd unit

```ini
[Unit]
Description=Rust LLM HTTP Relay
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
ExecStart=/usr/local/bin/llm-relay --config /etc/llm-relay/config.yaml
Restart=on-failure
RestartSec=2

# Hard resource ceiling, enforced by the kernel via cgroups.
# Leaves headroom below the 512MB physical limit for sshd, systemd,
# and other baseline OS processes.
MemoryMax=350M
MemoryHigh=300M
TasksMax=512

# Reduces glibc malloc arena bloat if ever built against glibc
# instead of musl; harmless and ignored on musl builds.
Environment=MALLOC_ARENA_MAX=1

# Hardening (no memory cost)
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=/etc/llm-relay /var/log/llm-relay
PrivateTmp=true

[Install]
WantedBy=multi-user.target
```

`MemoryHigh` throttles the process via cgroup memory pressure before `MemoryMax` triggers a hard kill, giving the application's own admission control (§15.2) a chance to shed load first.

## 32.3 CLI Commands

Beyond running the server, the same `llm-relay` binary exposes a small set of local, non-networked administrative subcommands. These are operator tools, not relay endpoints — there is no HTTP route for any of them; each runs, prints its output, and exits.

```text
llm-relay [OPTIONS] [COMMAND]

Commands:
  serve                Run the relay HTTP server (default when no command is given)
  providers list        List the providers currently defined in config.yaml
  keys list             List relay access keys (metadata only — see below)
  keys generate          Generate a new relay access key
  config show            Print the active config path and a few key resolved settings
  config check           Validate config.yaml and keys.yaml without starting the server
  help                   Print this help, or the help of a subcommand

Options:
  -c, --config <PATH>   Path to config.yaml [default: /etc/llm-relay/config.yaml]
  -h, --help             Print help
  -V, --version          Print version
```

Because `keys.yaml` and `config.yaml` are typically root/service-user-owned (§32, §33), these subcommands are expected to be run as the same user the service runs as (or via `sudo`), the same way an operator would edit the files directly.

### `providers list`

Reads `providers:` from the resolved config file and prints each configured provider name alongside its `base_url` — a quick way to confirm what the relay will currently route to, without opening the YAML file:

```text
$ llm-relay providers list
PROVIDER      BASE URL
openai        https://api.openai.com
openrouter    https://openrouter.ai
gemini        https://generativelanguage.googleapis.com
anthropic     https://api.anthropic.com
```

This reads the same `providers` map used by provider resolution (§21); it does not query the providers themselves or validate that the URLs are reachable.

### `keys list`

Lists the metadata for every relay access key currently stored in `keys.yaml` (§11.2). Since only a hash is ever persisted, **the plaintext key value is never shown here** — only what was recorded at generation time, plus a short, non-reusable prefix of the hash for operators to visually distinguish entries in logs or scripts:

```text
$ llm-relay keys list
ID          LABEL              CREATED (UTC)          HASH PREFIX
3f9a2b7c    default            2026-08-16T09:12:03Z    8f2a1c...
a4e710d2    billing-service    2026-08-20T14:03:47Z    1b77fe...
```

If an operator needs the actual key value for an existing entry, there is no way to retrieve it — that is a deliberate consequence of hashed storage (§11.2). The remedy is `keys generate` to issue a new one and update the caller's configuration.

### `keys generate`

Generates a new relay access key, stores its hash in `keys.yaml`, and prints the plaintext **exactly once**, to stdout:

```text
$ llm-relay keys generate --label mobile-app
Generated new relay access key (id: 9c21ffab, label: mobile-app):

  7e2f4a1c9b8d3e6f0a5c7b2d9e4f1a6c8b3d5e0f7a2c9b4d6e1f8a3c5b0d7e2f

Store this value now — it will not be shown again.
Use it as the first path segment, e.g.:
  https://your-host:5017/7e2f4a1c.../proxy/openai
```

- `--label <NAME>` is optional; if omitted, the key is stored with an empty label (still listable and usable, just less identifiable in `keys list`).
- This command can be run whether or not the relay server is currently running; if the server is running, the new key becomes valid on its very next request — there is no restart required, since the server re-reads `keys.yaml` lazily and the file is small (this is the one piece of relay state that intentionally supports a lightweight form of hot-reload, in contrast to `config.yaml`, which does not — see `config check` below).
- Key removal/rotation-out is intentionally not a CLI subcommand in this version, consistent with keeping relay-owned state management minimal (§3); an operator who needs to invalidate a specific key can remove its entry from `keys.yaml` directly and, if the server is running, nothing further is required.

### `config show` / `config check`

`config show` prints the path of the config file currently in effect and a handful of top-level resolved values (host, port, provider count, key count) as a fast sanity check after an edit. `config check` parses `config.yaml` and `keys.yaml` and reports the first error found (bad YAML, an unknown field, a provider missing `base_url`, etc.) without starting the HTTP server — intended to be run after hand-editing either file and before restarting the service.

### Configuration guidance in `--help`

Because `config.yaml` has no in-process editor and no hot-reload, `llm-relay --help` (and `llm-relay config --help`) always ends with the same short, concrete guidance rather than assuming the operator already knows the file's location and reload story:

```text
CONFIGURATION
  Settings (server, providers, timeouts, resource limits) live in a YAML
  file, by default:
      /etc/llm-relay/config.yaml
  Override the path for any command with -c/--config <PATH>.

  To change a setting:
    1. Edit the file (root or the llm-relay service user; see §33)
    2. Validate it:            llm-relay config check
    3. Apply it:                systemctl restart llm-relay
       (config.yaml is read fully on startup; there is no hot-reload)

  Relay access keys live separately, in:
      /etc/llm-relay/keys.yaml
  and are managed with `llm-relay keys list` / `llm-relay keys generate`,
  not by hand-editing config.yaml. New keys take effect without a restart.

  See config/config.example.yaml (§40) for a fully annotated example.
```

This keeps the "how do I change something" answer in the one place an operator is most likely to look — the tool's own `--help` — rather than only in this design document or a separate README.

---

# 33. Security Model

The first version should focus on transport security rather than business authorization.

Recommended security measures:

### 1. Relay access key

Every request must present a valid relay access key as the leading URL path segment, matching one of the relay's stored keys (§11.2), each of which may be independently generated via `llm-relay keys generate` (§32.3). Requests with a missing or incorrect key are rejected with `401` before any forwarding, provider resolution, or upstream connection is attempted. This is the first line of defense and applies regardless of which provider is targeted. Because the key travels in the URL rather than a header, TLS (§33, item 3) and log redaction (§26) are what actually keep it confidential — both are treated as required, not optional, for any deployment reachable beyond localhost.

### 2. Provider allowlist

Only configured providers are accessible.

### 3. HTTPS for public deployments

If exposed publicly, use TLS through:

```text
Nginx
Caddy
Traefik
Cloud Load Balancer
```

### 4. Do not log request bodies

### 5. Do not store provider API keys

The relay's own access key (§11.2) is stored locally with restricted file permissions; this is distinct from provider credentials, which are never stored at all.

### 6. Protect configuration files and the keys file

`config.yaml` and `keys.yaml` should both be owner-readable only (`chmod 600`) by the service's own user, and `keys.yaml` in particular should never be committed to version control, copied into logs, or included in backups that are less tightly access-controlled than the host itself — even though it holds hashes rather than plaintext keys, those hashes are still the sole record needed to validate legitimate access. The `keys list` CLI command (§32.3) is safe to run without this concern, since it never prints a stored key's plaintext, only metadata.

### 7. Apply reasonable request-size limits

### 8. Apply connection/request timeouts

### 9. Apply concurrency admission control

Bound the number of simultaneous in-flight requests (§15.2) so that load never translates into unbounded memory growth.

---

# 34. Request Size Limits

Even though the relay should not parse the body, it should still protect itself against extremely large requests.

For example:

```yaml
server:
  max_body_size: "8MB"
```

The exact value can be adjusted depending on use cases; 8MB is generous for typical chat/completion payloads, including moderate multimodal attachments. This prevents accidental or malicious memory/resource exhaustion, and matters even with streaming in place because it bounds the in-flight buffer per connection — a factor that compounds under high concurrency (see §15.4).

---

# 35. Why Not Use `X-Target-URL`?

An alternative design would be:

```http
POST /proxy

X-Target-URL: https://api.openai.com/v1/chat/completions
```

This is flexible, but it introduces a serious security problem.

The client could potentially request:

```text
http://127.0.0.1
```

or:

```text
http://localhost
```

or cloud metadata endpoints.

Therefore, the recommended design is:

```text
/proxy/{provider}/{path}
```

with the provider mapped to a configured base URL.

This preserves simplicity while providing a clear security boundary.

---

# 36. Why Not Parse the Model?

The relay should not perform:

```text
if model == "gpt-4o"
    ...
else if model == "gemini"
    ...
```

because this creates unnecessary coupling.

Instead:

```text
Client
 │
 │ model = whatever
 ▼
Relay
 │
 │ unchanged
 ▼
Provider
```

This means a newly released model can immediately work without changing the relay.

For example, if a provider introduces:

```text
new-model-2026
```

the relay does not need to be rebuilt.

---

# 37. Why Not Convert Provider APIs?

The relay should not initially convert:

```text
OpenAI format
       ↓
Gemini format
```

or:

```text
Gemini response
       ↓
OpenAI response
```

That would turn the project into an LLM abstraction layer.

The goal of this project is different:

> **Transparent forwarding, not protocol normalization.**

If protocol conversion becomes necessary later, it can be implemented as an optional layer.

---

# 38. Performance Characteristics

The relay should introduce minimal overhead.

The ideal data path is:

```text
Client
   │
   │ bytes
   ▼
Axum
   │
   │ bytes
   ▼
Reqwest
   │
   │ network
   ▼
Provider
```

The relay should avoid:

```text
JSON parsing
JSON serialization
Database operations
Response buffering
Token processing
Prompt processing
```

unless explicitly required.

This makes Rust particularly suitable for the service, and is what keeps per-connection memory overhead low enough to sustain high concurrency within a 512MB budget.

---

# 39. Recommended Technology Stack

| Component | Technology |
|---|---|
| Language | Rust |
| Runtime | Tokio (trimmed feature set, explicit worker-thread count) |
| HTTP Server | Axum |
| HTTP Client | Reqwest (rustls-tls, capped connection pool) |
| Streaming | Tokio / Streams |
| Configuration | YAML + Serde |
| Logging | tracing → stdout → journald (capped) |
| Process supervision | systemd (native, no container runtime) |
| Concurrency control | `tower::limit::ConcurrencyLimitLayer` / semaphore-based admission control |
| Database | None |
| Cache | None |
| Authentication | Provider credentials supplied by client |
| Routing | Static provider mapping |

The entire service can therefore remain very small, both in code size and in runtime footprint.

---

# 40. Example Configuration

```yaml
server:
  host: "0.0.0.0"
  port: 5017
  max_body_size: "8MB"
  max_concurrent_requests: 800

auth:
  keys_file: "/etc/llm-relay/keys.yaml"   # relay access keys (hashed); default key auto-generated on first run if absent

runtime:
  worker_threads: 2

timeout:
  connect: 10
  request: 300

pool:
  max_idle_per_host: 8
  idle_timeout: 30

providers:

  openai:
    base_url: "https://api.openai.com"

  openrouter:
    base_url: "https://openrouter.ai"

  gemini:
    base_url: "https://generativelanguage.googleapis.com"

  anthropic:
    base_url: "https://api.anthropic.com"
```

---

# 41. Example Client Usage

## OpenAI

```bash
curl \
  "http://localhost:5017/$RELAY_API_KEY/proxy/openai/v1/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {
        "role": "user",
        "content": "Hello"
      }
    ]
  }'
```

`$RELAY_API_KEY` is embedded directly in the path — the relay's own access key (§11.2). `Authorization` still carries the client's OpenAI credential, forwarded upstream unchanged. An SDK client only needs its `base_url` pointed at `http://localhost:5017/$RELAY_API_KEY/proxy/openai`; no header configuration is required.

---

## OpenRouter

```bash
curl \
  "http://localhost:5017/$RELAY_API_KEY/proxy/openrouter/api/v1/chat/completions" \
  -H "Authorization: Bearer $OPENROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "google/gemini-2.5-flash",
    "messages": [
      {
        "role": "user",
        "content": "Hello"
      }
    ]
  }'
```

---

# 42. Streaming Example

```bash
curl \
  "http://localhost:5017/$RELAY_API_KEY/proxy/openai/v1/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {
        "role": "user",
        "content": "Write a short story."
      }
    ],
    "stream": true
  }'
```

The relay should stream the response directly rather than waiting for the complete response.

---

# 43. Future Extension: Multiple Network Routes

If needed later, different providers can use different network routes.

For example:

```text
                     Rust Relay
                         │
             ┌───────────┴───────────┐
             │                       │
             ▼                       ▼
         Direct Route           SOCKS5 Route
             │                       │
             ▼                       ▼
          OpenAI                 OpenRouter
```

This should be implemented at the HTTP client/transport layer rather than in the LLM request processing layer.

---

# 44. Future Extension: Optional Request Transformation

If future requirements introduce a need for transformation, it can be added without changing the basic architecture.

For example:

```text
                 Request
                    │
                    ▼
             ┌──────────────┐
             │ Optional      │
             │ Middleware    │
             └──────┬───────┘
                    │
                    ▼
                Forward
```

Potential middleware could include:

```text
Authentication
Header injection
Request modification
Logging
Metrics
Tracing
```

However, these should remain optional.

---

# 45. Architecture Principles

The project should follow these principles.

### Principle 1 — Keep the relay thin

Do not turn the relay into an AI platform.

### Principle 2 — Do not understand what you do not need to understand

The relay does not need to understand model semantics.

### Principle 3 — Forward bytes whenever possible

Avoid unnecessary serialization and deserialization.

### Principle 4 — Preserve streaming, in both directions

Do not buffer large LLM requests or responses.

### Principle 5 — Client controls the request

The client decides:

```text
provider
model
provider API key
prompt
parameters
stream
tools
response format
```

### Principle 6 — Configuration controls destination

The server controls:

```text
provider → base URL
```

### Principle 7 — Security boundaries remain explicit

The relay must not become an unrestricted proxy.

### Principle 8 — Resource usage stays bounded and predictable

Concurrency, memory, and disk usage are actively capped rather than left to grow unbounded under load.

### Principle 9 — The relay authenticates access to itself

Independent of the provider credential the client forwards through, the relay owns and enforces its own access key (§11.2). A request without a valid key never reaches provider resolution or the network.

---

# 46. Final Architecture

The recommended final architecture is:

```text
                           Client
                             │
                             │
                             │
                  ┌──────────▼──────────┐
                  │      Axum HTTP      │
                  │       Server        │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │  Auth Middleware    │
                  │ (URL key segment    │
                  │  check, then strip) │
                  │  401 if missing/    │
                  │  incorrect — stop   │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │ Concurrency Limiter │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │    Proxy Handler    │
                  │                     │
                  │ Provider Resolution │
                  │ URL Construction    │
                  │ Header Forwarding   │
                  │ Body Forwarding     │
                  └──────────┬──────────┘
                             │
                             ▼
                  ┌─────────────────────┐
                  │   Reqwest Client    │
                  │                     │
                  │ Direct / SOCKS5     │
                  └──────────┬──────────┘
                             │
            ┌────────────────┼─────────────────┐
            │                │                 │
            ▼                ▼                 ▼
         OpenAI         OpenRouter          Gemini
            │                │                 │
            └────────────────┼─────────────────┘
                             │
                             ▼
                         Response
                             │
                             ▼
                  ┌─────────────────────┐
                  │    Rust LLM Relay   │
                  │                     │
                  │ Status              │
                  │ Headers             │
                  │ Streaming Body      │
                  └──────────┬──────────┘
                             │
                             ▼
                           Client
```

Running under systemd on a single mainstream Linux host, with resource limits enforced via cgroups (§32.2).

---

# 47. Initial Scope

The first production-capable version should contain only:

```text
✓ Axum HTTP server
✓ Relay access keys (auto-generated default key, locally stored as salted hashes, validated on every request)
✓ CLI: `providers list`, `keys list`, `keys generate`, `config show`/`config check`, with configuration-editing steps in `--help`
✓ Provider-based routing
✓ Configurable provider base URLs
✓ HTTP method forwarding
✓ Header forwarding
✓ Request body streaming
✓ Response status forwarding
✓ Response header forwarding
✓ Response body streaming
✓ SSE/LLM streaming support
✓ Request ID
✓ Basic structured logging (journald-capped)
✓ Timeout
✓ Request size limit
✓ Provider allowlist
✓ Concurrency admission control
✓ systemd-based deployment (no container runtime)
✓ Optional SOCKS5 outbound proxy
```

It should **not** contain:

```text
✗ Container runtime or orchestration
✗ Database
✗ User management
✗ Model management
✗ Token accounting
✗ Billing
✗ Model routing
✗ Provider health management
✗ Prompt storage
✗ RAG
✗ Agent orchestration
✗ Automatic model selection
✗ Provider API key storage
```

---

# 48. Summary

The Rust LLM Relay is intentionally designed as a **thin, stateless HTTP forwarding service** that runs comfortably within a 512MB RAM / 10GB disk envelope on a single mainstream Linux host, with no container runtime, while remaining able to absorb high concurrency safely.

Its responsibility can be summarized in one sentence:

> **Receive an LLM HTTP request from the client, forward it to the configured provider without understanding its business semantics, and stream the provider's response back to the client — all within a fixed, bounded resource budget.**

The essential abstraction is:

```text
                 ┌──────────────────┐
                 │      Client      │
                 └────────┬─────────┘
                          │
                          ▼
                 ┌──────────────────┐
                 │   Rust Relay     │
                 │                  │
                 │   Route          │
                 │   Forward        │
                 │   Stream         │
                 └────────┬─────────┘
                          │
                          ▼
                 ┌──────────────────┐
                 │ LLM Provider     │
                 └──────────────────┘
```

This deliberately simple architecture, combined with a static musl binary, systemd-native deployment, and explicit concurrency/memory controls, provides a strong foundation for later additions such as network isolation, authentication middleware, observability, request transformation, or provider failover — without forcing those concerns, or a container runtime, into the initial implementation.

The key architectural decision is to keep the **LLM protocol outside the relay's business logic**, and to keep **resource usage bounded and predictable** rather than assuming an elastic cloud environment underneath it. The relay is an HTTP transport component, not an LLM abstraction layer.
