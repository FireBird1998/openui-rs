//! The one value every limit lives in (ADR 0001).

use crate::{MAX_AMPLIFICATION, MAX_DEPTH, MAX_PROGRAM_SIZE, MAX_RESULT_SIZE, MIN_RESULT_SIZE};

/// The bounds on what one parse may read, build and nest, with documented
/// defaults; an embedder may raise or lower each one. Hitting any of them
/// gives a cut result, flagged ([`crate::ParseResult::size_limited`] or
/// [`crate::ParseResult::depth_limited`]), never an error or a panic. They
/// count bytes and levels, never time, so a result is the same on every
/// machine.
///
/// The result-size limit scales with the program, bounding its
/// amplification: by default, the larger of 64 KiB and 64 times the
/// program's size, never above 8 MiB (see [`Limits::result_size_for`]).
///
/// Its fields are private and set through the `with_*` methods, so limits
/// added later aren't a breaking change for code that builds one:
///
/// ```
/// use openui_lang::Limits;
///
/// let limits = Limits::default().with_depth(1_000).with_program_size(64 * 1024);
/// assert_eq!(limits.depth(), 1_000);
/// // A 10 KB program's result may grow to 64 times its size.
/// assert_eq!(limits.result_size_for(10_000), 640_000);
/// // A fixed limit, whatever the program, as before scaling.
/// let fixed = limits.with_result_size(1024 * 1024);
/// assert_eq!(fixed.result_size_for(10), 1024 * 1024);
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Limits {
    program_size: usize,
    result_floor: usize,
    amplification: usize,
    result_ceiling: usize,
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
    /// The defaults: [`MAX_PROGRAM_SIZE`]; a result-size limit of the larger
    /// of [`MIN_RESULT_SIZE`] and [`MAX_AMPLIFICATION`] times the program,
    /// never above [`MAX_RESULT_SIZE`]; [`MAX_DEPTH`]; and a work limit that
    /// scales with the program (see [`Limits::work_for`]).
    pub const DEFAULT: Limits = Limits {
        program_size: MAX_PROGRAM_SIZE,
        result_floor: MIN_RESULT_SIZE,
        amplification: MAX_AMPLIFICATION,
        result_ceiling: MAX_RESULT_SIZE,
        depth: MAX_DEPTH,
        work: None,
    };

    /// How many bytes of a program are read; the rest is ignored as if the
    /// stream had stopped there, and the result is flagged `size_limited`.
    pub const fn program_size(&self) -> usize {
        self.program_size
    }

    /// The most a result may grow to, in bytes of its JSON, however large
    /// the program: the ceiling on [`Limits::result_size_for`].
    pub const fn result_size(&self) -> usize {
        self.result_ceiling
    }

    /// The result-size limit never scales below this many bytes, so a short
    /// program's result has room whatever its size.
    pub const fn result_floor(&self) -> usize {
        self.result_floor
    }

    /// How many times its program's size a result may grow to (above the
    /// floor, up to the ceiling).
    pub const fn amplification(&self) -> usize {
        self.amplification
    }

    /// About how large the result of a program of `program_bytes` may grow,
    /// in bytes of its JSON: [`Limits::amplification`] times the program,
    /// but no less than [`Limits::result_floor`] and no more than
    /// [`Limits::result_size`]. Past it, the rest becomes null and the result
    /// is flagged `size_limited`. A stream's limit is that of the text so
    /// far, so an early result can be cut and a later one not: each is the
    /// one-shot parse of the text so far.
    pub const fn result_size_for(&self, program_bytes: usize) -> usize {
        let scaled = self.amplification.saturating_mul(program_bytes);
        let at_least = if scaled > self.result_floor {
            scaled
        } else {
            self.result_floor
        };
        if at_least < self.result_ceiling {
            at_least
        } else {
            self.result_ceiling
        }
    }

    /// How deep expressions, reference chains and evaluated defaults may nest
    /// (defaults three JSON levels per level); deeper parts become null and
    /// the result is flagged `depth_limited`. The entry points that take JSON
    /// (reading it, converting it back to source, prompt specs) allow three
    /// JSON levels per level and 32 more, and refuse deeper input.
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

    /// These limits with a fixed result-size limit of `bytes`, whatever the
    /// program's size: the floor and the ceiling both become `bytes`.
    #[must_use]
    pub const fn with_result_size(self, bytes: usize) -> Limits {
        Limits {
            result_floor: bytes,
            result_ceiling: bytes,
            ..self
        }
    }

    /// These limits with the result-size floor set to `bytes`.
    #[must_use]
    pub const fn with_result_floor(self, bytes: usize) -> Limits {
        Limits {
            result_floor: bytes,
            ..self
        }
    }

    /// These limits with a result allowed `times` its program's size.
    #[must_use]
    pub const fn with_amplification(self, times: usize) -> Limits {
        Limits {
            amplification: times,
            ..self
        }
    }

    /// These limits with the result-size ceiling set to `bytes`
    /// ([`Limits::result_size`] reads it back).
    #[must_use]
    pub const fn with_result_ceiling(self, bytes: usize) -> Limits {
        Limits {
            result_ceiling: bytes,
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

    /// These limits for a program of `program_bytes`: the result-size and
    /// work limits fixed at what they scale to, so everything below reads one
    /// number from [`Limits::result_size`] and [`Limits::work_for`].
    pub(crate) const fn for_program(self, program_bytes: usize) -> Limits {
        self.with_result_size(self.result_size_for(program_bytes))
            .with_work(self.work_for(program_bytes))
    }
}

impl Default for Limits {
    fn default() -> Limits {
        Limits::DEFAULT
    }
}
