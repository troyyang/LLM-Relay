# LLM-Relay

Lightweight Rust HTTP relay for LLM provider APIs. It maps
`/proxy/{relay-api-key}/{provider}/{path...}` to an allowlisted provider base
URL, forwards headers and opaque request bytes upstream, then streams the
upstream status, headers, and body back to the client.

## Project Structure

```text
LLM-Relay/
├── Cargo.toml
├── Rust_LLM_HTTP_Relay_Design.md
├── config/
│   └── config.yaml
├── deploy/
│   └── llm-relay.service
└── src/
    ├── config.rs
    ├── error.rs
    ├── lib.rs
    ├── main.rs
    ├── proxy.rs
    └── security.rs
```

## Run

```bash
cargo run -- --config config/config.yaml
```

Or use the start script:

```bash
scripts/start.sh
scripts/start.sh config/config.yaml
```

Health check:

```bash
curl http://localhost:5017/healthz
```

OpenAI example:

```bash
curl \
  "http://localhost:5017/proxy/${LLM_RELAY_API_KEY}/openai/v1/chat/completions" \
  -H "Authorization: Bearer $OPENAI_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"gpt-4o","messages":[{"role":"user","content":"Hello"}]}'
```

OpenRouter example:

```bash
curl \
  "http://localhost:5017/proxy/${LLM_RELAY_API_KEY}/openrouter/api/v1/chat/completions" \
  -H "Authorization: Bearer $OPENROUTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"google/gemini-2.5-flash","messages":[{"role":"user","content":"Hello"}]}'
```

Cloudflare AI Gateway example:

```bash
curl --fail-with-body -sS \
  "http://localhost:5017/proxy/${LLM_RELAY_API_KEY}/cloudflare/v1/${CLOUDFLARE_ACCOUNT}/wispmemo-llm/compat/chat/completions" \
  -H "cf-aig-authorization: Bearer ${CLOUDFLARE_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "google-ai-studio/gemini-3.7-flash",
    "messages": [
      {
        "role": "system",
        "content": "You are a professional, accurate and concise assistant."
      },
      {
        "role": "user",
        "content": "What model are you?"
      }
    ],
    "temperature": 0.7,
    "max_tokens": 1024,
    "stream": false
  }'
```

The bundled `cloudflare` provider maps that relay URL to:

```text
https://gateway.ai.cloudflare.com/v1/${CLOUDFLARE_ACCOUNT}/wispmemo-llm/compat/chat/completions
```

The relay forwards Cloudflare's `cf-aig-authorization` header unchanged and
does not forward the relay API key upstream.

Streaming requests use the same endpoint; response bytes are relayed without
buffering the complete response.

## Configuration

The installed `llm-relay` command and `llm-relay.service` both use
`/etc/llm-relay/config.yaml` by default. The bundled configuration stores its
key at `/etc/llm-relay/api_key`. The package restricts key access to `root` and
the `llm-relay` operator group.

For a source checkout, pass the bundled configuration explicitly:

```bash
cargo run -- --config config/config.yaml
cargo run -- show-key --config config/config.yaml
```

Set `LLM_RELAY_CONFIG=/path/to/config.yaml` to use a custom configuration
without repeating `--config`.

Key settings:

- `server.max_concurrent_requests`: global admission-control limit.
- `server.max_body_size`: request body limit, such as `"8MB"`.
- `runtime.worker_threads`: explicit Tokio worker thread count.
- `timeout.connect` / `timeout.request`: outbound HTTP timeouts in seconds.
- `proxy.url`: optional outbound HTTP/SOCKS proxy.
- `security.api_key_file`: local file used to persist the relay API key. Relative
  paths are resolved beside the configuration file.
- `providers`: allowlisted provider base URLs.

Use `--api-key-file /path/to/api_key` or
`LLM_RELAY_API_KEY_FILE=/path/to/api_key` to override key storage for `run`,
`generate-key`, and `show-key`. Relative command-line key paths are resolved
beside the selected config file.

On first startup, the relay generates a 256-bit API key using the operating
system random source and stores it with restrictive permissions. Proxy requests
must include the key in the URL as
`/proxy/{relay-api-key}/{provider}/{path...}`. The relay API key is checked
locally, redacted from relay logs, and never forwarded upstream. Provider
credentials are not stored by the relay; clients send provider `Authorization`
headers, and the relay forwards them upstream.

Private or local provider IPs are rejected by default. For an intentional local
upstream, set `allow_private: true` on that provider.

## Build

Debug build:

```bash
cargo build
```

Static Linux release build:

```bash
rustup target add x86_64-unknown-linux-musl
cargo build --release --target x86_64-unknown-linux-musl
```

The release profile is tuned for a small static binary. With musl and rustls,
the binary should not depend on the target host's OpenSSL or glibc versions.

Debian/Ubuntu package:

```bash
rustup target add x86_64-unknown-linux-musl
scripts/package-deb.sh
sudo apt install ./target/package/llm-relay_0.1.1_amd64.deb
```

On macOS and other hosts without `dpkg-deb`, the packaging script uses Docker
automatically. Install and start Docker Desktop, then run the same command.

The installer generates the relay API key, installs `llm-relay.service`, and
enables and starts it automatically when systemd is running. View the generated
key with `sudo llm-relay show-key --config /etc/llm-relay/config.yaml`.

When the package is installed through `sudo`, it adds the invoking user to the
`llm-relay` group. Sign out and back in before running `llm-relay show-key`
without `sudo`. To authorize another operator:

```bash
sudo usermod -aG llm-relay <user>
```

## Installed Commands

After installing the binary and `deploy/llm-relay.service`:

```bash
sudo llm-relay generate-key
sudo llm-relay start
sudo llm-relay status
sudo llm-relay logs
sudo llm-relay restart
sudo llm-relay stop
```

Key management:

```bash
sudo llm-relay generate-key --force
sudo llm-relay restart
export LLM_RELAY_API_KEY="$(llm-relay show-key)"
```

Use `--config /path/to/config.yaml` with `run`, `generate-key`, or
`show-key` when the configuration is not at `/etc/llm-relay/config.yaml`.
Use `--api-key-file /path/to/api_key` when the key location must differ from
`security.api_key_file` in that configuration.

## Test

```bash
cargo test
```

## Operational Notes

- The relay is stateless and does not parse LLM JSON payloads.
- Hop-by-hop headers such as `Connection`, `Host`, `Transfer-Encoding`, and
  `Content-Length` are not forwarded.
- Upstream HTTP statuses are returned as-is; relay-level failures use `4xx`,
  `502`, `503`, or `504` as appropriate.
- Logs go to stdout/stderr through `tracing`; use systemd/journald limits to cap
  disk usage in production.
