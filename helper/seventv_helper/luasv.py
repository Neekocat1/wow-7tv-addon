"""Read WoW SavedVariables files and write Lua data files.

SavedVariables are plain Lua assignments that WoW writes on /reload and logout:

    SevenTVEmotesDB = {
        ["queue"] = {
            {
                ["kind"] = "add",
                ["ref"] = "https://7tv.app/emotes/01F6MQ33FG000FFJ97ZB8MWV52",
            }, -- [1]
        },
    }

Only literal data appears there, so a small parser is enough.
"""
from __future__ import annotations

import re

_NUM = re.compile(r"-?(?:0[xX][0-9a-fA-F]+|(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?|inf|nan)")
_IDENT = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
_ESC = {"n": "\n", "t": "\t", "r": "\r", "a": "\a", "b": "\b", "f": "\f", "v": "\v",
        "\\": "\\", '"': '"', "'": "'", "\n": "\n"}


class LuaParseError(ValueError):
    pass


class _Parser:
    def __init__(self, text: str):
        self.s = text
        self.i = 0

    def error(self, msg: str):
        line = self.s.count("\n", 0, self.i) + 1
        raise LuaParseError(f"{msg} at line {line}")

    def skip(self):
        s, n = self.s, len(self.s)
        while self.i < n:
            c = s[self.i]
            if c in " \t\r\n,;":  # separators are harmless at statement level too
                self.i += 1
            elif s.startswith("--", self.i):
                m = re.match(r"--\[(=*)\[", s[self.i:])
                if m:
                    end = s.find("]" + m.group(1) + "]", self.i)
                    self.i = n if end < 0 else end + len(m.group(1)) + 2
                else:
                    nl = s.find("\n", self.i)
                    self.i = n if nl < 0 else nl + 1
            else:
                break

    def parse_file(self) -> dict:
        out = {}
        while True:
            self.skip()
            if self.i >= len(self.s):
                return out
            m = _IDENT.match(self.s, self.i)
            if not m:
                self.error("expected a variable name")
            self.i = m.end()
            self.skip()
            if self.s[self.i:self.i + 1] != "=":
                self.error("expected '='")
            self.i += 1
            out[m.group(0)] = self.value()

    def value(self):
        self.skip()
        s, i = self.s, self.i
        if i >= len(s):
            self.error("unexpected end of file")
        c = s[i]
        if c == "{":
            return self.table()
        if c in "\"'":
            return self.string()
        if c == "[" and re.match(r"\[=*\[", s[i:]):
            return self.long_string()
        for word, val in (("true", True), ("false", False), ("nil", None)):
            if s.startswith(word, i) and not (i + len(word) < len(s) and (s[i + len(word)].isalnum() or s[i + len(word)] == "_")):
                self.i += len(word)
                return val
        m = _NUM.match(s, i)
        if m:
            self.i = m.end()
            t = m.group(0)
            if "x" in t.lower():
                return int(t, 16)
            if t.lstrip("-") in ("inf", "nan"):
                return float(t)
            return float(t) if any(ch in t for ch in ".eE") else int(t)
        self.error(f"unexpected character {c!r}")

    def string(self) -> str:
        s, q = self.s, self.s[self.i]
        self.i += 1
        out = []
        while True:
            if self.i >= len(s):
                self.error("unterminated string")
            c = s[self.i]
            if c == q:
                self.i += 1
                break
            if c == "\\":
                self.i += 1
                e = s[self.i]
                if e.isdigit():
                    m = re.match(r"\d{1,3}", s[self.i:])
                    out.append(chr(int(m.group(0))))
                    self.i += len(m.group(0))
                    continue
                out.append(_ESC.get(e, e))
                self.i += 1
                continue
            out.append(c)
            self.i += 1
        # Lua strings are bytes; WoW writes UTF-8. Decimal escapes give us raw bytes as chars.
        raw = "".join(out)
        try:
            return raw.encode("latin-1").decode("utf-8")
        except (UnicodeEncodeError, UnicodeDecodeError):
            return raw

    def long_string(self) -> str:
        m = re.match(r"\[(=*)\[", self.s[self.i:])
        close = "]" + m.group(1) + "]"
        start = self.i + m.end()
        end = self.s.find(close, start)
        if end < 0:
            self.error("unterminated long string")
        self.i = end + len(close)
        text = self.s[start:end]
        return text[1:] if text.startswith("\n") else text

    def table(self):
        self.i += 1  # {
        arr, hsh = [], {}
        while True:
            self.skip()
            if self.i >= len(self.s):
                self.error("unterminated table")
            c = self.s[self.i]
            if c == "}":
                self.i += 1
                break
            if c == "[" and not re.match(r"\[=*\[", self.s[self.i:]):
                self.i += 1
                key = self.value()
                self.skip()
                if self.s[self.i:self.i + 1] != "]":
                    self.error("expected ']'")
                self.i += 1
                self.skip()
                if self.s[self.i:self.i + 1] != "=":
                    self.error("expected '='")
                self.i += 1
                hsh[key] = self.value()
                continue
            m = _IDENT.match(self.s, self.i)
            if m and m.group(0) not in ("true", "false", "nil"):
                j = m.end()
                while j < len(self.s) and self.s[j] in " \t":
                    j += 1
                if self.s[j:j + 1] == "=" and self.s[j:j + 2] != "==":
                    self.i = j + 1
                    hsh[m.group(0)] = self.value()
                    continue
            arr.append(self.value())
        if not hsh:
            return arr
        for n, v in enumerate(arr, 1):
            hsh[n] = v
        keys = list(hsh)
        if all(isinstance(k, int) for k in keys) and sorted(keys) == list(range(1, len(keys) + 1)):
            return [hsh[k] for k in range(1, len(keys) + 1)]
        return hsh


def parse(text: str) -> dict:
    """Parse a SavedVariables file into {global_name: value}."""
    return _Parser(text).parse_file()


# --------------------------------------------------------------------------- writing

def lua_str(s: str) -> str:
    out = ['"']
    for b in str(s).encode("utf-8"):
        if b in (0x22, 0x5C):
            out.append("\\" + chr(b))
        elif 32 <= b < 127:
            out.append(chr(b))
        else:
            out.append("\\%03d" % b)
    out.append('"')
    return "".join(out)


def lua_value(v, indent: str = "") -> str:
    if v is None:
        return "nil"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, int):
        return str(v)
    if isinstance(v, float):
        return repr(v) if v == v and v not in (float("inf"), float("-inf")) else "0"
    if isinstance(v, str):
        return lua_str(v)
    inner = indent + "  "
    if isinstance(v, (list, tuple)):
        if not v:
            return "{}"
        return "{ " + ", ".join(lua_value(x, inner) for x in v) + " }"
    if isinstance(v, dict):
        if not v:
            return "{}"
        parts = []
        for k, x in v.items():
            key = f"[{k}]" if isinstance(k, int) else (k if _IDENT.fullmatch(str(k)) and k not in _KEYWORDS else f"[{lua_str(k)}]")
            parts.append(f"{inner}{key} = {lua_value(x, inner)},")
        return "{\n" + "\n".join(parts) + "\n" + indent + "}"
    raise TypeError(f"can't write {type(v).__name__} to Lua")


_KEYWORDS = {"and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if",
             "in", "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while"}
