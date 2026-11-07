# Data Model: Application Foundation (Phase 1)

## System of record (MSSQL)

### `users`
| column | type | notes |
|---|---|---|
| id | UNIQUEIDENTIFIER PK | |
| email | NVARCHAR(320) UNIQUE NOT NULL | |
| name | NVARCHAR(200) NOT NULL | |
| password_hash | NVARCHAR(500) NOT NULL | argon2/bcrypt — choice fixed in tasks |
| theme | NVARCHAR(20) | default 'default' |
| created_at | DATETIME2 NOT NULL | |

### `organizations`
| column | type | notes |
|---|---|---|
| id | UNIQUEIDENTIFIER PK | |
| name | NVARCHAR(200) NOT NULL | |

### `user_organizations` (membership + role)
| column | type | notes |
|---|---|---|
| user_id | FK → users | |
| organization_id | FK → organizations | |
| role | NVARCHAR(40) NOT NULL | `admin` \| `member` |
| PK | (user_id, organization_id) | |

### `sessions`
| column | type | notes |
|---|---|---|
| token_hash | NVARCHAR(64) PK | sha256 of the cookie token |
| user_id | FK → users | |
| organization_id | FK → organizations | the active scope for this session |
| created_at | DATETIME2 NOT NULL | |
| last_activity | DATETIME2 NOT NULL | sliding-expiry clock |
| expires_at | DATETIME2 NOT NULL | absolute bound |

### `schema_migrations` (R1)
| column | type | notes |
|---|---|---|
| name | NVARCHAR(200) PK | script file name |
| checksum | NVARCHAR(64) NOT NULL | |
| applied_at | DATETIME2 NOT NULL | |

A seeder (part of the migration step) creates one development user
(`dev@example.com`) and one development organization, and links them (`admin` role) —
the seeder is idempotent (spec Assumptions).

## Ephemeral (Valkey) — nothing durable (Principle VI)

| key | value | TTL |
|---|---|---|
| `lms:session:{token_hash}:bag` | JSON object of per-session ephemeral state (later: the SCORM `cmi.*` bag) | session lifetime, refreshed with the sliding expiry |

A store flush loses only the bag; the session (DB) survives (spec story 3, scenario 4).

## Contract shapes (`contracts` crate)

Typed, `serde`-serializable, no dependencies beyond `serde`:

- `User { id, email, name, theme }`
- `Organization { id, name }`
- `Membership { user_id, organization_id, role }`
- `Course { id, title, description, organization_id }` *(placeholder — populated by the Catalog slice)*
- `Enrollment { id, user_id, course_id, organization_id, status }` *(placeholder — Enrollment slice)*
- `Completion { enrollment_id, completed_at }` *(placeholder — Enrollment slice)*
- `ScormAttemptStatus { session_id, course_id, state, suspend_data, score_raw, session_time }`
  *(placeholder — SCORM slice; the **live** `cmi.*` bag lives in Valkey (`lms:session:{token_hash}:bag`, not in this row); **committed** state (final status, score) persists to MSSQL on LMSCommit/LMSFinish — Principle VI)*

Rule (Principle III): module crates consume these shapes via `contracts` only; the
boundary check fails the build on module→module dependencies.
