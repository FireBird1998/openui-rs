//! The one value every limit lives in (ADR 0001).

use crate::{MAX_DEPTH, MAX_PROGRAM_SIZE, MAX_RESULT_SIZE};

/// The bounds on what one parse may read, build and nest, with documented
/// defaults; an embedder may raise or lower each one. Hitting any of them
/// gives a cut result, flagged ([`crate::ParseResult::size_limited`] or
/// [`crate::ParseResult::depth_limited`]), never an error or a panic. They
/// count bytes and levels, never time, so a result is the same on every
/// machine.
///
/// Its fields are private and set through the `with_*` methods, so limits
/// added later (a limit on evaluating's work is planned) aren't a breaking
/// change for code that builds one:
///
/// ```
/// use openui_lang::Limits;
///
/// let limits = Limits::default().with_depth(1_000).with_program_size(64 * 1024);
/// assert_eq!(limits.depth(), 1_000);
/// assert_eq!(limits.result_size(), openui_lang::MAX_RESULT_SIZE);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Limits {
    program_size: usize,
    result_size: usize,
    depth: usize,
}

impl Limits {
    /// The defaults: [`MAX_PROGRAM_SIZE`], [`MAX_RESULT_SIZE`] and
    /// [`MAX_DEPTH`].
    pub const DEFAULT: Limits = Limits {
        program_size: MAX_PROGRAM_SIZE,
        result_size: MAX_RESULT_SIZE,
        depth: MAX_DEPTH,
    };

    /// How many bytes of a program are read; the rest is ignored as if the
    /// stream had stopped there, and the result is flagged `size_limited`.
    pub const fn program_size(&self) -> usize {
        self.program_size
    }

    /// About how large a result may grow, in bytes of its JSON; past it, the
    /// rest becomes null and the result is flagged `size_limited`.
    pub const fn result_size(&self) -> usize {
        self.result_size
    }

    /// How deep expressions, reference chains and evaluated defaults may nest
    /// (defaults three JSON levels per level); deeper parts become null and
    /// the result is flagged `depth_limited`. The entry points that take JSON
    /// (reading it, converting it back to source, prompt specs) allow three
    /// JSON levels per level and 32 more, and refuse deeper input.
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// These limits with the program-size limit set to `bytes`.
    #[must_use]
    pub const fn with_program_size(self, bytes: usize) -> Limits {
        Limits {
            program_size: bytes,
            ..self
        }
    }

    /// These limits with the result-size limit set to `bytes`.
    #[must_use]
    pub const fn with_result_size(self, bytes: usize) -> Limits {
        Limits {
            result_size: bytes,
            ..self
        }
    }

    /// These limits with the depth limit set to `levels`. The core grows its
    /// stack on demand (ADR 0002), so raising it needs no larger thread stack.
    #[must_use]
    pub const fn with_depth(self, levels: usize) -> Limits {
        Limits {
            depth: levels,
            ..self
        }
    }
}

impl Limits {
    /// How deep JSON may nest for the entry points that take JSON (reading
    /// it, converting it back to source, prompt specs): three JSON levels per
    /// level of the depth limit, as deep as a parse result under these
    /// limits nests (an evaluated default; measured at 764 of 768 at the
    /// default), and 32 more to spare. 800 at the default depth.
    pub(crate) const fn json_depth(&self) -> usize {
        self.depth.saturating_mul(3).saturating_add(32)
    }

    /// The part of `text` within `room` bytes, never splitting a character,
    /// and whether anything was left out.
    pub(crate) fn fit(text: &str, room: usize) -> (&str, bool) {
        if text.len() <= room {
            return (text, false);
        }
        let end = (0..=room)
            .rev()
            .find(|&i| text.is_char_boundary(i))
            .unwrap_or(0);
        (&text[..end], true)
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}
