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
/// added later aren't a breaking change for code that builds one:
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
    /// Set with [`Limits::with_work`]; `None` scales with the program.
    work: Option<usize>,
}

/// The default work limit's floor, its steps per byte of program, and its
/// ceiling (see [`Limits::work_for`]).
const MIN_WORK: usize = 1_000_000;
const WORK_PER_BYTE: usize = 1_000;
const MAX_WORK: usize = 64_000_000;

impl Limits {
    /// The defaults: [`MAX_PROGRAM_SIZE`], [`MAX_RESULT_SIZE`],
    /// [`MAX_DEPTH`], and a work limit that scales with the program (see
    /// [`Limits::work_for`]).
    pub const DEFAULT: Limits = Limits {
        program_size: MAX_PROGRAM_SIZE,
        result_size: MAX_RESULT_SIZE,
        depth: MAX_DEPTH,
        work: None,
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
    /// the result is flagged `depth_limited`.
    pub const fn depth(&self) -> usize {
        self.depth
    }

    /// The work limit, if one was set with [`Limits::with_work`]; `None`
    /// means the default, which scales with the program (see
    /// [`Limits::work_for`]).
    pub const fn work(&self) -> Option<usize> {
        self.work
    }

    /// How many steps evaluating a program's state defaults may take, for a
    /// program of `program_bytes`: the limit set with [`Limits::with_work`],
    /// or by default the larger of a million steps and 1,000 steps per byte,
    /// never above 64 million.
    ///
    /// A step is one expression evaluated, one item sorted, filtered or
    /// copied into an `@Each` template, one node of a component or runtime
    /// expression read as a value, or one byte built. Past the limit the
    /// default being evaluated is null, as is every expression default after
    /// it, and the result is flagged `size_limited`.
    pub const fn work_for(&self, program_bytes: usize) -> usize {
        match self.work {
            Some(steps) => steps,
            None => {
                let scaled = program_bytes.saturating_mul(WORK_PER_BYTE);
                let floored = if scaled > MIN_WORK { scaled } else { MIN_WORK };
                if floored < MAX_WORK {
                    floored
                } else {
                    MAX_WORK
                }
            }
        }
    }

    /// These limits with the work limit set to `steps`, for every program
    /// (see [`Limits::work_for`]).
    #[must_use]
    pub const fn with_work(self, steps: usize) -> Limits {
        Limits {
            work: Some(steps),
            ..self
        }
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
