# Word frequency resources

Used by `core/spellcheck.rs` to rank spelling-correction candidates by how
common they are (see that module's doc comment for why: plain edit-distance
alone can't tell "simple" from "smile" — frequency can).

- `en_top10k.txt` — the 10,000 most frequent English words, one per line,
  ordered by frequency (most common first). Source:
  https://github.com/first20hours/google-10000-english (derived from
  Google's Trillion Word Corpus). Repo license is unspecified
  ("NOASSERTION" on GitHub) — fine for this project's own local use; if
  DocuMind is ever distributed more broadly, swap for a clearly-licensed
  frequency list (e.g. one derived from Wiktionary or a corpus with an
  explicit open license).

- `vi_syllables.tsv` — `word<TAB>count`, one Vietnamese syllable per line,
  sorted by frequency (most common first). Derived by this project (not a
  raw upstream file): built by tokenizing
  https://github.com/cytauxzoonosis/vietnamese100K's `merged.txt` (a merge
  of several Vietnamese word/phrase lists) into individual syllables and
  counting occurrences. That source repo carries no explicit license
  either — same caveat as above applies, more acutely since it's itself a
  merge of multiple uncredited sources. This is also why DocuMind's
  installed `hunspell-vi` dictionary was missing common words like "hóa" /
  "hòa" / "thỏa": that dictionary only has ~6,600 entries and happens to
  use the *old-style* tone-mark placement convention for some diphthongs,
  while this derived list, being usage-frequency-based, naturally covers
  both placement conventions.

Regenerate `vi_syllables.tsv` with:
```
python3 - <<'PY'
import re
from collections import Counter
c = Counter()
with open("merged.txt", encoding="utf-8") as f:
    for line in f:
        for w in line.strip().split():
            w = w.lower()
            if re.fullmatch(r"[a-zàáảãạăằắẳẵặâầấẩẫậđèéẻẽẹêềếểễệìíỉĩịòóỏõọôồốổỗộơờớởỡợùúủũụưừứửữựỳýỷỹỵ]+", w):
                c[w] += 1
with open("vi_syllables.tsv", "w", encoding="utf-8") as out:
    for word, count in c.most_common():
        out.write(f"{word}\t{count}\n")
PY
```
