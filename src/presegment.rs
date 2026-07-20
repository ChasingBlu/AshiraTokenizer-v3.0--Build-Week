use crate::token::is_special_alias_sequence;
use std::error::Error;
use std::fmt;

pub const PRESEGMENTER_VERSION: &str = "ashira_v3_presegment_v1";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PresegmentStats {
    pub input_bytes: u64,
    pub logical_lines: u64,
    pub accepted_lines: u64,
    pub skipped_empty_lines: u64,
    pub skipped_special_only_lines: u64,
    pub accepted_line_bytes: u64,
    pub emitted_segments: u64,
    pub emitted_segment_bytes: u64,
    pub pair_opportunities: u64,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PresegmentError {
    ArithmeticOverflow { operation: &'static str },
    Consumer { message: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LosslessPresegment<'a> {
    Mergeable(&'a [u8]),
    LiteralByte(u8),
}

impl fmt::Display for PresegmentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ArithmeticOverflow { operation } => {
                write!(
                    formatter,
                    "pre-segmentation arithmetic overflow: {operation}"
                )
            }
            Self::Consumer { message } => {
                write!(formatter, "pre-segment consumer failed: {message}")
            }
        }
    }
}

impl Error for PresegmentError {}

pub const fn is_ashira_ascii_whitespace(byte: u8) -> bool {
    matches!(byte, b'\t' | b'\n' | 0x0B | 0x0C | b'\r' | b' ')
}

pub fn visit_presegments<F>(
    input: &[u8],
    mut consumer: F,
) -> Result<PresegmentStats, PresegmentError>
where
    F: FnMut(&[u8]) -> Result<(), PresegmentError>,
{
    let mut stats = PresegmentStats {
        input_bytes: u64::try_from(input.len()).map_err(|_| {
            PresegmentError::ArithmeticOverflow {
                operation: "input byte length conversion",
            }
        })?,
        ..PresegmentStats::default()
    };

    for raw_line in input.split(|byte| *byte == b'\n') {
        checked_increment(&mut stats.logical_lines, "logical line count")?;
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);
        if line.is_empty() {
            checked_increment(&mut stats.skipped_empty_lines, "empty line count")?;
            continue;
        }
        if is_special_alias_sequence(line) {
            checked_increment(
                &mut stats.skipped_special_only_lines,
                "special-only line count",
            )?;
            continue;
        }

        checked_increment(&mut stats.accepted_lines, "accepted line count")?;
        stats.accepted_line_bytes =
            checked_add_len(stats.accepted_line_bytes, line.len(), "accepted line bytes")?;
        visit_line_segments(line, &mut stats, &mut consumer)?;
    }

    Ok(stats)
}

/// Visits the same mergeable line segments as [`visit_presegments`] while
/// preserving structural CR/LF bytes that the training policy excludes.
///
/// Unlike training admission, this lossless view does not skip empty or
/// special-only logical lines. Consumers must still apply the locked special
/// alias policy within each `Mergeable` segment. A terminal CR and its LF are
/// emitted as literal base bytes so merges cannot cross a logical-line
/// boundary and decoding can reconstruct the original byte stream whenever no
/// non-canonical alias spelling was collapsed.
pub fn visit_lossless_presegments<F>(input: &[u8], mut consumer: F) -> Result<(), PresegmentError>
where
    F: FnMut(LosslessPresegment<'_>) -> Result<(), PresegmentError>,
{
    let mut line_start = 0usize;
    loop {
        let remaining = &input[line_start..];
        let newline_offset = remaining.iter().position(|byte| *byte == b'\n');
        let raw_line_end = match newline_offset {
            Some(offset) => {
                line_start
                    .checked_add(offset)
                    .ok_or(PresegmentError::ArithmeticOverflow {
                        operation: "lossless logical-line end",
                    })?
            }
            None => input.len(),
        };
        let raw_line = &input[line_start..raw_line_end];
        let line = raw_line.strip_suffix(b"\r").unwrap_or(raw_line);

        if !line.is_empty() {
            let mut ignored_stats = PresegmentStats::default();
            visit_line_segments(line, &mut ignored_stats, &mut |segment| {
                consumer(LosslessPresegment::Mergeable(segment))
            })?;
        }
        if line.len() != raw_line.len() {
            consumer(LosslessPresegment::LiteralByte(b'\r'))?;
        }

        match newline_offset {
            Some(offset) => {
                consumer(LosslessPresegment::LiteralByte(b'\n'))?;
                line_start = line_start
                    .checked_add(offset)
                    .and_then(|position| position.checked_add(1))
                    .ok_or(PresegmentError::ArithmeticOverflow {
                        operation: "lossless logical-line advance",
                    })?;
            }
            None => break,
        }
    }
    Ok(())
}

fn visit_line_segments<F>(
    line: &[u8],
    stats: &mut PresegmentStats,
    consumer: &mut F,
) -> Result<(), PresegmentError>
where
    F: FnMut(&[u8]) -> Result<(), PresegmentError>,
{
    let mut cursor = 0usize;
    while cursor < line.len() {
        let segment_start = cursor;
        while cursor < line.len() && is_ashira_ascii_whitespace(line[cursor]) {
            cursor += 1;
        }
        while cursor < line.len() && !is_ashira_ascii_whitespace(line[cursor]) {
            cursor += 1;
        }

        let segment = &line[segment_start..cursor];
        if segment.is_empty() {
            return Err(PresegmentError::ArithmeticOverflow {
                operation: "pre-segment cursor progress",
            });
        }
        let segment_bytes =
            u64::try_from(segment.len()).map_err(|_| PresegmentError::ArithmeticOverflow {
                operation: "pre-segment byte length conversion",
            })?;
        let next_count =
            stats
                .emitted_segments
                .checked_add(1)
                .ok_or(PresegmentError::ArithmeticOverflow {
                    operation: "pre-segment count",
                })?;
        let next_bytes = stats
            .emitted_segment_bytes
            .checked_add(segment_bytes)
            .ok_or(PresegmentError::ArithmeticOverflow {
                operation: "pre-segment bytes",
            })?;
        let opportunities = segment_bytes.saturating_sub(1);
        let next_opportunities = stats.pair_opportunities.checked_add(opportunities).ok_or(
            PresegmentError::ArithmeticOverflow {
                operation: "pair opportunities",
            },
        )?;

        consumer(segment)?;
        stats.emitted_segments = next_count;
        stats.emitted_segment_bytes = next_bytes;
        stats.pair_opportunities = next_opportunities;
    }
    Ok(())
}

fn checked_increment(value: &mut u64, operation: &'static str) -> Result<(), PresegmentError> {
    *value = value
        .checked_add(1)
        .ok_or(PresegmentError::ArithmeticOverflow { operation })?;
    Ok(())
}

fn checked_add_len(
    current: u64,
    additional: usize,
    operation: &'static str,
) -> Result<u64, PresegmentError> {
    let additional =
        u64::try_from(additional).map_err(|_| PresegmentError::ArithmeticOverflow { operation })?;
    current
        .checked_add(additional)
        .ok_or(PresegmentError::ArithmeticOverflow { operation })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(input: &[u8]) -> (Vec<Vec<u8>>, PresegmentStats) {
        let mut segments = Vec::new();
        let stats = visit_presegments(input, |segment| {
            segments.push(segment.to_vec());
            Ok(())
        })
        .expect("pre-segment fixture");
        (segments, stats)
    }

    #[test]
    fn logical_lines_lf_crlf_double_cr_and_unterminated_are_exact() {
        let (segments, stats) = collect(b"a\nb\r\nc\r\r\nlast");
        assert_eq!(
            segments,
            [
                b"a".to_vec(),
                b"b".to_vec(),
                b"c".to_vec(),
                b"\r".to_vec(),
                b"last".to_vec(),
            ]
        );
        assert_eq!(stats.logical_lines, 4);
        assert_eq!(stats.accepted_lines, 4);
        assert_eq!(stats.skipped_empty_lines, 0);
        assert_eq!(stats.emitted_segments, 5);
    }

    #[test]
    fn terminal_lf_preserves_and_skips_final_empty_logical_line() {
        let (segments, stats) = collect(b"a\n");
        assert_eq!(segments, [b"a".to_vec()]);
        assert_eq!(stats.logical_lines, 2);
        assert_eq!(stats.accepted_lines, 1);
        assert_eq!(stats.skipped_empty_lines, 1);
    }

    #[test]
    fn exact_ascii_whitespace_runs_attach_to_word_or_stand_alone() {
        let (segments, stats) = collect(b"\t\x0b\x0c\r lead  two \t");
        assert_eq!(
            segments,
            [
                b"\t\x0b\x0c\r lead".to_vec(),
                b"  two".to_vec(),
                b" \t".to_vec(),
            ]
        );
        assert_eq!(stats.pair_opportunities, 13);
        assert_eq!(stats.emitted_segment_bytes, 16);
        for byte in u8::MIN..=u8::MAX {
            let expected = matches!(byte, b'\t' | b'\n' | 0x0B | 0x0C | b'\r' | b' ');
            assert_eq!(is_ashira_ascii_whitespace(byte), expected, "byte {byte}");
        }
    }

    #[test]
    fn empty_special_only_and_alias_plus_whitespace_rules_are_distinct() {
        let (segments, stats) =
            collect(b"\n<KAREEM>\n<KAREEM></KAREEM>\n[[ANCHOR]][[/ANCHOR]]\n<KAREEM> \n");
        assert_eq!(segments, [b"<KAREEM>".to_vec(), b" ".to_vec()]);
        assert_eq!(stats.logical_lines, 6);
        assert_eq!(stats.skipped_empty_lines, 2);
        assert_eq!(stats.skipped_special_only_lines, 3);
        assert_eq!(stats.accepted_lines, 1);
    }

    #[test]
    fn whitespace_only_and_non_utf8_bytes_are_preserved() {
        let (segments, stats) = collect(b" \t\n\xff\xfe a");
        assert_eq!(
            segments,
            [b" \t".to_vec(), vec![0xFF, 0xFE], b" a".to_vec()]
        );
        assert_eq!(stats.accepted_lines, 2);
        assert_eq!(stats.emitted_segment_bytes, 6);
        assert_eq!(stats.pair_opportunities, 3);
    }

    #[test]
    fn segment_larger_than_hash_buffer_is_not_split_or_normalized() {
        let input = vec![b'x'; 16_385];
        let (segments, stats) = collect(&input);
        assert_eq!(segments, [input]);
        assert_eq!(stats.emitted_segments, 1);
        assert_eq!(stats.emitted_segment_bytes, 16_385);
        assert_eq!(stats.pair_opportunities, 16_384);
    }

    #[test]
    fn consumer_failure_stops_without_returning_partial_stats() {
        let error = visit_presegments(b"a b", |_| {
            Err(PresegmentError::Consumer {
                message: "injected".to_owned(),
            })
        })
        .expect_err("consumer failure must propagate");
        assert_eq!(
            error,
            PresegmentError::Consumer {
                message: "injected".to_owned()
            }
        );
    }

    #[test]
    fn lossless_view_shares_segments_and_preserves_structural_bytes() {
        let input = b"\n<KAREEM>\r\na  b\r\r\nlast";
        let mut pieces = Vec::new();
        visit_lossless_presegments(input, |piece| {
            pieces.push(match piece {
                LosslessPresegment::Mergeable(bytes) => (true, bytes.to_vec()),
                LosslessPresegment::LiteralByte(byte) => (false, vec![byte]),
            });
            Ok(())
        })
        .expect("lossless pre-segment fixture");

        assert_eq!(
            pieces,
            [
                (false, b"\n".to_vec()),
                (true, b"<KAREEM>".to_vec()),
                (false, b"\r".to_vec()),
                (false, b"\n".to_vec()),
                (true, b"a".to_vec()),
                (true, b"  b".to_vec()),
                (true, b"\r".to_vec()),
                (false, b"\r".to_vec()),
                (false, b"\n".to_vec()),
                (true, b"last".to_vec()),
            ]
        );
        let reconstructed: Vec<u8> = pieces
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        assert_eq!(reconstructed, input);
    }
}
