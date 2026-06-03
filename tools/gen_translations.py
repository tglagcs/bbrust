#!/usr/bin/env python3
"""Generate src/translations_ru.rs from BleachBit's po/ru.po.

Collects the English label/description/warning strings used by the bundled
cleaners, matches them against the Russian .po catalog, and emits a compact
Rust table. Re-run after changing the cleaner set.
"""
import glob
import os
import re
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PO = os.path.join(ROOT, "..", "BleachBit-SourceCode", "po", "ru.po")
OUT = os.path.join(ROOT, "src", "translations_ru.rs")


def collect_strings():
    strings = set()

    def add(s):
        if s and s.strip():
            strings.add(s.strip())

    for path in glob.glob(os.path.join(ROOT, "cleaners", "*.xml")):
        try:
            root = ET.parse(path).getroot()
        except Exception as e:
            print("skip", path, e)
            continue
        for tag in ("label", "description"):
            el = root.find(tag)
            if el is not None and el.text:
                add(el.text)
        for opt in root.findall("option"):
            for tag in ("label", "description", "warning"):
                el = opt.find(tag)
                if el is not None and el.text:
                    add(el.text)
    return strings


def unescape(s):
    return s.replace('\\"', '"').replace("\\n", "\n").replace("\\\\", "\\")


def parse_po():
    trans = {}
    cur_id = None
    cur_str = None
    mode = None

    def flush():
        nonlocal cur_id, cur_str
        if cur_id is not None and cur_str:
            trans[cur_id] = cur_str
        cur_id = None
        cur_str = None

    with open(PO, encoding="utf-8") as f:
        for raw in f:
            line = raw.rstrip("\n")
            m = re.match(r'msgid "(.*)"$', line)
            if m:
                flush()
                cur_id = unescape(m.group(1))
                mode = "id"
                continue
            m = re.match(r'msgstr "(.*)"$', line)
            if m:
                cur_str = unescape(m.group(1))
                mode = "str"
                continue
            m = re.match(r'"(.*)"$', line)
            if m:
                chunk = unescape(m.group(1))
                if mode == "id":
                    cur_id = (cur_id or "") + chunk
                elif mode == "str":
                    cur_str = (cur_str or "") + chunk
                continue
    flush()
    return trans


# Hand-written translations for strings that BleachBit's po/ru.po lacks
# (e.g. labels introduced by this fork's own cleaners).
MANUAL = {
    "Media cache": "Медиа-кэш",
    "Instant messenger": "Мессенджер",
    "Event logs": "Журналы событий",
    "MUICache": "MUICache",
    "ONLYOFFICE": "ONLYOFFICE",
}


def rs_escape(s):
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def main():
    strings = collect_strings()
    trans = parse_po()
    pairs = []
    missing = 0
    for en in sorted(strings):
        ru = trans.get(en) or MANUAL.get(en)
        if ru and ru.strip() and ru != en:
            pairs.append((en, ru))
        else:
            missing += 1

    with open(OUT, "w", encoding="utf-8", newline="\n") as out:
        out.write("//! Russian translations for cleaner and option names.\n//!\n")
        out.write("//! Auto-generated from BleachBit's `po/ru.po` by tools/gen_translations.py.\n")
        out.write("//! Do not edit by hand; regenerate if the cleaner set changes.\n\n")
        out.write("/// English -> Russian for labels/warnings used by the bundled cleaners.\n")
        out.write("pub static RU_PAIRS: &[(&str, &str)] = &[\n")
        for en, ru in pairs:
            out.write(f'    ("{rs_escape(en)}", "{rs_escape(ru)}"),\n')
        out.write("];\n")

    print(f"collected {len(strings)} strings; translated {len(pairs)}; missing {missing}")


if __name__ == "__main__":
    main()
