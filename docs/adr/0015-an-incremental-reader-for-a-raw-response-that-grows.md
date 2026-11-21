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

Each call costs the line still arriving plus what is new. Tests assert, by
counting bytes looked at, that streaming 64 KiB in 16-byte chunks costs less
than three times what pushing the program does, and that streaming twice the
text costs about twice as much (32 to 256 KiB), for a bare program, a CRLF
message, a bundle and a program that goes on after its scripts section, in
both the core's and JavaScript's string forms. The count leaves out the core
form's comparison of the whole text with the last one, `bundle_json` writing
the program on every call (and the Swift side reading it back), and the
package's `startsWith`: each is one pass at memory speed over a text the
driver has already copied whole.

Known bounds, all within the program-size limit:

- One long line in a scripts section is looked at again on every call while
  it arrives, and a Cloud script's code is one JSON string, so its line is
  as long as its code. This is quadratic in that line and no better than
  before the reader: script code is not part of the program, so the push
  path never pays for it. Only the program-size limit bounds it. Measured by
  an independent review (native release, 16-byte chunks):

  | One script line | Core form | WebAssembly form | Push of the program |
  |---|---|---|---|
  | 16 KB | 0.23 s | 0.08 s | |
  | 64 KB | 2.9 s | 1.2 s | 0.018 s |
  | 256 KB | 11 s | 10 s | |
  | 1 MiB | 110 s | 88 s | |

  Committing the settled part of a line still arriving would remove it; that
  is its own ticket.
- Any other very long line still arriving is looked at again on each call,
  as the push path's last statement is.
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
