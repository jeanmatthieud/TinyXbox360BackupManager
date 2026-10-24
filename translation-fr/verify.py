"""Verify French placeholders and preserve the application's protocol values."""
from collections import Counter
import json
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
BASE = "3059f881f9a2bf7a54808cd0478cea4000ea3e8f"
count = 0
for name in ("ui.json", "rust.json", "core.json", "extra.json"):
    catalog = Path(__file__).parent / name
    translations = json.loads(catalog.read_text(encoding="utf-8"))
    pattern = r"\\\{[^}]+\}" if catalog.name == "ui.json" else r"\{[^{}]*\}"
    for source, translated in translations.items():
        assert Counter(re.findall(pattern, source)) == Counter(re.findall(pattern, translated)), source
        count += 1

baseline = json.loads((Path(__file__).parent / "upstream-ui-keys.json").read_text(encoding="utf-8"))
for path in (ROOT / "ui").rglob("*.slint"):
    relative = path.relative_to(ROOT).as_posix()
    current = path.read_text(encoding="utf-8")
    # Serialized settings and sort prefixes must not be translated.
    pattern = r'(?:value:|sort-key:|field-key:)\s*"[^"]*"'
    assert baseline[relative] == re.findall(pattern, current), relative

details = (ROOT / "crates/gui/src/game_details.rs").read_text(encoding="utf-8")
modal = (ROOT / "ui/components/game-info-modal.slint").read_text(encoding="utf-8")
assert 'kind: "Disc"' in details and 'kind: "Game"' in details
assert 'k == "Game"' in modal
assert '"Disc" => ContentKind::Disc' in (ROOT / "crates/gui/src/update.rs").read_text(encoding="utf-8")
assert 'SharedString::from("Autre…")' in (ROOT / "crates/gui/src/update.rs").read_text(encoding="utf-8")
assert 'OTHER: "Autre…"' in (ROOT / "ui/components/storage-config-modal.slint").read_text(encoding="utf-8")
print(f"OK: {count} translations; placeholders, settings, sort keys and content types preserved.")
