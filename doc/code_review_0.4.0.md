# Code-Review fifi v0.4.0

Datum: 2026-06-09 · Toolchain: cargo 1.96.0, clippy 0.1.96

**Gesamtbild:** `cargo check` sauber, alle 64 Tests grün, keine Standard-Clippy-Warnungen
(nur Pedantic-Findings). Die Architektur (Scanner → Hardlink-Dedup → dreiphasige
Pipeline → Renderer) ist klar geschnitten, gut kommentiert und gut getestet.
Die Findings unten sind überwiegend Randfälle und Politur — ein echter
Logikfehler (B1) und eine Format-Inkonsistenz im Maschinen-Interface (B2)
stechen heraus.

---

## Bugs

### B1 (Mittel) — Bytewise-Vergleich: unlesbare Datei wird falsch attribuiert

`src/pipeline.rs:179-203` (`bucket_one_group_bytewise`)

Der erste Eintrag einer Gruppe wird ungeöffnet als Repräsentant in `subgroups`
abgelegt. Ist *dieser* Repräsentant unlesbar (Rechte entzogen, Datei zwischen
Scan und Vergleich gelöscht), schlägt `files_equal` für jeden nachfolgenden
Eintrag fehl — aber in `out.unreadable` landet jeweils der **nachfolgende,
lesbare** Eintrag. Der tatsächlich unlesbare Repräsentant bleibt als Singleton
übrig und wird am Ende als `unique` gemeldet. Eine einzige unlesbare Datei
kippt damit alle anderen Gruppenmitglieder nach `unreadable` und tarnt sich
selbst als lesbar — exakt invertiertes Ergebnis.

Tritt nur mit `--algo bytewise` auf; im Digest-Pfad öffnet `hasher.digest()`
genau die eine Datei, dort stimmt die Zuordnung.

**Fix-Vorschlag:** In `files_equal` unterscheidbar machen, welche Seite nicht
lesbar war (z. B. beide `File::open` getrennt ausführen und den Fehler mit dem
jeweiligen Pfad annotieren, etwa als `enum CompareError { Left(io::Error),
Right(io::Error), Io(io::Error) }`). Bei `Right`-Fehler den Repräsentanten
nach `unreadable` verschieben und die Untergruppe neu verankern.

### B2 (Mittel) — `--dupes-only` mischt NUL- und Newline-Trenner

`src/cli/render.rs:126-132` (`render_dupes_only`)

Innerhalb einer Gruppe trennt `\0`, zwischen Gruppen `\n`. Die Hilfe und das
README versprechen aber „NUL-delimited“. Für `xargs -0` ist der Strom damit
nicht sicher konsumierbar: der letzte Pfad von Gruppe N und der erste von
Gruppe N+1 verschmelzen zu einem Token `…letzter\nerster…`. Das README-Beispiel
(`tr '\0' '\n'`) funktioniert nur zufällig, weil es beide Trenner einebnet.

**Fix-Vorschlag:** Konsequent `\0` **nach jedem** Pfad ausgeben (auch am
Gruppenende). Wer Gruppenstruktur braucht, hat `--json`. Alternative, falls
die Gruppierung im Stream erhalten bleiben soll: Doppel-NUL als Gruppentrenner —
aber dann Doku und README-Beispiel anpassen. Achtung: Verhaltensänderung des
Maschinen-Interfaces → CHANGELOG-Eintrag als `fix:` oder `feat!:`.

### B3 (Niedrig) — `files_equal` behandelt Short-Reads als Ungleichheit

`src/pipeline.rs:142-154`

`Read::read` darf weniger Bytes liefern als angefordert, ohne dass EOF
erreicht ist (Netz-Dateisysteme, Signal-Unterbrechung). Liefert eine Seite
einen Short-Read, greift `na != nb → Ok(false)` und zwei identische Dateien
gelten als verschieden. Auf lokalen Dateisystemen praktisch nicht auslösbar,
aber das Projekt wirbt explizit mit SMB/CIFS-Robustheit (Hardlink-Dedup-Warnung).

**Fix-Vorschlag:** Kleine Helferfunktion, die den Puffer bis EOF oder
Füllstand nachliest (read-Schleife wie in `read_exact`, aber EOF-tolerant),
und erst dann vergleichen.

### B4 (Niedrig) — Überlappende oder doppelte Scan-Roots werden doppelt erfasst

`src/scanner.rs:170-179` (`walk_paths`)

`fifi /a /a/sub` oder `fifi x x` erfasst dieselben Dateien mehrfach. Im
Default-Modus fängt `dedup_hardlinks` das über `(dev, ino)` ab — erzeugt aber
einen Alias, der mit dem Kanonpfad identisch sein kann („x = x“ in der
Ausgabe). Mit `--per-path` entsteht aus jeder doppelt erfassten Datei ein
falsches Duplikat-Paar.

**Fix-Vorschlag:** Vor dem Walk die Roots kanonisieren und deduplizieren;
Roots, die in einem anderen Root enthalten sind, mit Warnung überspringen.
Minimal-Variante: nach dem Walk Einträge mit identischem Pfad deduplizieren.

### B5 (Niedrig) — Log-Zeile zählt Größengruppen falsch

`src/pipeline.rs:230-234`

`candidates.len() + unique.len()` zählt jede Unique-*Datei* als eigene
„size group“; mehrere leere Dateien bilden zudem eine Gruppe, werden aber
einzeln gezählt. Nur kosmetisch (Info-Log), aber die Zahl ist falsch.

**Fix-Vorschlag:** Gruppenzahl direkt in `split_by_size` ermitteln und
zurückgeben, oder die Meldung auf „… in N candidate group(s)“ umformulieren.

---

## Code-Qualität

### Q1 (Niedrig) — Toter Code

- `src/error.rs:8-13`: Die Variante `ScanError::Io` wird nirgends konstruiert.
  Entfernen (oder beim B1-Fix sinnvoll einsetzen).
- `src/scanner.rs:231-233`: `_ignore_path` ist ein leerer Platzhalter, dessen
  Underscore-Präfix die Dead-Code-Lint unterdrückt. Entfernen — Git erinnert
  sich, und der Plan steht im Projektgedächtnis/Changelog.
- `src/util.rs:69-79`: `path_sort` und `path_sort_paths` haben keine Aufrufer
  (weder intern noch via `lib.rs`-Re-Export). Entfernen oder bewusst als
  Public-API dokumentieren.

### Q2 (Niedrig) — Veraltete Doku am `available()`-Hook

`src/hash.rs:18-23`

Der Kommentar beschreibt `Sha256Hasher` als Platzhalter, der `available() →
false` überschreibt — SHA-256 ist aber längst implementiert und niemand
überschreibt `available()` mehr. Der Guard in `run_pipeline`
(`pipeline.rs:264-266`) ist damit für alle eingebauten Hasher toter Pfad.
Kommentar aktualisieren (Hook für externe `DigestHasher`-Implementierungen)
oder Hook samt `UnsupportedAlgo` entfernen.

### Q3 (Niedrig) — `natural_cmp` allokiert pro Vergleich

`src/util.rs:47-67`

Jeder Vergleich baut zwei `Vec<Chunk>` und ruft pro Text-Chunk zweimal
`to_lowercase()` — bei n·log n Vergleichen über große Ergebnislisten spürbar.

**Fix-Vorschlag:** `sort_by_cached_key` mit vorberechnetem Schlüssel
(`Vec<OwnedChunk>` mit bereits gefalteter Kleinschreibung).

### Q4 (Niedrig) — Formulierungen in `summary_line`

`src/cli/render.rs:149-176`

- „X duplicates out of N **files**“ — N ist die *Gruppen*zahl; „out of N
  originals“ oder „in N groups“ wäre präzise.
- `plural(n, "unreadable", "unreadable")` ist ein No-op — Aufruf weglassen.

### Q5 (Niedrig) — Vermeidbare Klone im Bytewise-Pfad

`src/pipeline.rs:183,195`

`entry.clone()` (inkl. `PathBuf` und Aliase) nur, um den Borrow auf
`subgroups` zu umgehen. Mit einem Match-Ergebnis-Index nach der Schleife
(`for`-Schleife liefert `Option<usize>`, Push danach) entfällt der Klon.

---

## Error-Handling

### E1 (Niedrig) — Unlesbare Verzeichnisse verschwinden im Debug-Log

`src/scanner.rs:139-148`

Walk-Fehler (typisch: `EACCES` auf ein Verzeichnis) werden auf `debug`
gedämpft und tauchen in keiner Statistik auf. Bei Default-Verbosity wirkt der
Scan vollständig, obwohl Teilbäume fehlen — relevant, weil Exit-Code 0
„keine Duplikate“ bedeutet und der Nutzer sich darauf verlässt.

**Fix-Vorschlag:** Permission-Fehler mindestens auf `warn` heben oder einen
Zähler (`skipped_dirs`) in `ScanResult`/Statistik aufnehmen. Loop-Erkennung
unter `--follow` kann auf `debug` bleiben.

---

## Typ-Sicherheit / Clippy (Pedantic)

Keine Findings auf Standard-Level. Pedantic meldet 35 Warnungen, die
lohnendsten:

| Lint | Stellen | Bewertung |
|---|---|---|
| `must_use_candidate` | 14× (Konstruktoren, `name()`, …) | sinnvoll für Lib-API |
| `redundant_closure` | `cli/output.rs:59,68` u. a. | `cargo clippy --fix` |
| `needless_lifetimes` | `cli/output.rs:55,63` | `cargo clippy --fix` |
| `ref_option` (`&Option<Arc<…>>`) | `scanner.rs:84` | `Option<&…>` nehmen |
| `cast_precision_loss` (i64→f64) | `scanner.rs:72-73` | akzeptabel — Zeitstempel passen in 52 Bit Mantisse; `#[allow]` mit Begründung |
| `struct_excessive_bools` | `scanner.rs:15` | bewusste CLI-Abbildung, ignorierbar |

Empfehlung: `cargo clippy --fix -- -W clippy::pedantic` für die mechanischen
Fälle, Rest selektiv mit begründetem `#[allow]`.

---

## Unsafe / Sicherheit

- Einziger `unsafe`-Block: `main.rs:43-48` (SIGPIPE auf Default). Korrekt und
  kommentiert; ein formaler `// SAFETY:`-Kommentar wäre die Kirsche obendrauf.
- Keine Credentials, keine Injection-Flächen. Dateinamen werden unverändert
  ausgegeben (`Path::display`/`to_string_lossy`) — Dateinamen mit
  ANSI-Escape-Sequenzen erreichen das Terminal ungefiltert. Üblich bei
  CLI-Tools dieser Art, der Vollständigkeit halber erwähnt.

---

## Tests

64 Tests (21 Unit, 43 Integration), alle grün. Abdeckung ist für die
Projektgröße stark: Partial-Hash-Grenzfälle, Symlink-Zyklen, unlesbare
Dateien, Tiefenbegrenzung, CLI-Konflikte, Parität zur Python-Vorlage.

Lücken (jeweils klein):

- **Bytewise + unlesbarer Repräsentant** — hätte B1 gefangen. Der bestehende
  Test `duplicates_still_found_when_one_file_unreadable` läuft nur über den
  Digest-Pfad.
- **`--dupes-only` über mehrere Gruppen hinweg** — die bestehenden Tests
  prüfen NUL-Zahl innerhalb einer Gruppe, nicht die Trenner-Semantik des
  Gesamtstroms (hätte B2 sichtbar gemacht).
- **Überlappende Roots** (B4).

---

## Zusammenfassung

> **Status 2026-06-09:** B1, B2, Q1 und Q2 wurden direkt nach dem Review
> umgesetzt (inkl. Regressionstests und CHANGELOG-Eintrag). Offen bleiben
> E1, B3, B4, B5, Q3–Q5 und die Pedantic-Politur.

| # | Finding | Priorität | Aufwand |
|---|---|---|---|
| B1 | Bytewise: unreadable falsch attribuiert | Mittel | ~1 h inkl. Test |
| B2 | `--dupes-only`: NUL/Newline-Mix | Mittel | ~30 min inkl. Doku |
| E1 | Unlesbare Verzeichnisse unsichtbar | Niedrig–Mittel | ~30 min |
| B3 | Short-Reads in `files_equal` | Niedrig | ~30 min |
| B4 | Überlappende Scan-Roots | Niedrig | ~45 min |
| Q1 | Toter Code (3 Stellen) | Niedrig | ~15 min |
| Q2 | Stale Doku `available()` | Niedrig | ~10 min |
| B5 | Log-Zeile Gruppenzahl | Niedrig | ~10 min |
| Q3 | `natural_cmp`-Allokationen | Niedrig | ~45 min |
| Q4/Q5 | Wording / Klone | Niedrig | ~20 min |
| — | Clippy-Pedantic-Politur | Niedrig | ~30 min |
