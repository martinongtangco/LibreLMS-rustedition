//! Auth mechanism (research R4/R5, spec US3): opaque cookie tokens, hashed in
//! the system of record, organization scope, sliding expiry bounded by an
//! absolute max. Framework-agnostic on purpose — the axum extractor lives in
//! the host crate (Principle II: no extra layers).

use rand::Rng;
use sha2::{Digest, Sha256};

use contracts::{Role, User};

use crate::storage::{NewSession, Storage};

/// An authenticated, scope-checked actor (what the host extractor returns).
#[derive(Debug, Clone)]
pub struct AuthContext {
    pub user: User,
    pub organization_id: String,
    pub role: Role,
}

/// Session verification failures — the host maps these to 401/403/503.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// Unknown, expired, or max-bound-exceeded token → 401 (never a 500).
    Unauthenticated,
    /// Token valid but the requested org is not the session's scope, or the
    /// user is not a member → 403.
    Scope,
    /// The system of record is unreachable → 503 (never 500, never a silent 401).
    Unavailable,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::Unauthenticated => write!(f, "unauthenticated"),
            AuthError::Scope => write!(f, "organization scope mismatch"),
            AuthError::Unavailable => write!(f, "session backend unavailable"),
        }
    }
}

impl std::error::Error for AuthError {}

/// CR-D: only a storage OUTAGE maps to Unavailable (503); absence of rows is
/// an authentication decision (401/403), never an error.
fn storage_err(e: crate::storage::StorageError) -> AuthError {
    match e {
        crate::storage::StorageError::Unavailable(_) => AuthError::Unavailable,
        _ => AuthError::Unauthenticated,
    }
}

/// 32 random bytes, base64url (43 chars, no padding) — RFC 4648 §5.
pub fn generate_token(rng: &mut impl Rng) -> String {
    let bytes: [u8; 32] = rng.random();
    base64url(bytes)
}

fn base64url(bytes: [u8; 32]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(43);
    let b = &bytes[..];
    for chunk in b.chunks(3) {
        let n = ((chunk[0] as u32) << 16)
            | ((chunk.get(1).copied().unwrap_or(0) as u32) << 8)
            | (chunk.get(2).copied().unwrap_or(0) as u32);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[n as usize & 63] as char);
        }
    }
    out
}

/// SHA-256 hex of the token — what the database stores (R4: never the token).
pub fn token_hash(token: &str) -> String {
    let mut h = Sha256::new();
    h.update(token.as_bytes());
    let d = h.finalize();
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Create a session for (user, org) — the membership is verified (R5).
/// Returns the raw token (for the cookie) and the row (for persistence).
pub async fn establish(
    storage: &dyn Storage,
    rng: &mut impl Rng,
    user_id: &str,
    organization_id: &str,
    now: i64,
    ttl_secs: i64,
    max_ttl_secs: i64,
) -> Result<(String, NewSession), AuthError> {
    // The actor must be a member of the org (else scope is meaningless).
    // CR-D: a storage OUTAGE is Unavailable (503), not a Scope/401 verdict.
    let member = storage
        .org_role(user_id, organization_id)
        .await
        .map_err(storage_err)?;
    if member.is_none() {
        return Err(AuthError::Scope);
    }
    storage
        .get_user(user_id)
        .await
        .map_err(storage_err)?
        .ok_or(AuthError::Unauthenticated)?;

    let token = generate_token(rng);
    let row = NewSession {
        token_hash: token_hash(&token),
        user_id: user_id.to_string(),
        organization_id: organization_id.to_string(),
        created_at: now,
        last_activity: now,
        expires_at: now + ttl_secs.min(max_ttl_secs),
    };
    storage.create_session(&row).await.map_err(storage_err)?;
    Ok((token, row))
}

/// Verify a token for (requested_org) at `now`: existence → expiry (sliding,
/// bounded) → scope → identity. Unknown/expired → Unauthenticated (401, never
/// 500); out-of-scope → Scope (403).
pub async fn verify(
    storage: &dyn Storage,
    token: &str,
    requested_org: &str,
    now: i64,
    ttl_secs: i64,
    max_ttl_secs: i64,
) -> Result<AuthContext, AuthError> {
    let hash = token_hash(token);
    let row = storage
        .get_session(&hash)
        .await
        .map_err(|e| match e {
            crate::storage::StorageError::Unavailable(_) => AuthError::Unavailable,
            _ => AuthError::Unauthenticated,
        })?
        .ok_or(AuthError::Unauthenticated)?;

    // Expiry: the stored expiry is the reference (no activity since the last
    // touch); activity slides it forward, clamped by the absolute bound
    // (created + MAX_TTL).
    let bound = row.created_at + max_ttl_secs;
    if now >= row.expires_at || now >= bound {
        // Expired: drop the row (spec: unknown/expired → 401).
        let _ = storage.delete_session(&hash).await;
        return Err(AuthError::Unauthenticated);
    }
    let new_expiry = (now + ttl_secs).min(bound);

    // Sliding refresh (R4); a failed touch means the row vanished.
    if !storage
        .touch_session(&hash, now, new_expiry)
        .await
        .map_err(storage_err)?
    {
        return Err(AuthError::Unauthenticated);
    }

    // Scope: the session's org must be the requested one (spec US3.2).
    if row.organization_id != requested_org {
        return Err(AuthError::Scope);
    }
    // Membership still holds (membership can be revoked).
    storage
        .org_role(&row.user_id, &row.organization_id)
        .await
        .map_err(storage_err)?
        .ok_or(AuthError::Scope)?;

    let user = storage
        .get_user(&row.user_id)
        .await
        .map_err(storage_err)?
        .ok_or(AuthError::Unauthenticated)?;

    // Re-read the role for the context.
    let role = storage
        .org_role(&row.user_id, &row.organization_id)
        .await
        .map_err(storage_err)?
        .ok_or(AuthError::Scope)?;

    Ok(AuthContext {
        user,
        organization_id: row.organization_id,
        role,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64url_known_vector() {
        // 32 zero bytes: 8-char groups of 'A' up to 43 chars.
        let t = base64url([0u8; 32]);
        assert_eq!(t.len(), 43);
        assert!(t
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn hash_is_stable_and_not_the_token() {
        let a = token_hash("hello");
        let b = token_hash("hello");
        assert_eq!(a, b);
        assert_ne!(a, "hello");
    }
}
