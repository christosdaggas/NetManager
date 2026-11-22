"""Embed the GTK gettext catalogs in the desktop binary, without runtime files."""
import gettext
import io
import json
import subprocess
import sys
from pathlib import Path

po = Path(__file__).resolve().parents[1] / "po"
catalogs = {}
for language in (po / "LINGUAS").read_text().splitlines():
    language = language.partition("#")[0].strip()
    if not language:
        continue
    for code in language.split():
        mo = subprocess.check_output(["msgfmt", "-o", "-", str(po / (code + ".po"))])
        catalog = gettext.GNUTranslations(io.BytesIO(mo))._catalog
        catalogs[code] = {k: v for k, v in catalog.items() if isinstance(k, str) and k and v}
Path(sys.argv[1]).write_text(json.dumps(catalogs, ensure_ascii=False, separators=(",", ":")))
