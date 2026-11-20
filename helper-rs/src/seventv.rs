//! 7TV API access: emote sets, single emotes, Twitch channels and name search.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::fmt;
#[cfg(feature = "native")]
use std::time::Duration;

pub const API: &str = "https://7tv.io/v3";
pub const GQL: &str = "https://7tv.io/v3/gql";
pub const IVR: &str = "https://api.ivr.fi/v2/twitch/user?login="; // public Twitch login -> id lookup
pub const USER_AGENT: &str = concat!(
    "SevenTVEmotesHelper/",
    env!("CARGO_PKG_VERSION"),
    " (World of Warcraft addon helper)"
);

pub const ZERO_WIDTH_EMOTE_FLAG: i64 = 1 << 8; // Emote.flags
pub const ZERO_WIDTH_ACTIVE_FLAG: i64 = 1 << 0; // ActiveEmote.flags (set-level)

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// 400/404, or a lookup that found nothing. The message is shown to the player for some kinds.
    NotFound(String),
    /// Network trouble after retries ("Couldn't reach 7TV (...)"). Requests are retried later.
    Network(String),
    /// Bad input or an unexpected answer (the Python helper's ValueError).
    Invalid(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::NotFound(m) | ApiError::Network(m) | ApiError::Invalid(m) => f.write_str(m),
        }
    }
}
impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

/// The HTTP layer, replaceable in tests (the Python tests monkeypatched `seventv.http`).
pub trait Http: Send + Sync {
    fn get_json(&self, url: &str) -> ApiResult<Value>;
    fn get_bytes(&self, url: &str) -> ApiResult<Vec<u8>>;
    fn post_json(&self, url: &str, body: &Value) -> ApiResult<Value>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Emote {
    pub name: String,
    pub id: String,
    #[serde(default)]
    pub animated: bool,
    /// {"url": "//cdn.7tv.app/emote/<id>", "files": [...]}, kept as 7TV sent it.
    pub host: Value,
}

// --------------------------------------------------------------------------- HTTP

#[cfg(feature = "native")]
pub struct UreqHttp {
    agent: ureq::Agent,
    retries: u32,
    /// who's being reached, for error messages
    site: &'static str,
}

#[cfg(feature = "native")]
impl Default for UreqHttp {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "native")]
impl UreqHttp {
    pub fn new() -> Self {
        Self::with_timeout(30, "7TV")
    }

    /// `secs`: how long one request may take in all (downloads of big files need longer).
    pub fn with_timeout(secs: u64, site: &'static str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(secs)))
            .user_agent(USER_AGENT)
            .build()
            .into();
        Self {
            agent,
            retries: 3,
            site,
        }
    }

    /// GET (or POST JSON when `body` is given). NotFound on 400/404; Network after retries.
    fn request(&self, url: &str, body: Option<&Value>) -> ApiResult<Vec<u8>> {
        let mut last = String::new();
        for attempt in 0..self.retries {
            let res = match body {
                Some(b) => self
                    .agent
                    .post(url)
                    .header("Content-Type", "application/json")
                    .send(serde_json::to_vec(b).unwrap_or_default()),
                None => self.agent.get(url).call(),
            };
            match res {
                Ok(mut resp) => match resp
                    .body_mut()
                    .with_config()
                    .limit(64 * 1024 * 1024)
                    .read_to_vec()
                {
                    Ok(bytes) => return Ok(bytes),
                    Err(e) => last = e.to_string(),
                },
                Err(ureq::Error::StatusCode(code)) if code == 400 || code == 404 => {
                    return Err(ApiError::NotFound(url.to_string()));
                }
                Err(e) => last = e.to_string(),
            }
            std::thread::sleep(Duration::from_millis(1500 * (attempt as u64 + 1)));
        }
        Err(ApiError::Network(format!(
            "Couldn't reach {} ({last})",
            self.site
        )))
    }

    fn json(&self, url: &str, body: Option<&Value>) -> ApiResult<Value> {
        // A garbled answer is retried like a network error, as in the Python helper.
        let mut last = String::new();
        for _ in 0..self.retries {
            let bytes = self.request(url, body)?;
            match serde_json::from_slice(&bytes) {
                Ok(v) => return Ok(v),
                Err(e) => last = e.to_string(),
            }
        }
        Err(ApiError::Network(format!(
            "Couldn't reach {} ({last})",
            self.site
        )))
    }
}

#[cfg(feature = "native")]
impl Http for UreqHttp {
    fn get_json(&self, url: &str) -> ApiResult<Value> {
        self.json(url, None)
    }
    fn get_bytes(&self, url: &str) -> ApiResult<Vec<u8>> {
        self.request(url, None)
    }
    fn post_json(&self, url: &str, body: &Value) -> ApiResult<Value> {
        self.json(url, Some(body))
    }
}

// --------------------------------------------------------------------------- parsing user input

pub fn is_ulid(s: &str) -> bool {
    // [0-9A-HJKMNP-TV-Z]{26}, case-insensitive
    s.len() == 26
        && s.bytes().all(|c| {
            let c = c.to_ascii_uppercase();
            c.is_ascii_digit()
                || (c.is_ascii_uppercase() && !matches!(c, b'I' | b'L' | b'O' | b'U'))
        })
}

pub fn is_objid(s: &str) -> bool {
    s.len() == 24 && s.bytes().all(|c| c.is_ascii_hexdigit())
}

/// `^[A-Za-z0-9]+$`
pub fn is_id(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ref {
    Id(String),
    Set(String),
    Name(String),
}

/// Path segments of a URL (no scheme/host/query/fragment).
fn url_path_parts(url: &str) -> Vec<String> {
    let after_scheme = url.split_once("://").map(|(_, r)| r).unwrap_or(url);
    let path = match after_scheme.find('/') {
        Some(i) => &after_scheme[i..],
        None => "",
    };
    let path = path.split(['?', '#']).next().unwrap_or("");
    path.split('/')
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

/// Classify what a player typed after /7tv add.
pub fn parse_ref(reference: &str) -> ApiResult<Ref> {
    let r = reference.trim().trim_matches(|c| c == '<' || c == '>');
    if r.contains("7tv.app")
        || r.contains("7tv.io")
        || r.starts_with("http://")
        || r.starts_with("https://")
    {
        let url = if r.contains("://") {
            r.to_string()
        } else {
            format!("https://{r}")
        };
        let parts = url_path_parts(&url);
        if parts.len() > 1 {
            for i in 0..parts.len() - 1 {
                match parts[i].as_str() {
                    "emotes" | "emote" => return Ok(Ref::Id(parts[i + 1].clone())),
                    "emote-sets" | "emote-set" | "sets" => {
                        return Ok(Ref::Set(parts[i + 1].clone()));
                    }
                    _ => {}
                }
            }
        }
        if let Some(last) = parts.last()
            && is_id(last)
        {
            return Ok(Ref::Id(last.clone()));
        }
        return Err(ApiError::Invalid(
            "That link doesn't look like a 7TV emote link.".into(),
        ));
    }
    if is_ulid(r) || is_objid(r) {
        return Ok(Ref::Id(r.to_string()));
    }
    Ok(Ref::Name(r.to_string()))
}

// --------------------------------------------------------------------------- endpoints

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}
fn flags(v: &Value) -> i64 {
    v.get("flags").and_then(Value::as_i64).unwrap_or(0)
}

pub fn emotes_from_set(set_json: &Value, include_zero_width: bool) -> Vec<Emote> {
    let mut out = Vec::new();
    let Some(list) = set_json.get("emotes").and_then(Value::as_array) else {
        return out;
    };
    let empty = json!({});
    for ae in list {
        let data = ae.get("data").filter(|d| d.is_object()).unwrap_or(&empty);
        if !include_zero_width
            && (flags(ae) & ZERO_WIDTH_ACTIVE_FLAG != 0 || flags(data) & ZERO_WIDTH_EMOTE_FLAG != 0)
        {
            continue;
        }
        let host = data.get("host");
        let eid = s(ae, "id").or_else(|| s(data, "id"));
        let name = s(ae, "name").or_else(|| s(data, "name"));
        let (Some(host), Some(eid), Some(name)) = (host, eid, name) else {
            continue;
        };
        if !truthy(host.get("files")) {
            continue;
        }
        out.push(Emote {
            name: name.to_string(),
            id: eid.to_string(),
            animated: truthy(data.get("animated")),
            host: host.clone(),
        });
    }
    out
}

// URLs, shared with the web pack builder.
pub fn global_set_url() -> String {
    format!("{API}/emote-sets/global")
}
pub fn set_url(set_id: &str) -> String {
    format!("{API}/emote-sets/{set_id}")
}
pub fn emote_url(emote_id: &str) -> String {
    format!("{API}/emotes/{emote_id}")
}
pub fn twitch_login_url(login: &str) -> String {
    format!("{IVR}{}", quote(&login.to_lowercase()))
}
pub fn twitch_user_url(twitch_id: &str) -> String {
    format!("{API}/users/twitch/{twitch_id}")
}

/// The set's display name (falls back to its id).
pub fn set_name(j: &Value, set_id: &str) -> String {
    s(j, "name").unwrap_or(set_id).to_string()
}

/// A single emote from `/emotes/{id}`.
pub fn emote_from_json(j: &Value) -> ApiResult<Emote> {
    match (
        j.get("name").and_then(Value::as_str),
        j.get("id").and_then(Value::as_str),
        j.get("host"),
    ) {
        (Some(name), Some(id), Some(host)) => Ok(Emote {
            name: name.into(),
            id: id.into(),
            animated: truthy(j.get("animated")),
            host: host.clone(),
        }),
        _ => Err(ApiError::Invalid(
            "7TV sent an unexpected answer for that emote.".into(),
        )),
    }
}

/// Twitch id from the ivr.fi lookup.
pub fn twitch_id_from_json(j: &Value, login: &str) -> ApiResult<String> {
    let first = match j {
        Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    match first.get("id") {
        Some(Value::String(s)) if !s.is_empty() => Ok(s.clone()),
        Some(Value::Number(n)) => Ok(n.to_string()),
        _ => Err(ApiError::NotFound(format!(
            "Couldn't find the Twitch channel '{login}'."
        ))),
    }
}

/// The active emote set from `/users/twitch/{id}`. Returns the set and, when 7TV left out its
/// emotes, the id of the set to fetch separately.
pub fn channel_set_from_json(j: &Value) -> (Value, Option<String>) {
    let es = j
        .get("emote_set")
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or(json!({}));
    let fetch = match s(&es, "id") {
        Some(id) if es.get("emotes").is_none_or(Value::is_null) => Some(id.to_string()),
        _ => None,
    };
    (es, fetch)
}

pub fn fetch_global(http: &dyn Http) -> ApiResult<Vec<Emote>> {
    Ok(emotes_from_set(
        &http.get_json(&format!("{API}/emote-sets/global"))?,
        false,
    ))
}

pub fn fetch_set(http: &dyn Http, set_id: &str) -> ApiResult<(String, Vec<Emote>)> {
    let j = http.get_json(&set_url(set_id))?;
    Ok((set_name(&j, set_id), emotes_from_set(&j, false)))
}

pub fn fetch_emote(http: &dyn Http, emote_id: &str) -> ApiResult<Emote> {
    emote_from_json(&http.get_json(&emote_url(emote_id))?)
}

/// `urllib.parse.quote` (safe="/").
fn quote(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"_.-~/".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

pub fn twitch_login_to_id(http: &dyn Http, login: &str) -> ApiResult<String> {
    let not_found = || ApiError::NotFound(format!("Couldn't find the Twitch channel '{login}'."));
    let j = match http.get_json(&twitch_login_url(login)) {
        Err(ApiError::NotFound(_)) => return Err(not_found()),
        other => other?,
    };
    twitch_id_from_json(&j, login)
}

/// Returns (twitch_id, emotes) for a channel's active 7TV emote set.
pub fn fetch_twitch_channel(http: &dyn Http, login: &str) -> ApiResult<(String, Vec<Emote>)> {
    let twitch_id = twitch_login_to_id(http, login)?;
    let j = match http.get_json(&twitch_user_url(&twitch_id)) {
        Err(ApiError::NotFound(_)) => {
            return Err(ApiError::NotFound(format!("{login} doesn't use 7TV")));
        }
        other => other?,
    };
    let (mut es, fetch) = channel_set_from_json(&j);
    if let Some(id) = fetch {
        es = http.get_json(&set_url(&id))?;
    }
    Ok((twitch_id, emotes_from_set(&es, false)))
}

const SEARCH: &str = "query SearchEmotes($query: String!, $page: Int, $limit: Int, $filter: EmoteSearchFilter, $sort: Sort) {
  emotes(query: $query, page: $page, limit: $limit, filter: $filter, sort: $sort) {
    items { id name animated host { url files { name format width height } } }
  }
}";

/// The GraphQL request body for an exact-name search (POST to [`GQL`]).
pub fn search_body(name: &str) -> Value {
    json!({
        "query": SEARCH,
        "variables": {
            "query": name, "page": 1, "limit": 20,
            "filter": {"exact_match": true, "case_sensitive": true, "ignore_tags": true},
            "sort": {"value": "popularity", "order": "DESCENDING"},
        }
    })
}

/// Most popular public 7TV emote with exactly this name (case-sensitive), if any.
pub fn search_by_name(http: &dyn Http, name: &str) -> ApiResult<Option<Emote>> {
    Ok(search_pick(&http.post_json(GQL, &search_body(name))?, name))
}

/// The first search result with exactly this name.
pub fn search_pick(j: &Value, name: &str) -> Option<Emote> {
    let items = j
        .pointer("/data/emotes/items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for it in items {
        if it.get("name").and_then(Value::as_str) == Some(name)
            && truthy(it.get("host"))
            && let Some(id) = it.get("id").and_then(Value::as_str)
        {
            return Some(Emote {
                name: name.to_string(),
                id: id.to_string(),
                animated: truthy(it.get("animated")),
                host: it["host"].clone(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_refs() {
        assert_eq!(
            parse_ref("https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52").unwrap(),
            Ref::Id("01F6MQ33FG000FFJ97ZB8MWV52".into())
        );
        assert_eq!(
            parse_ref("7tv.app/emotes/60ae958e229664e8667aea38").unwrap(),
            Ref::Id("60ae958e229664e8667aea38".into())
        );
        assert_eq!(
            parse_ref("https://7tv.app/emote-sets/01HKQT8EWR000ESSWF3625XCS4").unwrap(),
            Ref::Set("01HKQT8EWR000ESSWF3625XCS4".into())
        );
        assert_eq!(
            parse_ref("01F6MQ33FG000FFJ97ZB8MWV52").unwrap(),
            Ref::Id("01F6MQ33FG000FFJ97ZB8MWV52".into())
        );
        assert_eq!(parse_ref("catJAM").unwrap(), Ref::Name("catJAM".into()));
        assert_eq!(
            parse_ref("<https://7tv.app/emotes/abc?x=1>").unwrap(),
            Ref::Id("abc".into())
        );
        assert!(matches!(
            parse_ref("https://7tv.app/"),
            Err(ApiError::Invalid(_))
        ));
        assert_eq!(
            parse_ref("https://7tv.app/emotes/").unwrap(),
            Ref::Id("emotes".into())
        ); // same as Python
    }

    #[test]
    fn zero_width_skipped() {
        let set = json!({"emotes": [
            {"id": "a", "name": "A", "flags": 0, "data": {"flags": 256, "host": {"url": "//x", "files": [1]}}},
            {"id": "b", "name": "B", "flags": 1, "data": {"flags": 0, "host": {"url": "//x", "files": [1]}}},
            {"id": "c", "name": "C", "flags": 0, "data": {"flags": 0, "animated": true, "host": {"url": "//x", "files": [1]}}},
            {"id": "d", "name": "D", "flags": 0, "data": {"flags": 0, "host": {"url": "//x", "files": []}}},
        ]});
        let e = emotes_from_set(&set, false);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].name, "C");
        assert!(e[0].animated);
    }
}
