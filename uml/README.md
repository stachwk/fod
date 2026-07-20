# Diagramy UML projektu FOD

Katalog `uml/` jest stałym miejscem na diagramy PlantUML dokumentujące architekturę, przepływy, sekwencje i stany projektu FOD.

## Zasady

- źródła diagramów zapisujemy jako pliki `.puml`;
- nazwy plików powinny opisywać moduł i rodzaj diagramu;
- diagramy dotyczące jednego procesu mogą mieć wspólny prefiks;
- plik `*-all.puml` może grupować kilka powiązanych diagramów;
- wygenerowanych plików PNG/SVG nie trzeba commitować, jeśli można je odtworzyć ze źródeł `.puml`.

## FOD 3.2.20 — `fod-indexer file read`

- `fod-file-read-sequence.puml` — diagram sekwencji całego wywołania;
- `fod-file-read-activity.puml` — szczegółowy diagram aktywności i decyzji;
- `fod-file-read-components.puml` — diagram komponentów i przepływu danych;
- `fod-file-read-errors.puml` — maszyna stanów błędów;
- `fod-file-read-all.puml` — wszystkie cztery diagramy w jednym pliku.

## Renderowanie

```bash
plantuml -tsvg uml/fod-file-read-sequence.puml
plantuml -tpng uml/fod-file-read-activity.puml
```

Przy użyciu pliku JAR:

```bash
java -jar plantuml.jar -tsvg uml/fod-file-read-all.puml
```

Plik zbiorczy wygeneruje kilka osobnych obrazów.

## FOD 3.2.21 — catalogue snapshots

- `fod-catalog-snapshot-flow.puml` — creation, immutable reads, and deletion of catalogue snapshots.
