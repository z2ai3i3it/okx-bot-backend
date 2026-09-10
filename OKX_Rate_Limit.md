---
name: okx-rate-limits
description: Reference knowledge for OKX (okx.com) v5 API rate limits — REST, WebSocket (public/private), order placement/amend/cancel throttling, sub-account caps, VIP/fill-ratio tiers, and error codes 50011/50061. Use this skill whenever the user asks about OKX API rate limits, throttling, "how many orders can I send", designing/debugging an OKX trading bot or market-data feed, sizing request frequency for OKX endpoints, or troubleshooting OKX error 50011 or 50061 — even if they don't say the words "rate limit" explicitly (e.g. "my OKX bot keeps getting rejected", "how fast can I poll OKX", "best way to place many orders on OKX quickly").
---

# OKX v5 API Rate Limits

Reference knowledge distilled from OKX's official docs (`https://www.okx.com/docs-v5/en/`). Use this to answer questions or design systems around OKX's rate limits without re-deriving them from scratch. Always caveat that endpoint-specific numbers should be spot-checked against the live docs page for the exact endpoint in question, since OKX updates these periodically (see `references/changelog-notes.md` pattern below — check the "Upcoming Changes" log if the user needs certainty for production).

## Core mental model

OKX enforces rate limits per **endpoint**, using one of four counting rules:

| Rule | Applies to | Counted per |
|---|---|---|
| IP address | Public/unauthenticated REST | IP |
| User ID | Private REST | User ID (each sub-account has its own) |
| User ID | WebSocket order management (place/amend/cancel via WS) | User ID |
| Connection | WebSocket login/subscribe/unsubscribe | Per WS connection |

Breaching any limit returns **error code 50011** ("Rate limit reached"). The trading-specific sub-account cap instead returns **50061**.

## 1. REST API — general endpoints

Every endpoint has its own published limit (check the specific endpoint's docs page for the exact number). Typical shape, e.g.:

| Endpoint example | Limit | Rule |
|---|---|---|
| GET /account/instruments | 20 req / 2s | User ID + Instrument Type |
| GET /account/balance | 10 req / 2s | User ID |
| GET /account/positions | 10 req / 2s | User ID |
| GET /account/bills (7d) | 5 req / 1s | User ID |
| GET /account/bills-archive (3mo) | 5 req / 2s | User ID |
| POST /account/bills-history-archive (apply) | 1 req / 10s | User ID |
| POST /account/set-leverage | 20 req / 2s | User ID |
| Most public market-data endpoints (tickers, order book, candles) | ~20 req / 2s | IP |

Rule of thumb for market data polling: stay comfortably under 20 requests/2s per IP, or better — use WebSocket public channels instead of REST polling.

## 2. Trading-related APIs (Place / Amend / Cancel order) — the part that matters most for bots

These follow special combined rules, independent of the general REST table above:

- **Shared quota across REST and WebSocket** — sending via WS does not give you a separate bucket from REST.
- Place, Amend, and Cancel each have **independent** quotas from one another.
- Quota is scoped to **Instrument ID** (except Options, which are scoped to **Instrument Family**).
- Single-order endpoint and multi-order (batch) endpoint have **independent** quotas — except if you send only 1 order to the batch endpoint, it's counted as a single order.
- An order can have **at most 3 amend requests in flight** simultaneously; a 4th is rejected with error `51513`.
- Standard default: **Single order — 60 requests / 2 seconds** per Instrument ID (= ~30 orders/sec, or ~33 ms/order sustained). **Batch order (up to 20 orders/request) — 300 requests / 2 seconds** on the batch endpoint itself.
- **Colocation clients get 4× the standard limits.**
- A special sub-case: orders tagged `rpiTakerAccess:true` are capped at **50 orders / 2s per User ID per Instrument ID**, shared across the single and batch place-order endpoints.

### Sub-account level cap (applies on top of the per-instrument cap, in parallel — not instead of it)

- **1,000 order requests / 2 seconds** per sub-account (counts new + amend only; cancel is free). Batch requests count each order inside individually.
- Exceeding this → error **50061** (distinct from 50011).
- Master account counts as a sub-account too for this purpose.
- To scale beyond this: **trade via multiple sub-accounts** (OKX's official recommendation) — each sub-account gets its own independent 1,000/2s pool.

### Fill-ratio based tier upgrade (VIP5+ only)

Sub-account cap can be raised based on a rolling 7-day fill ratio = (USDT trade volume) / (Σ new+amend order count × symbol multiplier), recalculated daily at 00:00 UTC, applied at 08:00 UTC:

| Tier | Fill ratio range | Sub-account limit /2s |
|---|---|---|
| 1 | [0, 1) | 1,000 |
| 2 | [1, 2) | 1,250 |
| 3 | [2, 3) | 1,500 |
| 4 | [3, 5) | 1,750 |
| 5 | [5, 10) | 2,000 |
| 6 | [10, 20) | 2,500 |
| 7 | [20, 50) | 3,000 |
| 8 | ≥ 50 | 10,000 |

Notes: upgrades apply immediately; downgrades get a 1-day grace period. New sub-accounts start at Tier 1 until T+1 08:00 UTC. Use the max of the sub-account's own ratio and the master account's aggregated ratio. Query your live numbers via `GET /api/v5/account/rate-limit` (updated daily 08:00 UTC) — always check this endpoint before hard-coding a throttle value for a production bot.

## 3. WebSocket — general rules (apply to both public and private)

| Rule | Value |
|---|---|
| New connection attempts | 3 requests/sec, per IP |
| subscribe + unsubscribe + login combined | 480 per hour, per connection |
| Idle disconnect | Connection auto-closes if no data/message received for 30s — implement ping/pong (send text `"ping"`, expect `"pong"`) with a timer < 30s |
| Connection count limit | 30 WS connections per specific channel per sub-account — applies only to: Orders, Account, Positions, Balance and positions, Position risk warning, Account greeks channels. Order placement/amend/cancel over WS is NOT affected by this cap. |

Production URLs:
- REST: `https://openapi.okx.com` (or `https://www.okx.com`)
- Public WS: `wss://ws.okx.com:8443/ws/v5/public`
- Private WS: `wss://ws.okx.com:8443/ws/v5/private`
- Business WS (grid/algo/etc.): `wss://ws.okx.com:8443/ws/v5/business`

## 4. Private WebSocket specifics

- Requires login first (HMAC-SHA256 signed, same scheme as REST auth).
- **Order management over private WS (place/amend/cancel) shares the exact same quota as REST** — see section 2. There is no separate/additive WS-only order quota.
- Channels like `orders`, `account`, `positions` are subject to the 30-connections-per-channel cap above.

## 5. Public WebSocket specifics

- No authentication needed — market data only (tickers, order book, candlesticks, mark price, funding rate, etc.).
- Subject only to the general WS rules in section 3 (connect 3/sec/IP, subscribe 480/hr/connection). No order-level quota applies since no trading happens here.
- Preferred over REST polling for market data — reduces REST quota pressure and gives lower latency.

## 6. Quick worked example (sizing a bot's firing rate)

Given a bot that wants to fire orders faster than X ms apart on one instrument, default (non-VIP5+) limits:

1. Per-instrument Place-order cap: 60/2s → sustainable interval ≈ **33 ms/order** on that single instrument. Faster than that → 50011 within the rolling 2s window.
2. Sub-account cap: 1,000/2s → sustainable interval ≈ **2 ms/order** aggregated across all instruments on that sub-account.
3. So for a single-instrument strategy, #1 is almost always the binding constraint, not #2.
4. To exceed #1's ceiling: spread across more Instrument IDs (each gets its own 60/2s bucket), use the batch endpoint, split across sub-accounts, raise fill ratio (VIP5+), or use colocation (4×).

## 7. Error code cheat-sheet

| Code | Meaning |
|---|---|
| 50011 | Generic rate limit reached — endpoint/instrument-level |
| 50061 | Sub-account order rate limit exceeded |
| 51513 | Too many concurrent amend requests on one order (>3 in flight) |
| 60009 | WebSocket login failed |

## When answering questions with this skill

- Always distinguish which of the 3 limit "layers" (per-endpoint/instrument, sub-account, VIP-tier) is the actual bottleneck for the user's described workload — most questions like "how many orders can I send" hinge on this distinction, not a single flat number.
- If the user needs a production-grade guarantee, point them to `GET /api/v5/account/rate-limit` to read their live, current values rather than relying purely on published defaults.
- Note that OKX explicitly documents "the rate limit is different for each endpoint" — if the user names a specific non-trading endpoint not listed above, say the general figures given here are typical but recommend checking that endpoint's own docs entry for the exact number rather than asserting a number with false confidence.