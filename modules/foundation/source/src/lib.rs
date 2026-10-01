//! File identity and byte-based source locations shared by compiler layers.

/// Identifies one source file within an analysis session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileId(pub u32);

/// A half-open byte range in UTF-8 source text.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct TextRange {
    pub start: usize,
    pub end: usize,
}

impl TextRange {
    /// Returns a valid half-open range, if the end is not before the start.
    pub const fn new(start: usize, end: usize) -> Option<Self> {
        if start <= end {
            Some(Self { start, end })
        } else {
            None
        }
    }
}

/// A file-qualified source range.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SourceSpan {
    pub file: FileId,
    pub range: TextRange,
}

/// Monotonically changing input revision within an analysis session.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(pub u64);

/// Converts byte offsets to zero-based line and byte-column positions.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LineIndex {
    text: String,
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Indexes line starts without changing or normalizing the source text.
    pub fn new(text: &str) -> Self {
        let mut line_starts = vec![0];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self {
            text: text.to_owned(),
            line_starts,
        }
    }

    /// Returns a zero-based line and byte column at a UTF-8 boundary.
    pub fn line_col(&self, offset: usize) -> Option<(usize, usize)> {
        if offset > self.text.len() || !self.text.is_char_boundary(offset) {
            return None;
        }
        let line = self.line_starts.partition_point(|start| *start <= offset) - 1;
        Some((line, offset - self.line_starts[line]))
    }

    /// Returns a source byte offset from a zero-based line and byte column.
    pub fn offset(&self, line: usize, byte_col: usize) -> Option<usize> {
        let start = *self.line_starts.get(line)?;
        let offset = start.checked_add(byte_col)?;
        // The next line's start belongs to that line, even when the previous
        // line ends in CRLF. EOF belongs to the final line.
        let in_line = self
            .line_starts
            .get(line + 1)
            .map_or(offset <= self.text.len(), |next| offset < *next);
        (in_line && self.text.is_char_boundary(offset)).then_some(offset)
    }

    /// Returns the number of lines, including a trailing empty line after a newline.
    pub fn line_count(&self) -> usize {
        self.line_starts.len()
    }
}

#[cfg(test)]
mod tests;
