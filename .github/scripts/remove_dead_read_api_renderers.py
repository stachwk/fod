from pathlib import Path

path = Path("rust_indexer/src/read_api.rs")
text = path.read_text(encoding="utf-8")

markers = [
    "impl FileCatalogItem {",
    "impl FileCatalogOutput {",
    "impl FileShowOutput {",
]

for marker in markers:
    start = text.find(marker)
    if start < 0:
        raise SystemExit(f"missing dead renderer block: {marker}")

    line_start = text.rfind("\n", 0, start) + 1
    depth = 0
    end = None
    for index in range(start, len(text)):
        char = text[index]
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                end = index + 1
                break

    if end is None:
        raise SystemExit(f"unterminated block: {marker}")

    while end < len(text) and text[end] == "\n":
        end += 1

    text = text[:line_start] + text[end:]

path.write_text(text, encoding="utf-8")
