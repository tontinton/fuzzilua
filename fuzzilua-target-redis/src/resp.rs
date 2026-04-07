use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

use thiserror::Error;

const MAX_BULK_LEN: usize = 512 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum RespValue {
    SimpleString(String),
    Error(String),
    Integer(i64),
    BulkString(String),
    Null,
    Array(Vec<RespValue>),
}

#[derive(Debug, Error)]
pub enum RespError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("protocol error: {0}")]
    Protocol(String),
}

pub struct RespClient {
    reader: BufReader<TcpStream>,
}

impl RespClient {
    pub fn new(stream: TcpStream) -> Self {
        Self {
            reader: BufReader::new(stream),
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<std::time::Duration>) -> std::io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)
    }

    pub(crate) fn send_command(&mut self, args: &[&str]) -> Result<(), RespError> {
        let mut buf = Vec::with_capacity(64);
        encode_command_into(args, &mut buf);
        self.reader.get_mut().write_all(&buf)?;
        Ok(())
    }

    pub(crate) fn read_response(&mut self) -> Result<RespValue, RespError> {
        parse_value(&mut self.reader)
    }

    pub fn command(&mut self, args: &[&str]) -> Result<RespValue, RespError> {
        self.send_command(args)?;
        self.read_response()
    }
}

fn read_line<R: BufRead>(reader: &mut R) -> Result<String, RespError> {
    let mut line = String::new();
    let n = reader.read_line(&mut line)?;
    if n == 0 {
        return Err(RespError::Protocol("unexpected EOF".into()));
    }
    if !line.ends_with("\r\n") {
        return Err(RespError::Protocol("missing CRLF".into()));
    }
    line.truncate(line.len() - 2);
    Ok(line)
}

fn parse_value<R: BufRead + Read>(reader: &mut R) -> Result<RespValue, RespError> {
    let line = read_line(reader)?;
    if line.is_empty() {
        return Err(RespError::Protocol("empty line".into()));
    }

    let (prefix, payload) = line.split_at(1);
    match prefix {
        "+" => Ok(RespValue::SimpleString(payload.to_string())),
        "-" => Ok(RespValue::Error(payload.to_string())),
        ":" => {
            let n = payload
                .parse::<i64>()
                .map_err(|e| RespError::Protocol(format!("invalid integer: {e}")))?;
            Ok(RespValue::Integer(n))
        }
        "$" => {
            let len = payload
                .parse::<i64>()
                .map_err(|e| RespError::Protocol(format!("invalid bulk length: {e}")))?;
            if len == -1 {
                return Ok(RespValue::Null);
            }
            if len < 0 {
                return Err(RespError::Protocol(format!("negative bulk length: {len}")));
            }
            let len = len as usize;
            if len > MAX_BULK_LEN {
                return Err(RespError::Protocol(format!("bulk string too large: {len}")));
            }
            let mut buf = vec![0u8; len + 2];
            reader.read_exact(&mut buf)?;
            if &buf[len..] != b"\r\n" {
                return Err(RespError::Protocol(
                    "bulk string missing trailing CRLF".into(),
                ));
            }
            buf.truncate(len);
            let s = String::from_utf8(buf)
                .map_err(|e| RespError::Protocol(format!("invalid UTF-8 in bulk string: {e}")))?;
            Ok(RespValue::BulkString(s))
        }
        "*" => {
            let count = payload
                .parse::<i64>()
                .map_err(|e| RespError::Protocol(format!("invalid array length: {e}")))?;
            if count == -1 {
                return Ok(RespValue::Null);
            }
            if count < 0 {
                return Err(RespError::Protocol(format!(
                    "negative array length: {count}"
                )));
            }
            let count = count as usize;
            let mut items = Vec::with_capacity(count);
            for _ in 0..count {
                items.push(parse_value(reader)?);
            }
            Ok(RespValue::Array(items))
        }
        _ => Err(RespError::Protocol(format!("unknown RESP type: {prefix}"))),
    }
}

fn encode_command_into(args: &[&str], buf: &mut Vec<u8>) {
    buf.push(b'*');
    buf.extend_from_slice(args.len().to_string().as_bytes());
    buf.extend_from_slice(b"\r\n");
    for arg in args {
        buf.push(b'$');
        buf.extend_from_slice(arg.len().to_string().as_bytes());
        buf.extend_from_slice(b"\r\n");
        buf.extend_from_slice(arg.as_bytes());
        buf.extend_from_slice(b"\r\n");
    }
}

#[cfg(test)]
fn parse_resp(input: &[u8]) -> Result<RespValue, RespError> {
    let mut cursor = std::io::Cursor::new(input);
    parse_value(&mut cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    #[test_case(b"+OK\r\n", RespValue::SimpleString("OK".into()) ; "simple_string")]
    #[test_case(b"+hello world\r\n", RespValue::SimpleString("hello world".into()) ; "simple_string_with_space")]
    #[test_case(b"-ERR unknown command\r\n", RespValue::Error("ERR unknown command".into()) ; "error")]
    #[test_case(b":42\r\n", RespValue::Integer(42) ; "positive_integer")]
    #[test_case(b":-1\r\n", RespValue::Integer(-1) ; "negative_integer")]
    #[test_case(b":0\r\n", RespValue::Integer(0) ; "zero_integer")]
    #[test_case(b"$5\r\nhello\r\n", RespValue::BulkString("hello".into()) ; "bulk_string")]
    #[test_case(b"$0\r\n\r\n", RespValue::BulkString("".into()) ; "empty_bulk_string")]
    #[test_case(b"$-1\r\n", RespValue::Null ; "null_bulk_string")]
    #[test_case(b"*-1\r\n", RespValue::Null ; "null_array")]
    #[test_case(b"*0\r\n", RespValue::Array(vec![]) ; "empty_array")]
    #[test_case(b"*3\r\n:1\r\n:2\r\n:3\r\n", RespValue::Array(vec![RespValue::Integer(1), RespValue::Integer(2), RespValue::Integer(3)]) ; "homogeneous_array")]
    #[test_case(b"*3\r\n+OK\r\n:100\r\n$5\r\nhello\r\n", RespValue::Array(vec![RespValue::SimpleString("OK".into()), RespValue::Integer(100), RespValue::BulkString("hello".into())]) ; "mixed_array")]
    #[test_case(b"*2\r\n*2\r\n:1\r\n:2\r\n*2\r\n:3\r\n:4\r\n", RespValue::Array(vec![RespValue::Array(vec![RespValue::Integer(1), RespValue::Integer(2)]), RespValue::Array(vec![RespValue::Integer(3), RespValue::Integer(4)])]) ; "nested_array")]
    fn parse_resp_values(input: &[u8], expected: RespValue) {
        let result = parse_resp(input).unwrap();
        assert_eq!(result, expected);
    }

    #[test_case(b"$10\r\nhello\r\n" ; "truncated_bulk_string")]
    #[test_case(b"+OK" ; "missing_crlf")]
    #[test_case(b"~invalid\r\n" ; "unknown_type")]
    #[test_case(b"" ; "empty_input")]
    fn parse_resp_rejects_malformed(input: &[u8]) {
        assert!(parse_resp(input).is_err());
    }

    #[test]
    fn encode_command_format() {
        let mut buf = Vec::new();
        encode_command_into(&["EVAL", "return 1", "0"], &mut buf);
        assert_eq!(buf, b"*3\r\n$4\r\nEVAL\r\n$8\r\nreturn 1\r\n$1\r\n0\r\n");
    }
}
