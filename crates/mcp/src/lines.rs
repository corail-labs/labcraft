//! The lines of stdio, one message each (`docs/mcp.md`, 1), read within a bound (section 5): a line longer than the bound
//! is dropped as it comes, never held, up to its line feed, and reading goes on after it.

use std::io::{BufRead, ErrorKind};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Line {
    /// A line, without its line feed and a `\r` before it.
    Text(Vec<u8>),
    /// A line longer than the bound, dropped.
    TooLong,
    /// The input closed, or cannot be read any more.
    End,
}

/// The next line of `input`, at most `max` bytes. A last line without a line feed counts as a line.
pub(crate) fn read_line(input: &mut impl BufRead, max: usize) -> Line {
    let mut line = Vec::new();
    let mut too_long = false;
    loop {
        let buffer = match input.fill_buf() {
            Ok(buffer) => buffer,
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(_) => return if too_long { Line::TooLong } else { Line::End },
        };
        if buffer.is_empty() {
            return if too_long {
                Line::TooLong
            } else if line.is_empty() {
                Line::End
            } else {
                Line::Text(trimmed(line))
            };
        }
        let feed = buffer.iter().position(|byte| *byte == b'\n');
        let taken = feed.unwrap_or(buffer.len());
        if !too_long {
            if line.len().saturating_add(taken) > max {
                too_long = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(buffer.get(..taken).unwrap_or_default());
            }
        }
        let consumed = feed.map_or(taken, |at| at + 1);
        input.consume(consumed);
        if feed.is_some() {
            return if too_long { Line::TooLong } else { Line::Text(trimmed(line)) };
        }
    }
}

fn trimmed(mut line: Vec<u8>) -> Vec<u8> {
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gives its bytes a few at a time, as a pipe does.
    struct Trickle<'a> {
        bytes: &'a [u8],
        at: usize,
        step: usize,
    }

    impl std::io::Read for Trickle<'_> {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            let rest = self.bytes.get(self.at..).unwrap_or_default();
            let n = rest.len().min(self.step).min(out.len());
            out.get_mut(..n).unwrap_or_default().copy_from_slice(rest.get(..n).unwrap_or_default());
            self.at += n;
            Ok(n)
        }
    }

    fn lines(bytes: &[u8], max: usize, step: usize) -> Vec<Line> {
        let mut input = std::io::BufReader::with_capacity(4, Trickle { bytes, at: 0, step });
        let mut lines = Vec::new();
        loop {
            let line = read_line(&mut input, max);
            if line == Line::End {
                return lines;
            }
            lines.push(line);
        }
    }

    #[test]
    fn lines_are_cut_at_their_feed_and_a_long_one_is_dropped() {
        let text = b"{}\r\n0123456789\nabc\n\nlast";
        for step in [1, 3, 64] {
            assert_eq!(
                lines(text, 5, step),
                vec![Line::Text(b"{}".to_vec()), Line::TooLong, Line::Text(b"abc".to_vec()), Line::Text(Vec::new()), Line::Text(b"last".to_vec()),],
                "step {step}"
            );
        }
    }

    #[test]
    fn a_line_of_exactly_the_bound_is_kept_and_one_byte_more_is_not() {
        assert_eq!(lines(b"12345\n123456\n", 5, 2), vec![Line::Text(b"12345".to_vec()), Line::TooLong]);
        assert_eq!(lines(b"123456", 5, 2), vec![Line::TooLong], "a long last line without a feed");
    }
}
