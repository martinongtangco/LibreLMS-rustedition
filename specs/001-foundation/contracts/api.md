# Contracts: Application Foundation (Phase 1)

Observable surface of the foundation slice. Module slices extend (never change) these.

## HTTP

### `GET /healthz` (JSON)
```json
{
  "status": "ok",              // "ok" | "degraded"
  "process": "alive",
  "database": "reachable",     // "reachable" | "unreachable" | "schema-not-ready"
  "session_store": "reachable" // "reachable" | "unreachable"
}
```
- 200 when `status=ok`; 503 when `status=degraded` (process alive but a backend is
  down) — the process keeps serving (spec story 1, scenario 4).
- No auth required.

### `GET /` (server-rendered, askama)
- 200 with the home page for everyone (public).

### Unknown routes
- 404, plain body, no stack traces / internal paths (spec edge case).

### Auth mechanism (cookie)
- Cookie: `lms_session`, `HttpOnly`, `Secure` (when served over HTTPS), `SameSite=Lax`,
  path `/`, Max-Age = configured lifetime.
- Unauthenticated request to a protected route → 401 (JSON) or redirect to `/signin`
  (HTML) — login *pages* are a later slice; the mechanism (session verification,
  sliding expiry, scope check) is this slice.
- Authenticated request outside the session's organization scope → 403 with
  `{"error":"scope"}` (spec story 3, scenario 2).
- Expired or unknown token → 401, never a 500 (spec edge case).
- Session store down at runtime + session-dependent request → 503 `{"error":"session-store-unavailable"}`.

## Error envelope (JSON, all module endpoints in later slices)
```json
{ "error": "<stable-code>", "message": "human-readable, no secrets" }
```
Stable codes introduced by this slice: `unauthenticated`, `scope`,
`session-store-unavailable`. Later slices add their own codes; existing codes are never
reassigned.

## Configuration (environment)

| variable | required | meaning |
|---|---|---|
| `DATABASE_URL` | yes | MSSQL connection (fail-fast if missing/malformed, naming the item — FR-001) |
| `VALKEY_URL` | yes | session store address |
| `PORT` | no (default 5000) | bind port; bind failure → fail-fast naming the port |
| `SESSION_TTL_SECS` | no (default 28800) | sliding lifetime |
| `SESSION_MAX_TTL_SECS` | no (default 604800) | absolute bound |
| `LOG_LEVEL` | no (default `info`) | tracing filter |

Missing/malformed `DATABASE_URL` or `VALKEY_URL` → exit non-zero within 5 s, message
names the item (SC-002). Secrets in these values are never logged (FR-009).

## Module boundary contract

`crates/{catalog,enrollment,scorm,management}` may depend on `contracts`, `shared`, and
the Rust ecosystem — never on each other. Enforced by
`.github/scripts/check-boundaries.sh` in CI and locally (FR-006, Principle III).
