//! HTTPS JSON API over [`TenantManager`] (feature `server`).
//!
//! ```text
//! POST /v1/login            {tenant?, user, password}      -> {token, expires_in}
//! POST /v1/logout           (Bearer)
//! GET  /v1/me               (Bearer)                       -> principal
//! POST /v1/me/password      {old, new}
//! POST /v1/query            {query, params?}               -> {columns, rows, stats, ts}
//! POST /v1/checkpoint       (tenant_admin)
//! POST /v1/admin/tenants    {name, max_entities?, max_result_rows?}   (platform_admin)
//! POST /v1/admin/tenants/{name}/disable | /enable                      (platform_admin)
//! DELETE /v1/admin/tenants/{name}                                      (platform_admin, tenant disabled)
//! POST /v1/admin/users      {tenant?, name, password, role}
//! POST /v1/admin/users/disable        {tenant?, name, disabled}
//! POST /v1/admin/users/reset-password {tenant?, name, new_password}
//! GET  /v1/audit?limit=&tenant=
//! GET  /v1/bi/tables                [?as_of=ts]            -> catalog: tables, columns, types, row counts   (Bearer or Basic)
//! GET  /v1/bi/tables/{name}         [?format=csv|json&as_of=ts]  -> one flat table (see `bi`)                   (Bearer or Basic)
//! GET|POST /v1/bi/query             q=NQL | {query, params}  [?format=csv|json]  read-only statements only     (Bearer or Basic)
//! GET  /v1/bi/powerquery            [?base=https://host:port]  -> Power Query (M) script for Power BI          (no auth)
//! GET  /healthz
//! ```
//! Every other path is 404. Errors are `{"error": "..."}`; internal details are logged, never sent.

use crate::error::Error;
use crate::query::Params;
use crate::security::{Principal, Quotas, Role, Session, TenantManager};
use crate::value::Value;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Path, Query, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{middleware, Json, Router};
use serde_json::{json, Value as J};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// Maximum request body.
    pub max_body_bytes: usize,
    /// Responses are 504 after this long (the engine call itself cannot be cancelled).
    pub request_timeout: Duration,
    /// Engine/control-plane calls running at once; the rest wait.
    pub max_concurrent: usize,
    /// Login attempts per client IP per minute (credential-stuffing brake; per-account lockout is separate).
    pub logins_per_ip_per_minute: u32,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig { max_body_bytes: 1 << 20, request_timeout: Duration::from_secs(30), max_concurrent: 16, logins_per_ip_per_minute: 20 }
    }
}

struct App {
    mgr: Arc<TenantManager>,
    cfg: ServerConfig,
    sem: Semaphore,
    logins: Mutex<HashMap<IpAddr, (Instant, u32)>>,
    /// HTTP Basic credentials (hashed) -> issued token, so BI refreshes do not run Argon2id on every request.
    basic: Mutex<HashMap<String, (String, Instant)>>,
}

/// Longest a Basic credential's token is reused (shorter than the token's own lifetime; a changed or disabled
/// account stops working within this time even through the cache, and a disabled user is rejected immediately
/// because the token itself is re-checked on every request).
const BASIC_CACHE_TTL: Duration = Duration::from_secs(300);
const BASIC_CACHE_MAX: usize = 1024;

impl App {
    /// Count one login attempt for `ip`; false when the per-minute budget is spent.
    fn login_allowed(&self, ip: IpAddr) -> bool {
        let mut g = self.logins.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        g.retain(|_, (t, _)| now.duration_since(*t) < Duration::from_secs(60));
        let e = g.entry(ip).or_insert((now, 0));
        e.1 += 1;
        e.1 <= self.cfg.logins_per_ip_per_minute
    }
}

type St = State<Arc<App>>;

struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

fn reply(code: StatusCode, body: J) -> Response {
    (code, Json(body)).into_response()
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        match self.0 {
            Error::Auth(_) => {
                let mut r = reply(StatusCode::UNAUTHORIZED, json!({"error": "invalid credentials"}));
                r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
                // BI tools (Power BI) answer a Basic challenge; the first header stays Bearer for API clients.
                r.headers_mut().append(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"nexgraph\", charset=\"UTF-8\""));
                r
            }
            Error::Denied(m) => reply(StatusCode::FORBIDDEN, json!({"error": m})),
            Error::Parse { .. } | Error::Query(_) | Error::Invalid(_) => reply(StatusCode::BAD_REQUEST, json!({"error": self.0.to_string()})),
            other => {
                eprintln!("nexgraph-server: internal error: {other}");
                reply(StatusCode::INTERNAL_SERVER_ERROR, json!({"error": "internal error"}))
            }
        }
    }
}

type Res = Result<Response, ApiError>;

fn to_json(v: &Value) -> J {
    match v {
        Value::Null => J::Null,
        Value::Bool(b) => J::Bool(*b),
        Value::Int(i) => json!(i),
        Value::Float(f) => serde_json::Number::from_f64(*f).map(J::Number).unwrap_or_else(|| J::String(f.to_string())),
        Value::Str(s) => J::String(s.clone()),
        Value::List(l) => J::Array(l.iter().map(to_json).collect()),
        Value::Vector(v) => J::Array(v.iter().map(|x| to_json(&Value::Float(*x as f64))).collect()),
        Value::Node(id) => json!({"$node": id}),
        Value::Edge(id) => json!({"$edge": id}),
    }
}

fn from_json(j: &J) -> Result<Value, Error> {
    Ok(match j {
        J::Null => Value::Null,
        J::Bool(b) => Value::Bool(*b),
        J::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().ok_or_else(|| Error::Invalid("number out of range".into()))?),
        },
        J::String(s) => Value::Str(s.clone()),
        J::Array(a) => Value::List(a.iter().map(from_json).collect::<Result<_, _>>()?),
        J::Object(_) => return Err(Error::Invalid("objects are not valid parameter values".into())),
    })
}

fn body<T: serde::de::DeserializeOwned>(b: &Bytes) -> Result<T, ApiError> {
    serde_json::from_slice(b).map_err(|e| Error::Invalid(format!("invalid JSON body: {e}")).into())
}

fn bearer(h: &HeaderMap) -> Result<String, ApiError> {
    let v = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).ok_or_else(|| Error::Auth("missing token".into()))?;
    match v.strip_prefix("Bearer ") {
        Some(t) if !t.is_empty() => Ok(t.trim().to_string()),
        _ => Err(Error::Auth("malformed Authorization header".into()).into()),
    }
}

async fn blocking<T: Send + 'static>(app: &Arc<App>, f: impl FnOnce() -> Result<T, Error> + Send + 'static) -> Result<T, ApiError> {
    let _permit = app.sem.acquire().await.map_err(|_| Error::Backend("server is shutting down".into()))?;
    match tokio::time::timeout(app.cfg.request_timeout, tokio::task::spawn_blocking(f)).await {
        Err(_) => Err(Error::Backend("request timed out".into()).into()),
        Ok(Err(e)) => Err(Error::Backend(format!("worker failed: {e}")).into()),
        Ok(Ok(r)) => r.map_err(Into::into),
    }
}

async fn principal(app: &Arc<App>, h: &HeaderMap) -> Result<(String, Principal), ApiError> {
    let token = bearer(h)?;
    let (cp, t) = (app.mgr.control_plane().clone(), token.clone());
    let p = blocking(app, move || cp.authenticate(&t)).await?;
    Ok((token, p))
}

fn p_json(p: &Principal) -> J {
    json!({"user": p.user, "tenant": p.tenant, "role": p.role.as_str()})
}

// ------------------------------------------------------------------ handlers

async fn healthz() -> &'static str {
    "ok"
}

#[derive(serde::Deserialize)]
struct LoginReq {
    tenant: Option<String>,
    user: String,
    password: String,
}

async fn login(State(app): St, ConnectInfo(addr): ConnectInfo<SocketAddr>, b: Bytes) -> Res {
    if !app.login_allowed(addr.ip()) {
        let mut r = reply(StatusCode::TOO_MANY_REQUESTS, json!({"error": "too many login attempts"}));
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        return Ok(r);
    }
    let r: LoginReq = body(&b)?;
    let cp = app.mgr.control_plane().clone();
    let t = blocking(&app, move || cp.login(r.tenant.as_deref().filter(|t| !t.is_empty()), &r.user, &r.password)).await?;
    Ok(reply(StatusCode::OK, json!({"token": t.token, "expires_in": t.expires_in_secs})))
}

async fn logout(State(app): St, h: HeaderMap) -> Res {
    let (token, _) = principal(&app, &h).await?;
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.logout(&token)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

async fn me(State(app): St, h: HeaderMap) -> Res {
    let (_, p) = principal(&app, &h).await?;
    Ok(reply(StatusCode::OK, p_json(&p)))
}

#[derive(serde::Deserialize)]
struct PwReq {
    old: String,
    new: String,
}

async fn change_password(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let r: PwReq = body(&b)?;
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.change_password(&p, &r.old, &r.new)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

#[derive(serde::Deserialize)]
struct QueryReq {
    query: String,
    #[serde(default)]
    params: HashMap<String, J>,
}

async fn query(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let token = bearer(&h)?;
    let r: QueryReq = body(&b)?;
    let params: Params = r.params.iter().map(|(k, v)| from_json(v).map(|v| (k.clone(), v))).collect::<Result<_, _>>()?;
    let mgr = app.mgr.clone();
    let out = blocking(&app, move || mgr.session(&token)?.query(&r.query, &params)).await?;
    let s = &out.stats;
    Ok(reply(
        StatusCode::OK,
        json!({
            "columns": out.columns,
            "rows": out.rows.iter().map(|r| r.iter().map(to_json).collect::<Vec<_>>()).collect::<Vec<_>>(),
            "stats": {"nodes_created": s.nodes_created, "edges_created": s.edges_created, "props_set": s.props_set, "deleted": s.deleted, "indexes_created": s.indexes_created},
            "ts": out.ts,
        }),
    ))
}

async fn checkpoint(State(app): St, h: HeaderMap) -> Res {
    let token = bearer(&h)?;
    let mgr = app.mgr.clone();
    let ts = blocking(&app, move || mgr.session(&token)?.checkpoint()).await?;
    Ok(reply(StatusCode::OK, json!({"ts": ts})))
}

#[derive(serde::Deserialize)]
struct TenantReq {
    name: String,
    max_entities: Option<u64>,
    max_result_rows: Option<u64>,
}

async fn create_tenant(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let r: TenantReq = body(&b)?;
    let cp = app.mgr.control_plane().clone();
    let t = blocking(&app, move || cp.create_tenant(&p, &r.name, Quotas { max_entities: r.max_entities, max_result_rows: r.max_result_rows })).await?;
    Ok(reply(StatusCode::CREATED, json!({"id": t.id, "name": t.name})))
}

async fn tenant_state(State(app): St, h: HeaderMap, Path((name, action)): Path<(String, String)>) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let disabled = match action.as_str() {
        "disable" => true,
        "enable" => false,
        _ => return Ok(reply(StatusCode::NOT_FOUND, json!({"error": "not found"}))),
    };
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.set_tenant_disabled(&p, &name, disabled)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

async fn purge_tenant(State(app): St, h: HeaderMap, Path(name): Path<String>) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let mgr = app.mgr.clone();
    blocking(&app, move || mgr.purge_tenant(&p, &name)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

#[derive(serde::Deserialize)]
struct UserReq {
    tenant: Option<String>,
    name: String,
    password: String,
    role: String,
}

async fn create_user(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let r: UserReq = body(&b)?;
    let role = Role::parse(&r.role)?;
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.create_user(&p, r.tenant.as_deref().filter(|t| !t.is_empty()), &r.name, &r.password, role)).await?;
    Ok(reply(StatusCode::CREATED, json!({"ok": true})))
}

#[derive(serde::Deserialize)]
struct DisableReq {
    tenant: Option<String>,
    name: String,
    disabled: bool,
}

async fn user_disable(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let r: DisableReq = body(&b)?;
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.set_user_disabled(&p, r.tenant.as_deref().filter(|t| !t.is_empty()), &r.name, r.disabled)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

#[derive(serde::Deserialize)]
struct ResetReq {
    tenant: Option<String>,
    name: String,
    new_password: String,
}

async fn reset_password(State(app): St, h: HeaderMap, b: Bytes) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let r: ResetReq = body(&b)?;
    let cp = app.mgr.control_plane().clone();
    blocking(&app, move || cp.reset_password(&p, r.tenant.as_deref().filter(|t| !t.is_empty()), &r.name, &r.new_password)).await?;
    Ok(reply(StatusCode::OK, json!({"ok": true})))
}

#[derive(serde::Deserialize)]
struct AuditQ {
    limit: Option<i64>,
    tenant: Option<String>,
}

async fn audit(State(app): St, h: HeaderMap, Query(q): Query<AuditQ>) -> Res {
    let (_, p) = principal(&app, &h).await?;
    let cp = app.mgr.control_plane().clone();
    let rows = blocking(&app, move || cp.read_audit(&p, q.tenant.as_deref(), q.limit.unwrap_or(100))).await?;
    let out: Vec<J> = rows.iter().map(|e| json!({"id": e.id, "ts": e.ts, "tenant_id": e.tenant_id, "actor": e.actor, "role": e.role, "action": e.action, "outcome": e.outcome, "detail": e.detail})).collect();
    Ok(reply(StatusCode::OK, json!({"entries": out})))
}

// ------------------------------------------------------------------ BI (Power BI and other flat-table consumers)

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Where a token came from, so a stale cached one can be replaced once.
enum Cred {
    Bearer(String),
    /// (cache key, token, came from the cache)
    Basic(String, String, bool),
}

impl Cred {
    fn token(&self) -> &str {
        match self {
            Cred::Bearer(t) | Cred::Basic(_, t, _) => t,
        }
    }
}

/// `Authorization: Bearer <token>` or `Basic base64(user:password)` where user is `tenant/user`.
/// Basic exists because Power BI's scheduled refresh can only send static credentials.
async fn credentials(app: &Arc<App>, h: &HeaderMap, ip: IpAddr, fresh: bool) -> Result<Cred, ApiError> {
    let v = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).ok_or_else(|| Error::Auth("missing credentials".into()))?;
    if let Some(t) = v.strip_prefix("Bearer ") {
        return if t.trim().is_empty() { Err(Error::Auth("malformed Authorization header".into()).into()) } else { Ok(Cred::Bearer(t.trim().to_string())) };
    }
    let Some(b64) = v.strip_prefix("Basic ") else { return Err(Error::Auth("unsupported Authorization scheme".into()).into()) };
    let key = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(v.as_bytes()))
    };
    if !fresh {
        let mut g = app.basic.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((tok, exp)) = g.get(&key) {
            if *exp > Instant::now() {
                return Ok(Cred::Basic(key, tok.clone(), true));
            }
        }
        g.remove(&key);
    }
    let raw = b64_decode(b64.trim()).and_then(|b| String::from_utf8(b).ok()).ok_or_else(|| Error::Auth("malformed Basic credentials".into()))?;
    let (user, pass) = raw.split_once(':').ok_or_else(|| Error::Auth("malformed Basic credentials".into()))?;
    let (tenant, user) = match user.split_once('/') {
        Some((t, u)) => (Some(t.to_string()), u.to_string()),
        None => (None, user.to_string()),
    };
    if !app.login_allowed(ip) {
        return Err(Error::Auth("too many login attempts".into()).into());
    }
    let (cp, pass) = (app.mgr.control_plane().clone(), pass.to_string());
    let t = blocking(app, move || cp.login(tenant.as_deref().filter(|t| !t.is_empty()), &user, &pass)).await?;
    let ttl = Duration::from_secs(t.expires_in_secs).min(BASIC_CACHE_TTL);
    {
        let mut g = app.basic.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        g.retain(|_, (_, e)| *e > now);
        if g.len() >= BASIC_CACHE_MAX {
            g.clear();
        }
        g.insert(key.clone(), (t.token.clone(), now + ttl));
    }
    Ok(Cred::Basic(key, t.token, false))
}

/// Run `f` as the authenticated user. A cached Basic token that the server no longer accepts is replaced once.
async fn with_session<T: Send + 'static>(app: &Arc<App>, h: &HeaderMap, ip: IpAddr, f: impl Fn(&Session) -> Result<T, Error> + Send + Sync + 'static) -> Result<T, ApiError> {
    let f = Arc::new(f);
    let mut fresh = false;
    loop {
        let cred = credentials(app, h, ip, fresh).await?;
        let (mgr, tok, f2) = (app.mgr.clone(), cred.token().to_string(), f.clone());
        match blocking(app, move || f2(&mgr.session(&tok)?)).await {
            Err(ApiError(Error::Auth(_))) => {
                if let Cred::Basic(key, _, true) = cred {
                    app.basic.lock().unwrap_or_else(|e| e.into_inner()).remove(&key);
                    if !fresh {
                        fresh = true;
                        continue;
                    }
                }
                return Err(Error::Auth("invalid credentials".into()).into());
            }
            other => return other,
        }
    }
}

#[derive(serde::Deserialize)]
struct BiParams {
    format: Option<String>,
    as_of: Option<u64>,
    q: Option<String>,
}

#[derive(Clone, Copy)]
enum Fmt {
    Csv,
    Json,
}

fn fmt_of(f: &Option<String>) -> Result<Fmt, ApiError> {
    match f.as_deref() {
        None | Some("csv") => Ok(Fmt::Csv),
        Some("json") => Ok(Fmt::Json),
        Some(o) => Err(Error::Invalid(format!("format must be csv or json, not {o:?}")).into()),
    }
}

fn table_reply(body: String, f: Fmt, ts: u64) -> Response {
    let ct = match f {
        Fmt::Csv => "text/csv; charset=utf-8",
        Fmt::Json => "application/json",
    };
    let mut r = (StatusCode::OK, body).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(ct));
    r.headers_mut().insert(HeaderName::from_static("x-nexgraph-ts"), HeaderValue::from(ts));
    r
}

async fn bi_catalog(State(app): St, ConnectInfo(addr): ConnectInfo<SocketAddr>, h: HeaderMap, Query(p): Query<BiParams>) -> Res {
    let as_of = p.as_of;
    let (tables, ts) = with_session(&app, &h, addr.ip(), move |s| s.bi_tables(as_of)).await?;
    Ok(table_reply(crate::bi::catalog_json(&tables, ts), Fmt::Json, ts))
}

async fn bi_table(State(app): St, ConnectInfo(addr): ConnectInfo<SocketAddr>, h: HeaderMap, Path(name): Path<String>, Query(p): Query<BiParams>) -> Res {
    let f = fmt_of(&p.format)?;
    let as_of = p.as_of;
    let (t, ts) = with_session(&app, &h, addr.ip(), move |s| s.bi_table(&name, as_of)).await?;
    let r = t.to_result(ts);
    Ok(table_reply(
        match f {
            Fmt::Csv => r.to_csv(),
            Fmt::Json => r.to_json(),
        },
        f,
        ts,
    ))
}

async fn bi_run(app: &Arc<App>, addr: SocketAddr, h: &HeaderMap, text: String, params: Params, f: Fmt) -> Res {
    let out = with_session(app, h, addr.ip(), move |s| s.query_read_only(&text, &params)).await?;
    Ok(table_reply(
        match f {
            Fmt::Csv => out.to_csv(),
            Fmt::Json => out.to_json(),
        },
        f,
        out.ts,
    ))
}

async fn bi_query_get(State(app): St, ConnectInfo(addr): ConnectInfo<SocketAddr>, h: HeaderMap, Query(p): Query<BiParams>) -> Res {
    let f = fmt_of(&p.format)?;
    let q = p.q.ok_or_else(|| Error::Invalid("missing q (the NQL statement)".into()))?;
    bi_run(&app, addr, &h, q, Params::new(), f).await
}

async fn bi_query_post(State(app): St, ConnectInfo(addr): ConnectInfo<SocketAddr>, h: HeaderMap, Query(p): Query<BiParams>, b: Bytes) -> Res {
    let f = fmt_of(&p.format)?;
    let r: QueryReq = body(&b)?;
    let params: Params = r.params.iter().map(|(k, v)| from_json(v).map(|v| (k.clone(), v))).collect::<Result<_, _>>()?;
    bi_run(&app, addr, &h, r.query, params, f).await
}

#[derive(serde::Deserialize)]
struct PqParams {
    base: Option<String>,
}

/// The Power Query (M) script, with this server's address filled in. No data and no secrets: no auth required.
async fn bi_powerquery(h: HeaderMap, Query(p): Query<PqParams>) -> Response {
    let ok = |s: &str| !s.is_empty() && s.len() <= 200 && s.chars().all(|c| c.is_ascii_alphanumeric() || ".-:/_[]".contains(c));
    let base = match p.base {
        Some(b) if ok(&b) => b,
        Some(_) => return reply(StatusCode::BAD_REQUEST, json!({"error": "invalid base"})),
        None => match h.get(header::HOST).and_then(|v| v.to_str().ok()).filter(|v| ok(v)) {
            Some(host) => format!("https://{host}"),
            None => "https://localhost:8443".to_string(),
        },
    };
    let mut r = (StatusCode::OK, crate::bi::power_query(&base)).into_response();
    r.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}

async fn not_found() -> Response {
    reply(StatusCode::NOT_FOUND, json!({"error": "not found"}))
}

async fn security_headers(mut r: Response) -> Response {
    let h = r.headers_mut();
    for (k, v) in [
        ("strict-transport-security", "max-age=63072000; includeSubDomains"),
        ("x-content-type-options", "nosniff"),
        ("cache-control", "no-store"),
        ("content-security-policy", "default-src 'none'"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
    ] {
        h.insert(HeaderName::from_static(k), HeaderValue::from_static(v));
    }
    r
}

pub fn router(mgr: Arc<TenantManager>, cfg: ServerConfig) -> Router {
    let limit = cfg.max_body_bytes;
    let app = Arc::new(App { sem: Semaphore::new(cfg.max_concurrent.max(1)), logins: Mutex::new(HashMap::new()), basic: Mutex::new(HashMap::new()), mgr, cfg });
    Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/login", post(login))
        .route("/v1/logout", post(logout))
        .route("/v1/me", get(me))
        .route("/v1/me/password", post(change_password))
        .route("/v1/query", post(query))
        .route("/v1/checkpoint", post(checkpoint))
        .route("/v1/audit", get(audit))
        .route("/v1/bi/tables", get(bi_catalog))
        .route("/v1/bi/tables/{name}", get(bi_table))
        .route("/v1/bi/query", get(bi_query_get).post(bi_query_post))
        .route("/v1/bi/powerquery", get(bi_powerquery))
        .route("/v1/admin/tenants", post(create_tenant))
        .route("/v1/admin/tenants/{name}", delete(purge_tenant))
        .route("/v1/admin/tenants/{name}/{action}", post(tenant_state))
        .route("/v1/admin/users", post(create_user))
        .route("/v1/admin/users/disable", post(user_disable))
        .route("/v1/admin/users/reset-password", post(reset_password))
        .fallback(not_found)
        .layer(DefaultBodyLimit::max(limit))
        .layer(middleware::map_response(security_headers))
        .with_state(app)
}

/// TLS settings for [`serve_on`]. `None` serves plain HTTP (the caller must have checked that this is acceptable).
pub async fn tls_config(cert_pem: &std::path::Path, key_pem: &std::path::Path) -> std::io::Result<axum_server::tls_rustls::RustlsConfig> {
    // A process-wide crypto provider is required by rustls; ignore "already installed".
    let _ = rustls::crypto::ring::default_provider().install_default();
    axum_server::tls_rustls::RustlsConfig::from_pem_file(cert_pem, key_pem).await
}

/// Serve on an already-bound listener until `handle` is told to shut down.
pub async fn serve_on(listener: std::net::TcpListener, router: Router, tls: Option<axum_server::tls_rustls::RustlsConfig>, handle: axum_server::Handle) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let svc = router.into_make_service_with_connect_info::<SocketAddr>();
    match tls {
        Some(cfg) => axum_server::from_tcp_rustls(listener, cfg).handle(handle).serve(svc).await,
        None => axum_server::from_tcp(listener).handle(handle).serve(svc).await,
    }
}
