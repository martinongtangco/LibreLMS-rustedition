/// Management migration manifest (FR-013 — empty BY CONSTRUCTION).
///
/// The module owns 001-foundation's `dbo.users` / `dbo.organization_members` /
/// `dbo.organizations` / `dbo.sessions` tables (read/written through the
/// 001 seam) and defines **no new tables** — so its manifest is empty.
/// The host still concatenates it, so the whole-manifest contract
/// (001) holds uniformly for every module: the runner applies `&[]`
/// as a verified no-op (T013: empty manifest → run → `applied == []`).
pub const MIGRATIONS: &[(&str, &str)] = &[];
