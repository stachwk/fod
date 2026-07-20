from pathlib import Path
import re

path = Path("rust_indexer/src/read_api.rs")
text = path.read_text(encoding="utf-8")

patterns = [
    r"\nimpl FileCatalogItem \{\n    fn human_readable\(&self\) -> String \{.*?\n    \}\n\}\n",
    r"\nimpl FileCatalogOutput \{\n    pub fn human_readable\(&self\) -> String \{.*?\n    \}\n\}\n",
    r"\nimpl FileShowOutput \{\n    pub fn human_readable\(&self\) -> String \{.*?\n    \}\n\}\n",
]

for pattern in patterns:
    text, count = re.subn(pattern, "\n", text, count=1, flags=re.S)
    if count != 1:
        raise SystemExit(f"expected exactly one dead renderer matching: {pattern}")

path.write_text(text, encoding="utf-8")
