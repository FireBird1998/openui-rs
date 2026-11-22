---
status: accepted
---

# An incremental reader for a raw response that grows

Both raw-response drivers (`RuntimeSession.setResponse` in the WebAssembly
package, `LiveProgram.show(response:streaming:)` in Swift) are handed the
whole response so far on every chunk. Until #116 each call read the whole
text again with the message and bundle readers and parsed its whole program,
so a stream cost time quadratic in its length: in review, 64 KB in 16-byte
chunks took 3.2 s against 35 ms for push in release WebAssembly. The
maintainer decided on 2026-11-20 (#115, decision 8) that the fix lives in the
Rust core and that the drivers' entry points do not change.

## One reader in the core

`openui_lang::ResponseReader` owns the reading and the program's
`StreamParser`. It keeps the last accepted text, the message rules' state
after every line that can no longer change (the committed lines), the
current section's content and context, and the last program. A call goes
through only the text after the committed lines (normally the last line)
with the same rules, from that state, and gives the parser only the new part
of the program. Any edit that is not an append reads the text afresh.

A complete line can be committed because every rule of the message reader
that looks ahead (a retry marker inside a line, a bare scripts marker, the
content marker line the renderer looks for) looks at most at the rest of its
line, its line break included. The rules that depend on where the text ends
(holding back a last line that may be a marker, a retry marker still
arriving) touch only the last line. The one exception is the end marker,
which stops the reader when exactly one empty line follows it: a line with an
end marker is committed only once two more complete lines follow it. The
scripts section is found with a resumable copy of the same scan, and its JSON
is read only when it can have closed: while its brackets are open (and
shallower than the depth limit) the JSON reader certainly fails, so the
section is invalid without reading it.

The one-shot readers (`parse_response_bundle`, `parse_message`,
`response_draw_json_utf16`) are not reimplemented on top of the reader. They
stay as they were and are the reference: a differential test compares the
reader with them on every prefix, in many chunkings and after edits that are
not appends. Rejected: rewriting the one-shot readers as a single pass of the
incremental one. It would leave the differential test comparing the new code
with itself, and goldens alone with the old behaviour.

## Deciding that a text is an append

An append can only be known by looking at every byte of the old text in the
new one: any byte not looked at could have changed. Rejected: comparing a
length or a hash alone, which would take a different text of the same length
(or a collision) for an append and then give a wrong result.

- Swift hands over the whole text, so it has already been copied once per
  call; the reader compares it with its copy of the last accepted text, one
  more pass of the same length at memory speed, and the rules and the parser
  then see only what is new.
- In JavaScript the costly step is turning the string into the UTF-16 units
  that cross into WebAssembly, so the package keeps the last accepted string
  and compares with `startsWith` (native, at memory speed). Only the new units
  cross, with the accepted length. The reader answers an accepted length
  other than its own with a request for the whole text, which the package
  then sends once, so a package and a reader that disagree cannot produce a
  wrong read or a stuck session.

The reader also checks its own account of its parser: the parser must hold
as many bytes as the last accepted program, and must accept the number of
bytes the reader says are kept. Either check failing reads the program whole
against what the parser holds instead of guessing (and asserts in debug
builds, where it can only mean a bug).

In the native binding the reader belongs to the `StreamSession` and its
parser is the session's: `finish`, `push` and `set` after `readResponse` go
on from the response's program, as they did when the driver handed the
program to `set`. A push or set marks the reader's account stale, and the
next read compares its program whole with what the parser holds.

The reader's copy of the accepted text, its content and its program are each
bounded by the program-size limit.

## A refused update leaves the reader where it was

A read is staged and kept only when the draw that uses it succeeds (the
guarantee from #101). The parser used to stage by copying itself, a cost
proportional to the program on every chunk. It now keeps journals instead:
the text, the preprocessor's finished lines and the final part of its cleaned
text only grow, so a mark records their lengths and copies only the last
lines, which every push already goes through again; the statement table and
the materialization cache record what an update changed (reach counts,
definitions, replaced and dropped entries, moved rather than copied) and put
it back if the draw is refused. Rejected: rebuilding a parser from the
accepted text after a refusal, which gives equal results but new identities
for completed state defaults, which the session compares.

When a marker line starts to arrive the program loses the line break before
it. A stream parser cannot take text back, and parsing afresh would cost the
whole program each time; but the cleaned text a parser works on is trimmed,
so trailing CR and LF change only the raw text it keeps for the last line.
The parser takes them back directly, and a staged update that took them back keeps them so that a refusal puts them back.

## What it costs

Each call costs the line still arriving plus what is new (only what is new
while that line is a script line the reader can resume, below). Tests assert, by
counting bytes looked at, that streaming 64 KiB in 16-byte chunks costs less
than three times what pushing the program does, and that streaming twice the
text costs about twice as much (32 to 256 KiB), for a bare program, a CRLF
message, a bundle, a program that goes on after its scripts section and a
script whose code is the whole size on one line, in both the core's and
JavaScript's string forms. The count leaves out the core
form's comparison of the whole text with the last one, `bundle_json` writing
the program on every call (and the Swift side reading it back), and the
package's `startsWith`: each is one pass at memory speed over a text the
driver has already copied whole.

Known bounds, all within the program-size limit:

- A very long line still arriving that is not a resumable script line
  (below) is looked at again on each call, as the push path's last
  statement is. Measured by an independent review at 64 KiB in 16-byte
  chunks, in operations (core form / UTF-16 form), against about 1.86M
  for a resumed script line:
  - script code holding `]]>` near its start, as `x[y[0]]>z` would (rare
    but plausible in real code): 1,616M / 1,482M, never resumed;
  - the same with the `]]>` at 90% of the code: 309M / 283M (resumed
    until it arrives);
  - a long line in a context section: 1,480M;
  - a long program line: 1,479M / 1,345M (the push path pays for this
    one too);
  - a long line after an unknown marker following the scripts section:
    1,346M / 1,211M.

  A script line nested past the JSON depth limit, and one whose array
  closed with only whitespace after it, also take the full path (the array
  is then read on every call, as before). A raw line break inside a
  script's JSON string is invalid JSON, but each such line is committed on
  its own, so that stays linear.
- A context section is read and returned on every call; it is part of each
  call's result.
- A scripts section nested past the JSON depth limit is read on every call,
  as before.
- In WebAssembly, a program holding a lone surrogate (or U+FDD0) is compared
  whole with the last one, because the parser takes it as the package's
  string conversion did before, lone surrogates replaced.

A program that goes on after its scripts section (past the next `]]>`, which
need not be a marker) was compared again from the section on, every call:
137M bytes at 64 KiB against the push path's 1.07M. The comparison now skips
the part after the section too, as far as the committed content reaches.

## A long line in a scripts section (#122)

A Cloud script's code is one JSON string, so its line is as long as its
code, and until #122 every call looked at the whole of that line again
while it arrived (normalised, masked, split for retries, folded, scanned):
quadratic in the line and no better than before the reader, since script
code is not part of the program and the push path never pays for it.
Measured in #116's review (native release, 16-byte chunks): 64 KB took
2.9 s in the core form and 1.2 s in the WebAssembly form, 1 MiB 110 s.

The reader now keeps, with the accepted text, the scripts scan's state at
the end of the line still arriving (`LineScan`: where the line starts, how
far the raw text was scanned, the scan's position in the content and its
stage; a few numbers, whatever the line's length). The next append goes on
from there with only the new bytes, when:

- the text after the committed lines is still one line, with no line break
  and no `]]>` (the new bytes are checked with the two before them, so a
  `]]>` split across chunks is seen), and longer than `]]`;
- the committed lines leave that line in the content, in a scripts section
  that has not closed, with no context in either view;
- and after the new bytes the scan is still in the array and the array
  certainly fails to read (open, or text after it), as `Shape::fails` says.

Why that gives the full read's result: without `]]>` no bare scripts marker
can be masked and no retry split or held, the line is no marker line and
no content marker line, and a last line is held back as a marker that may
still arrive only if it is `]` or `]]`. So both folds take it as one line
of content, the content is the committed content, a line break and the
line, and only the scripts scan looks into it, which stops and continues at
any character boundary (the commit already relies on that). The program is
the committed content before the section, so it is compared as before.
Anything else falls back to reading the line again, which also builds the
scan for the next call once (in the commit, from the committed scan and the
whole line). The scan stops before a lone high surrogate at the end, which
the next chunk may pair (the core joins the pair; the WebAssembly reader
replaces its escape), and before a held-back `/`, `]` or `]]`.

The scan is accepted state like the committed lines: a staged read carries
the scan it reached and the commit installs it, so a refused or discarded
read leaves it where the last accepted read left it. A line that can never
be resumed (it holds a line break or `]]>`, or the committed lines put it
elsewhere) is marked so and not rebuilt until a line is committed.

The ticket defined a quiet line as one without `]]`. The reader asks only
for no `]]>`: every rule that could make such a line more than one line of
content needs `]]>`, and code often holds `]]` (`rows[ids[0]]`), which
would have sent those lines back to the quadratic path.

Measured (native release, 16-byte chunks, operations are bytes the reader
looks at plus what its parser takes in; the same harness as before):

| Script code on one line | Core form | WebAssembly form |
|---|---|---|
| 64 KiB, before | 0.50 s, 1,606M | 0.46 s, 1,472M |
| 64 KiB, after | 0.013 s, 1.85M | 0.005 s, 1.72M |
| 256 KiB, before | 8.1 s, 25,724M | 7.4 s, 23,580M |
| 256 KiB, after | 0.064 s, 7.4M | 0.009 s, 6.9M |
| 1 MiB, before | 129 s, 411,890M | 122 s, 377,568M |
| 1 MiB, after | 0.68 s, 29.6M | 0.037 s, 27.5M |

The WebAssembly "before" times include about 0.07 s, 1 s and 15 s (64 KiB,
256 KiB, 1 MiB) of the harness counting UTF-16 units of the whole prefix on
every chunk, found and removed before the "after" runs.

The count now doubles with the line (2.00× per doubling, 32 to 256 KiB, in
both forms), and 64 KiB on one line costs 0.78× what the same code costs as
many one-script lines. What still grows faster than the line in the core
form is the comparison of the whole text with the last one, at memory
speed and outside the count as before.

The differential test streams script lines of at least 64 KiB with chunk
edges inside escapes, `\uXXXX` sequences, surrogate pairs and multi-byte
characters, around CR and CRLF, with `]]` and `]]>` inside the string, and
lines that later stop being resumable (a line break and a marker, a retry
mid-line, the array closing then junk), with a quarter of the updates
refused and followed by other appends, edits and shrinks (into the middle
of the line too), and under a program-size limit that falls inside the
line. Breaking the line-break, `]]>`, `]`-or-`]]`, array-fails or
position checks on purpose makes it fail, as does keeping a scan the commit
should replace, or removing the content check from both places it is made.
A separate test covers the marker line's depth check (below). The checks
that cannot fail (the scan's line and offsets against the accepted text,
a context in either view) are kept to fall back; leaving a trailing lone
high half to the next call and joining pairs in the resumed bytes keep the
scan's position a true content offset and change no result.

Rejected:

- Committing the settled part of the line into the committed state (the
  idea this ADR first named). It mixes half lines into a model built on
  whole lines, and a line that later turns out not to be resumable (a
  retry mid-line, `]]>` arriving) would have to be taken back out of the
  folds; the separate scan is dropped instead, and the full path is
  unchanged.
- Keeping the scan as an `Extract` with its count and closed-array read.
  Cloning those on every call costs the marker line's JSON again; the scan
  keeps only the stage. The marker line's JSON is checked once when the
  scan is built, and no scan is built if it is nested past the depth
  limit. The check is reachable when the accepted text ends in a held-back
  `]` or `]]`: its own read then had no line break after the marker line,
  never read that JSON, and the next append makes the full read refuse it.
- Resuming a line whose array has closed with only whitespace after it, or
  one nested past the depth limit. Both read the scripts JSON on every
  call in the full path anyway, so resuming the scan saves nothing there.

(2026-11-23: since #206 the native session also takes a response one
appended piece at a time, `append_response`, with `read_response`'s
refusals but no outputs, and reads the accepted text again to draw,
`reread_response`; `ResponseReader::read_append` stages exactly what `read`
of the whole text stages, and `ResponseReader::keep` keeps a read without
parsing it, the parser taking every kept piece in one update at the next
parse, because a parse result costs the whole program. The entry points
above are unchanged.)
