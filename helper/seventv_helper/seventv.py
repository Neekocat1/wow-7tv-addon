"""7TV API access: emote sets, single emotes, Twitch channels and name search."""
from __future__ import annotations

import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import asdict, dataclass

API = "https://7tv.io/v3"
GQL = "https://7tv.io/v3/gql"
IVR = "https://api.ivr.fi/v2/twitch/user?login="  # public Twitch login -> id lookup
USER_AGENT = "SevenTVEmotesHelper/0.2 (World of Warcraft addon helper)"

ZERO_WIDTH_EMOTE_FLAG = 1 << 8   # Emote.flags
ZERO_WIDTH_ACTIVE_FLAG = 1 << 0  # ActiveEmote.flags (set-level)

ULID_RE = re.compile(r"^[0-9A-HJKMNP-TV-Z]{26}$", re.I)
OBJID_RE = re.compile(r"^[0-9a-f]{24}$", re.I)
ID_RE = re.compile(r"^[A-Za-z0-9]+$")


class NotFound(Exception):
    pass


@dataclass
class Emote:
    name: str
    id: str
    animated: bool
    host: dict  # {"url": "//cdn.7tv.app/emote/<id>", "files": [...]}

    def to_json(self) -> dict:
        return asdict(self)

    @staticmethod
    def from_json(d: dict) -> "Emote":
        return Emote(d["name"], d["id"], bool(d.get("animated")), d["host"])


# --------------------------------------------------------------------------- HTTP

def http(url: str, *, binary: bool = False, data: dict | None = None, retries: int = 3):
    """GET (or POST JSON when `data` is given). Raises NotFound on 404."""
    last: Exception | None = None
    body = json.dumps(data).encode() if data is not None else None
    for attempt in range(retries):
        try:
            headers = {"User-Agent": USER_AGENT}
            if body is not None:
                headers["Content-Type"] = "application/json"
            req = urllib.request.Request(url, data=body, headers=headers)
            with urllib.request.urlopen(req, timeout=30) as resp:
                raw = resp.read()
            return raw if binary else json.loads(raw.decode("utf-8"))
        except urllib.error.HTTPError as e:
            if e.code in (400, 404):
                raise NotFound(url) from e
            last = e
        except (urllib.error.URLError, TimeoutError, json.JSONDecodeError, ConnectionError) as e:
            last = e
        time.sleep(1.5 * (attempt + 1))
    raise ConnectionError(f"Couldn't reach 7TV ({last})")


# --------------------------------------------------------------------------- parsing user input

def parse_ref(ref: str) -> tuple[str, str]:
    """Classify what a player typed after /7tv add.

    Returns ("id", emote_id), ("set", set_id) or ("name", name)."""
    ref = ref.strip().strip("<>")
    if "7tv.app" in ref or "7tv.io" in ref or ref.startswith(("http://", "https://")):
        url = ref if "://" in ref else "https://" + ref
        parts = [p for p in urllib.parse.urlparse(url).path.split("/") if p]
        for i, p in enumerate(parts[:-1]):
            if p in ("emotes", "emote"):
                return "id", parts[i + 1]
            if p in ("emote-sets", "emote-set", "sets"):
                return "set", parts[i + 1]
        if parts and ID_RE.match(parts[-1]):
            return "id", parts[-1]
        raise ValueError("That link doesn't look like a 7TV emote link.")
    if ULID_RE.match(ref) or OBJID_RE.match(ref):
        return "id", ref
    return "name", ref


# --------------------------------------------------------------------------- endpoints

def emotes_from_set(set_json: dict, include_zero_width: bool = False) -> list[Emote]:
    out = []
    for ae in set_json.get("emotes") or []:
        data = ae.get("data") or {}
        if not include_zero_width and ((ae.get("flags") or 0) & ZERO_WIDTH_ACTIVE_FLAG
                                       or (data.get("flags") or 0) & ZERO_WIDTH_EMOTE_FLAG):
            continue
        host = data.get("host")
        eid = ae.get("id") or data.get("id")
        name = ae.get("name") or data.get("name")
        if not host or not host.get("files") or not eid or not name:
            continue
        out.append(Emote(name, eid, bool(data.get("animated")), host))
    return out


def fetch_global() -> list[Emote]:
    return emotes_from_set(http(f"{API}/emote-sets/global"))


def fetch_set(set_id: str) -> tuple[str, list[Emote]]:
    j = http(f"{API}/emote-sets/{set_id}")
    return j.get("name") or set_id, emotes_from_set(j)


def fetch_emote(emote_id: str) -> Emote:
    j = http(f"{API}/emotes/{emote_id}")
    return Emote(j["name"], j["id"], bool(j.get("animated")), j["host"])


def twitch_login_to_id(login: str) -> str:
    try:
        j = http(IVR + urllib.parse.quote(login.lower()))
    except NotFound:
        raise NotFound(f"Couldn't find the Twitch channel '{login}'.")
    users = j if isinstance(j, list) else [j]
    if not users or not users[0] or not users[0].get("id"):
        raise NotFound(f"Couldn't find the Twitch channel '{login}'.")
    return str(users[0]["id"])


def fetch_twitch_channel(login: str) -> tuple[str, list[Emote]]:
    """Returns (twitch_id, emotes) for a channel's active 7TV emote set."""
    twitch_id = twitch_login_to_id(login)
    try:
        j = http(f"{API}/users/twitch/{twitch_id}")
    except NotFound:
        raise NotFound(f"{login} doesn't use 7TV")
    es = j.get("emote_set") or {}
    if es.get("id") and es.get("emotes") is None:
        es = http(f"{API}/emote-sets/{es['id']}")
    return twitch_id, emotes_from_set(es)


_SEARCH = """query SearchEmotes($query: String!, $page: Int, $limit: Int, $filter: EmoteSearchFilter, $sort: Sort) {
  emotes(query: $query, page: $page, limit: $limit, filter: $filter, sort: $sort) {
    items { id name animated host { url files { name format width height } } }
  }
}"""


def search_by_name(name: str) -> Emote | None:
    """Most popular public 7TV emote with exactly this name (case-sensitive), if any."""
    variables = {
        "query": name, "page": 1, "limit": 20,
        "filter": {"exact_match": True, "case_sensitive": True, "ignore_tags": True},
        "sort": {"value": "popularity", "order": "DESCENDING"},
    }
    j = http(GQL, data={"query": _SEARCH, "variables": variables})
    items = (((j or {}).get("data") or {}).get("emotes") or {}).get("items") or []
    for it in items:
        if it.get("name") == name and it.get("host"):
            return Emote(it["name"], it["id"], bool(it.get("animated")), it["host"])
    return None
