"""Generate AramBuff.json + aram-manifest.json for Seraphine.

Primary source: League of Legends Wiki Module:ChampionData/data (CC BY-SA 3.0).
The wiki is protected by a browser challenge for non-browser clients on some
networks; run this script from GitHub Actions (see update_aram_data.yaml).

Bootstrap source: a balance.json produced by the MIT-licensed project
nomi-san/balance-buff-viewer (same wiki data, pre-parsed).

Output contract (consumed by app/lol/aram.py and rust sidecar):
{
  "schema": 1,
  "version": "16.23.1",          # ddragon version
  "gameVersion": "16.23",        # comparable with getLolClientVersion()
  "generatedAt": "2026-11-26T...Z",
  "source": "...",
  "champions": {
    "75": {"catname": "...", "zcsh": "95", "sdsh": "105", "zlxl": "100",
           "hdxn": "100", "jnjs": "0", "renxing": "0", "description": "..."}
  }
}
"""

import argparse
import hashlib
import json
import os
import re
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

WIKI_DATA_URL = "https://wiki.leagueoflegends.com/en-us/Module:ChampionData/data"
WIKI_PRE_START = "-- &lt;pre>"
WIKI_PRE_END = "-- &lt;/pre>"
DDRAGON_VERSIONS_URL = "https://ddragon.leagueoflegends.com/api/versions.json"
BOOTSTRAP_URL = ("https://cdn.jsdelivr.net/npm/"
                 "balance-buff-viewer-plugin@latest/dist/balance.json")
HERO_LIST_URL = "https://game.gtimg.cn/images/lol/act/img/js/heroList/hero_list.js"
FETCH_UA = "curl/8.13.0"
MIN_CHAMPIONS = 120
VERSION_PATTERN = re.compile(r"^\d+\.\d+(\.\d+)?$")

RATIO_FIELDS = ("dmg_dealt", "dmg_taken", "healing", "shielding",
                "mana_regen", "energy_regen", "attack_speed", "movement_speed")


class LuaError(ValueError):
    pass


class Lexer:
    def __init__(self, text):
        self.text = text
        self.index = 0
        self.length = len(text)

    def _skip(self):
        while self.index < self.length:
            char = self.text[self.index]
            if char in " \t\r\n":
                self.index += 1
            elif self.text.startswith("--[[", self.index):
                end = self.text.find("]]", self.index + 4)
                self.index = self.length if end < 0 else end + 2
            elif self.text.startswith("--", self.index):
                end = self.text.find("\n", self.index)
                self.index = self.length if end < 0 else end + 1
            else:
                break

    def next(self):
        self._skip()
        if self.index >= self.length:
            return ("eof", None)
        char = self.text[self.index]
        if char in "{}[]=,;" or char in "+-*/":
            self.index += 1
            return (char, char)
        if char in "\"'":
            return ("string", self._string(char))
        if char.isdigit():
            return ("number", self._number())
        if char.isalpha() or char == "_":
            start = self.index
            self.index += 1
            while self.index < self.length and (
                self.text[self.index].isalnum() or self.text[self.index] == "_"
            ):
                self.index += 1
            return ("ident", self.text[start:self.index])
        raise LuaError("unexpected character %r" % char)

    def _string(self, quote):
        self.index += 1
        buffer = []
        escapes = {"n": "\n", "t": "\t", "r": "\r", "\\": "\\", "\"": "\"", "'": "'"}
        while self.index < self.length:
            char = self.text[self.index]
            if char == "\\" and self.index + 1 < self.length:
                nxt = self.text[self.index + 1]
                buffer.append(escapes.get(nxt, nxt))
                self.index += 2
                continue
            if char == quote:
                self.index += 1
                return "".join(buffer)
            buffer.append(char)
            self.index += 1
        raise LuaError("unterminated string")

    def _number(self):
        start = self.index
        while self.index < self.length and self.text[self.index].isdigit():
            self.index += 1
        if self.index < self.length and self.text[self.index] == ".":
            self.index += 1
            while self.index < self.length and self.text[self.index].isdigit():
                self.index += 1
        if self.index < self.length and self.text[self.index] in "eE":
            self.index += 1
            if self.index < self.length and self.text[self.index] in "+-":
                self.index += 1
            while self.index < self.length and self.text[self.index].isdigit():
                self.index += 1
        token = self.text[start:self.index]
        try:
            return float(token)
        except ValueError as error:
            raise LuaError("invalid number %r" % token) from error


class Parser:
    def __init__(self, lexer):
        self.lexer = lexer
        self.token = self.lexer.next()

    def _advance(self):
        self.token = self.lexer.next()

    def _expect(self, kind):
        if self.token[0] != kind:
            raise LuaError("expected %r, got %r" % (kind, self.token))
        value = self.token[1]
        self._advance()
        return value

    def parse_module(self):
        while self.token[0] != "eof":
            if self.token[0] == "ident" and self.token[1] == "return":
                self._advance()
                return self._value()
            self._advance()
        raise LuaError("module has no return statement")

    def _value(self):
        kind, value = self.token
        if kind == "{":
            return self._table()
        if kind == "string":
            self._advance()
            return value
        if kind in ("number", "+", "-"):
            return self._expression()
        if kind == "ident":
            self._advance()
            if value == "true":
                return True
            if value == "false":
                return False
            if value == "nil":
                return None
            raise LuaError("unexpected identifier %r" % value)
        raise LuaError("unexpected token %r" % (self.token,))

    def _expression(self):
        value = self._term()
        while self.token[0] in ("+", "-"):
            operator = self.token[0]
            self._advance()
            operand = self._term()
            value = value + operand if operator == "+" else value - operand
        return value

    def _term(self):
        value = self._factor()
        while self.token[0] in ("*", "/"):
            operator = self.token[0]
            self._advance()
            operand = self._factor()
            value = value * operand if operator == "*" else value / operand
        return value

    def _factor(self):
        if self.token[0] in ("+", "-"):
            operator = self.token[0]
            self._advance()
            value = self._factor()
            return -value if operator == "-" else value
        if self.token[0] == "number":
            value = self.token[1]
            self._advance()
            return value
        raise LuaError("expected number, got %r" % (self.token,))

    def _table(self):
        self._expect("{")
        keyed = {}
        array = []
        while self.token[0] != "}":
            if self.token[0] == "eof":
                raise LuaError("unterminated table")
            if self.token[0] == "[":
                self._advance()
                if self.token[0] == "string":
                    key = self.token[1]
                    self._advance()
                elif self.token[0] in ("number", "+", "-"):
                    value = self._expression()
                    key = (str(int(value)) if float(value).is_integer()
                           else str(value))
                else:
                    raise LuaError("expected table key, got %r" % (self.token,))
                self._expect("]")
                self._expect("=")
                keyed[key] = self._value()
            elif self.token[0] == "ident":
                key = self.token[1]
                self._advance()
                self._expect("=")
                keyed[key] = self._value()
            else:
                array.append(self._value())
            if self.token[0] in (",", ";"):
                self._advance()
            elif self.token[0] != "}":
                raise LuaError("expected ',' or '}', got %r" % (self.token,))
        self._advance()
        if keyed:
            for index, item in enumerate(array):
                keyed[str(index + 1)] = item
            return keyed
        return array


def parse_lua_module(script):
    return Parser(Lexer(script)).parse_module()


def fetch_text(url, timeout=30):
    request = urllib.request.Request(url, headers={
        "User-Agent": FETCH_UA,
        "Accept": "*/*",
    })
    with urllib.request.urlopen(request, timeout=timeout) as response:
        charset = response.headers.get_content_charset() or "utf-8"
        return response.read(), charset


def fetch_wiki_script():
    body, charset = fetch_text(WIKI_DATA_URL)
    html = body.decode(charset, "replace")
    start = html.find(WIKI_PRE_START)
    if start < 0:
        raise RuntimeError(
            "wiki page does not contain the Lua data block "
            "(browser challenge or page layout changed)")
    end = html.find(WIKI_PRE_END, start + len(WIKI_PRE_START))
    if end < 0:
        raise RuntimeError("wiki Lua data block is not terminated")
    import html as html_module
    script = html_module.unescape(html[start + len(WIKI_PRE_START):end])
    return script


def fetch_bootstrap():
    body, charset = fetch_text(BOOTSTRAP_URL)
    balance = json.loads(body.decode(charset, "replace"))
    if not isinstance(balance, dict):
        raise RuntimeError("bootstrap source did not return an object")
    return balance


def fetch_latest_version():
    body, charset = fetch_text(DDRAGON_VERSIONS_URL)
    versions = json.loads(body.decode(charset, "replace"))
    if not versions or not isinstance(versions[0], str):
        raise RuntimeError("ddragon returned no versions")
    return versions[0]


def fetch_champion_names(url=HERO_LIST_URL):
    try:
        body, charset = fetch_text(url)
        root = json.loads(body.decode(charset, "replace"))
        names = {}
        for hero in root.get("hero", []):
            hero_id = str(hero.get("heroId", ""))
            name = (hero.get("name") or "").strip()
            if hero_id and name:
                names[hero_id] = name
        return names
    except Exception as error:
        print("warning: champion names unavailable: %s" % error, file=sys.stderr)
        return {}


def _number(value, default):
    return float(value) if isinstance(value, (int, float)) else default


def _percent(ratio):
    return int(round(ratio * 100))


def _bonus(ratio):
    return int(round((ratio - 1.0) * 100))


def build_champion_entry(stats_aram, fallback_title):
    dmg_dealt = _number(stats_aram.get("dmg_dealt"), 1.0)
    dmg_taken = _number(stats_aram.get("dmg_taken"), 1.0)
    healing = _number(stats_aram.get("healing"), 1.0)
    shielding = _number(stats_aram.get("shielding"), 1.0)
    ability_haste = int(round(_number(stats_aram.get("ability_haste"), 0.0)))
    tenacity = _number(stats_aram.get("tenacity"), 1.0)
    entry = {
        "catname": fallback_title,
        "zcsh": str(_percent(dmg_dealt)),
        "sdsh": str(_percent(dmg_taken)),
        "zlxl": str(_percent(healing)),
        "hdxn": str(_percent(shielding)),
        "jnjs": str(ability_haste),
        "renxing": str(_bonus(tenacity)),
    }
    details = []
    if round(dmg_dealt, 4) != 1.0:
        details.append("造成伤害 %d%%" % _percent(dmg_dealt))
    if round(dmg_taken, 4) != 1.0:
        details.append("承受伤害 %d%%" % _percent(dmg_taken))
    if round(healing, 4) != 1.0:
        details.append("治疗效果 %d%%" % _percent(healing))
    if round(shielding, 4) != 1.0:
        details.append("护盾效果 %d%%" % _percent(shielding))
    if ability_haste:
        details.append("技能急速 %+d" % ability_haste)
    if round(tenacity, 4) != 1.0:
        details.append("韧性 %+d%%" % _bonus(tenacity))
    extras = (("mana_regen", "法力回复"), ("energy_regen", "能量回复"),
              ("attack_speed", "攻击速度"), ("movement_speed", "移动速度"))
    for key, label in extras:
        ratio = _number(stats_aram.get(key), 1.0)
        if round(ratio, 4) != 1.0:
            details.append("%s %+d%%" % (label, _bonus(ratio)))
    entry["description"] = "，".join(details)
    return entry


def _is_neutral(entry):
    return (entry["zcsh"] == "100" and entry["sdsh"] == "100"
            and entry["zlxl"] == "100" and entry["hdxn"] == "100"
            and entry["jnjs"] == "0" and entry["renxing"] == "0")


def build_from_module(module, names):
    root = module if isinstance(module, dict) else {}
    champions = {}
    for value in root.values():
        if not isinstance(value, dict):
            continue
        if "id" not in value or "stats" not in value:
            continue
        champion_id = int(float(value.get("id") or 0))
        if champion_id <= 0:
            continue
        stats = value.get("stats") or {}
        aram = stats.get("aram") if isinstance(stats, dict) else None
        if not isinstance(aram, dict):
            continue
        fallback = (names.get(str(champion_id))
                    or str(value.get("title") or "").strip()
                    or str(value.get("apiname") or champion_id))
        entry = build_champion_entry(aram, fallback)
        if _is_neutral(entry):
            continue
        champions[str(champion_id)] = entry
    return champions


def build_from_balance_json(balance, names):
    champions = {}
    for key, value in balance.items():
        if not str(key).isdigit() or not isinstance(value, dict):
            continue
        stats = (value.get("stats") or {}).get("aram")
        if not isinstance(stats, dict):
            continue
        fallback = (names.get(str(key))
                    or str(value.get("name") or "").strip()
                    or str(key))
        entry = build_champion_entry(stats, fallback)
        if _is_neutral(entry):
            continue
        champions[str(key)] = entry
    return champions


def validate_data(data):
    if not isinstance(data, dict):
        raise ValueError("data is not an object")
    version = data.get("version")
    if not isinstance(version, str) or not VERSION_PATTERN.match(version):
        raise ValueError("invalid data version: %r" % (version,))
    champions = data.get("champions")
    if not isinstance(champions, dict) or len(champions) < MIN_CHAMPIONS:
        raise ValueError("too few champions: %s"
                         % (len(champions) if isinstance(champions, dict) else "n/a"))
    interesting = 0
    for champion_id, entry in champions.items():
        if not champion_id.isdigit() or not isinstance(entry, dict):
            raise ValueError("invalid champion entry: %r" % (champion_id,))
        if not isinstance(entry.get("catname"), str) or not entry["catname"]:
            raise ValueError("champion %s has no catname" % champion_id)
        for key in ("zcsh", "sdsh", "zlxl", "hdxn", "jnjs", "renxing"):
            value = entry.get(key)
            if not isinstance(value, str):
                raise ValueError("champion %s field %s is not a string"
                                 % (champion_id, key))
            int(value)
        if not isinstance(entry.get("description"), str):
            raise ValueError("champion %s has no description" % champion_id)
        if (entry["zcsh"] != "100" or entry["sdsh"] != "100"
                or entry["zlxl"] != "100" or entry["hdxn"] != "100"
                or entry["jnjs"] != "0" or entry["renxing"] != "0"):
            interesting += 1
    if interesting == 0:
        raise ValueError("all champion modifiers are neutral")


def atomic_write(path, payload):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    handle, temporary = tempfile.mkstemp(prefix=".tmp-aram-", dir=str(path.parent))
    try:
        with os.fdopen(handle, "wb") as stream:
            stream.write(payload)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


def generate(args):
    names = {} if args.no_names else fetch_champion_names(args.names_url)
    if args.source == "balance-json":
        balance = json.loads(Path(args.balance_json).read_text(encoding="utf-8"))
        champions = build_from_balance_json(balance, names)
        version = args.version or str(balance.get("_patch") or "")
        source = "wiki.leagueoflegends.com Module:ChampionData/data (bootstrap)"
    else:
        try:
            script = fetch_wiki_script()
        except Exception as error:
            if args.source != "auto":
                raise
            print("warning: wiki source unavailable (%s); "
                  "falling back to the bootstrap snapshot" % error, file=sys.stderr)
            balance = fetch_bootstrap()
            champions = build_from_balance_json(balance, names)
            version = (args.version or str(balance.get("_patch") or "")
                       or fetch_latest_version())
            source = ("wiki.leagueoflegends.com Module:ChampionData/data "
                      "(bootstrap via jsdelivr)")
        else:
            module = parse_lua_module(script)
            champions = build_from_module(module, names)
            version = args.version or fetch_latest_version()
            source = "wiki.leagueoflegends.com Module:ChampionData/data"

    if not VERSION_PATTERN.match(version):
        raise RuntimeError("no usable version (pass --version): %r" % version)

    ordered = {key: champions[key]
               for key in sorted(champions, key=lambda item: int(item))}
    generated_at = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())
    data = {
        "schema": 1,
        "version": version,
        "gameVersion": ".".join(version.split(".")[:2]),
        "generatedAt": generated_at,
        "source": source,
        "champions": ordered,
    }
    validate_data(data)

    data_bytes = json.dumps(data, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    manifest = {
        "schema": 1,
        "version": version,
        "gameVersion": data["gameVersion"],
        "generatedAt": generated_at,
        "file": "AramBuff.json",
        "size": len(data_bytes),
        "sha256": hashlib.sha256(data_bytes).hexdigest(),
        "champions": len(ordered),
        "source": source,
    }

    output_dir = Path(args.output_dir)
    if args.dry_run:
        print(json.dumps({"manifest": manifest, "dryRun": True}, indent=2))
        return manifest
    atomic_write(output_dir / "AramBuff.json", data_bytes)
    atomic_write(output_dir / "aram-manifest.json",
                 (json.dumps(manifest, indent=2) + "\n").encode("utf-8"))
    print(json.dumps(manifest, indent=2))
    return manifest


def main(argv=None):
    parser = argparse.ArgumentParser(description="Generate ARAM balance data snapshot")
    parser.add_argument("--source", choices=("auto", "wiki", "balance-json"),
                        default="auto",
                        help="auto tries the wiki and falls back to the bootstrap snapshot")
    parser.add_argument("--balance-json", help="path to a balance-buff-viewer balance.json")
    parser.add_argument("--version", help="override data version (e.g. 16.23.1)")
    parser.add_argument("--output-dir", default="app/resource/data")
    parser.add_argument("--names-url", default=HERO_LIST_URL,
                        help="champion name source (Tencent hero list)")
    parser.add_argument("--no-names", action="store_true",
                        help="do not fetch Chinese champion names")
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    if args.source == "balance-json" and not args.balance_json:
        parser.error("--balance-json is required for --source balance-json")
    try:
        generate(args)
    except Exception as error:
        print("error: %s" % error, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())