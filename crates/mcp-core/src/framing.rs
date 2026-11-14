//! MCP stdio transport 프레이밍 (XIX.1.1).
//!
//! 메시지 하나는 개행(`\n`)으로 끝나는 한 줄의 JSON입니다. 메시지 안에는 개행이 없어야 합니다.
//! reader는 최대 크기를 넘는 줄을 메모리에 쌓지 않고 다음 개행까지 버린 뒤
//! [`Frame::TooLarge`]로 알립니다. writer는 개행 없는 JSON([`crate::json::write`])에 개행 하나를
//! 붙여 쓰고 즉시 flush합니다.

use std::io::{self, BufRead, Write};

use crate::json::{self, JsonValue};

/// 읽은 frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// 메시지 한 줄(끝 개행 제외).
    Message(Vec<u8>),
    /// 최대 크기를 넘어 버린 줄.
    TooLarge {
        /// 버린 byte 수(개행 제외).
        discarded: usize,
    },
    /// 입력 끝.
    Eof,
}

/// 한 frame을 읽습니다. 입력 끝의 개행 없는 마지막 줄도 메시지로 반환합니다.
pub fn read_frame<R: BufRead>(reader: &mut R, max_bytes: usize) -> io::Result<Frame> {
    let mut message = Vec::new();
    let mut total = 0usize;
    let mut overflow = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        if available.is_empty() {
            return Ok(if total == 0 && !overflow {
                Frame::Eof
            } else if overflow {
                Frame::TooLarge { discarded: total }
            } else {
                Frame::Message(message)
            });
        }
        let newline = available.iter().position(|&b| b == b'\n');
        let chunk_len = newline.unwrap_or(available.len());
        total += chunk_len;
        if !overflow {
            if message.len() + chunk_len > max_bytes {
                overflow = true;
                message = Vec::new();
            } else {
                message.extend_from_slice(&available[..chunk_len]);
            }
        }
        let consumed = chunk_len + usize::from(newline.is_some());
        reader.consume(consumed);
        if newline.is_some() {
            return Ok(if overflow {
                Frame::TooLarge { discarded: total }
            } else {
                Frame::Message(message)
            });
        }
    }
}

/// 메시지 하나를 개행으로 끝나는 한 줄로 쓰고 flush합니다.
pub fn write_frame<W: Write>(writer: &mut W, message: &JsonValue) -> io::Result<()> {
    let mut line = json::write(message);
    // writer는 개행을 항상 이스케이프하므로 줄 안에 개행이 없습니다.
    if line.contains('\n') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "프레임 안에 개행이 있습니다",
        ));
    }
    line.push('\n');
    writer.write_all(line.as_bytes())?;
    writer.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    fn frames(input: &[u8], max: usize, capacity: usize) -> Vec<Frame> {
        let mut reader = BufReader::with_capacity(capacity, Cursor::new(input.to_vec()));
        let mut out = Vec::new();
        loop {
            let frame = read_frame(&mut reader, max).unwrap();
            let end = frame == Frame::Eof;
            out.push(frame);
            if end {
                return out;
            }
        }
    }

    fn msg(text: &str) -> Frame {
        Frame::Message(text.as_bytes().to_vec())
    }

    #[test]
    fn splits_on_newlines_and_keeps_unterminated_tail() {
        for capacity in [1, 3, 8192] {
            assert_eq!(
                frames(b"{\"a\":1}\n\n[2]\r\n{}", 64, capacity),
                vec![
                    msg("{\"a\":1}"),
                    msg(""),
                    msg("[2]\r"),
                    msg("{}"),
                    Frame::Eof
                ],
                "capacity {capacity}"
            );
        }
        assert_eq!(frames(b"", 64, 16), vec![Frame::Eof]);
    }

    #[test]
    fn oversized_line_is_discarded_and_next_frame_survives() {
        for capacity in [1, 4, 8192] {
            let mut input = vec![b'x'; 40];
            input.extend_from_slice(b"\n{}\n");
            input.extend(vec![b'y'; 20]);
            assert_eq!(
                frames(&input, 16, capacity),
                vec![
                    Frame::TooLarge { discarded: 40 },
                    msg("{}"),
                    Frame::TooLarge { discarded: 20 },
                    Frame::Eof
                ],
                "capacity {capacity}"
            );
        }
        // 정확히 최대 크기인 줄은 허용합니다.
        assert_eq!(
            frames(b"0123456789\n", 10, 8192),
            vec![msg("0123456789"), Frame::Eof]
        );
    }

    #[test]
    fn written_frames_are_single_lines() {
        let value = JsonValue::object(vec![("text", JsonValue::string("a\nb\r\nc"))]);
        let mut out = Vec::new();
        write_frame(&mut out, &value).unwrap();
        write_frame(&mut out, &JsonValue::Null).unwrap();
        assert_eq!(out, b"{\"text\":\"a\\nb\\r\\nc\"}\nnull\n");
    }
}
