//! The reader: RFC 8259, strictly.

use crate::{Error, Json, Number, Object};

/// How deep arrays and objects may nest — `serde_json`'s limit, so a document
/// that read before reads now, and a hostile one cannot exhaust the stack.
const MAX_DEPTH: usize = 128;

pub(crate) fn parse(text: &str) -> Result<Json, Error> {
    let mut reader = Reader {
        text: text.as_bytes(),
        at: 0,
        depth: 0,
    };
    reader.whitespace();
    let value = reader.value()?;
    reader.whitespace();
    if reader.at != reader.text.len() {
        reader.at += 1;
        return Err(reader.error("trailing characters"));
    }
    Ok(value)
}

struct Reader<'a> {
    text: &'a [u8],
    at: usize,
    depth: usize,
}

impl Reader<'_> {
    /// An error at the current position, spelled as `serde_json` spelled it:
    /// the sentence, then `at line L column C` (1-based; the column counts
    /// bytes).
    fn error(&self, message: &str) -> Error {
        let before = &self.text[..self.at.min(self.text.len())];
        let line = before.iter().filter(|byte| **byte == b'\n').count() + 1;
        let column = before
            .iter()
            .rev()
            .take_while(|byte| **byte != b'\n')
            .count()
            .max(usize::from(self.at > 0 || !self.text.is_empty()));
        Error::new(format!("{message} at line {line} column {column}"))
    }

    fn peek(&self) -> Option<u8> {
        self.text.get(self.at).copied()
    }

    fn whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.at += 1;
        }
    }

    fn value(&mut self) -> Result<Json, Error> {
        match self.peek() {
            None => Err(self.error("EOF while parsing a value")),
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'"') => self.string().map(Json::String),
            Some(b'[') => self.array(),
            Some(b'{') => self.object(),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => {
                self.at += 1;
                Err(self.error("expected value"))
            }
        }
    }

    fn literal(&mut self, word: &[u8], value: Json) -> Result<Json, Error> {
        for expected in word {
            match self.peek() {
                None => return Err(self.error("EOF while parsing a value")),
                Some(byte) if byte == *expected => self.at += 1,
                Some(_) => {
                    self.at += 1;
                    return Err(self.error("expected ident"));
                }
            }
        }
        Ok(value)
    }

    fn enter(&mut self) -> Result<(), Error> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.error("recursion limit exceeded"));
        }
        Ok(())
    }

    fn array(&mut self) -> Result<Json, Error> {
        self.enter()?;
        self.at += 1;
        let mut items = Vec::new();
        self.whitespace();
        if self.peek() == Some(b']') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.whitespace();
            items.push(self.value()?);
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.whitespace();
                    if self.peek() == Some(b']') {
                        self.at += 1;
                        return Err(self.error("trailing comma"));
                    }
                }
                Some(b']') => {
                    self.at += 1;
                    self.depth -= 1;
                    return Ok(Json::Array(items));
                }
                None => return Err(self.error("EOF while parsing a list")),
                Some(_) => {
                    self.at += 1;
                    return Err(self.error("expected `,` or `]`"));
                }
            }
        }
    }

    fn object(&mut self) -> Result<Json, Error> {
        self.enter()?;
        self.at += 1;
        let mut object = Object::new();
        self.whitespace();
        if self.peek() == Some(b'}') {
            self.at += 1;
            self.depth -= 1;
            return Ok(Json::Object(object));
        }
        loop {
            self.whitespace();
            match self.peek() {
                Some(b'"') => {}
                None => return Err(self.error("EOF while parsing an object")),
                Some(_) => {
                    self.at += 1;
                    return Err(self.error("key must be a string"));
                }
            }
            let key = self.string()?;
            self.whitespace();
            match self.peek() {
                Some(b':') => self.at += 1,
                None => return Err(self.error("EOF while parsing an object")),
                Some(_) => {
                    self.at += 1;
                    return Err(self.error("expected `:`"));
                }
            }
            self.whitespace();
            let value = self.value()?;
            object.push(key, value);
            self.whitespace();
            match self.peek() {
                Some(b',') => {
                    self.at += 1;
                    self.whitespace();
                    if self.peek() == Some(b'}') {
                        self.at += 1;
                        return Err(self.error("trailing comma"));
                    }
                }
                Some(b'}') => {
                    self.at += 1;
                    self.depth -= 1;
                    return Ok(Json::Object(object));
                }
                None => return Err(self.error("EOF while parsing an object")),
                Some(_) => {
                    self.at += 1;
                    return Err(self.error("expected `,` or `}`"));
                }
            }
        }
    }

    fn number(&mut self) -> Result<Json, Error> {
        let start = self.at;
        let negative = self.peek() == Some(b'-');
        if negative {
            self.at += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.at += 1;
                if matches!(self.peek(), Some(b'0'..=b'9')) {
                    self.at += 1;
                    return Err(self.error("invalid number"));
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            None => return Err(self.error("EOF while parsing a value")),
            Some(_) => {
                self.at += 1;
                return Err(self.error("invalid number"));
            }
        }
        let mut float = false;
        if self.peek() == Some(b'.') {
            float = true;
            self.at += 1;
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                if self.peek().is_some() {
                    self.at += 1;
                }
                return Err(self.error("invalid number"));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            float = true;
            self.at += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.at += 1;
            }
            if !matches!(self.peek(), Some(b'0'..=b'9')) {
                if self.peek().is_some() {
                    self.at += 1;
                }
                return Err(self.error("invalid number"));
            }
            self.digits();
        }
        let text = std::str::from_utf8(&self.text[start..self.at]).expect("ASCII digits");
        if !float {
            // An integer past 64 bits reads as a float, as it did before.
            let integer = if negative {
                // `-0` is the float negative zero, as it always read.
                text.parse::<i64>()
                    .ok()
                    .filter(|value| *value != 0)
                    .map(Number::NegInt)
            } else {
                text.parse::<u64>().ok().map(Number::PosInt)
            };
            if let Some(number) = integer {
                return Ok(Json::Number(number));
            }
        }
        let value: f64 = text.parse().map_err(|_| self.error("invalid number"))?;
        if !value.is_finite() {
            return Err(self.error("number out of range"));
        }
        Ok(Json::Number(Number::Float(value)))
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.at += 1;
        }
    }

    fn string(&mut self) -> Result<String, Error> {
        self.at += 1;
        let mut out = String::new();
        loop {
            let start = self.at;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.at += 1;
            }
            // The input is a `&str`, so this stretch is whole characters.
            out.push_str(std::str::from_utf8(&self.text[start..self.at]).expect("UTF-8 input"));
            match self.peek() {
                None => return Err(self.error("EOF while parsing a string")),
                Some(b'"') => {
                    self.at += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.at += 1;
                    self.escape(&mut out)?;
                }
                Some(_) => {
                    self.at += 1;
                    return Err(self.error(
                        "control character (\\u0000-\\u001F) found while parsing a string",
                    ));
                }
            }
        }
    }

    fn escape(&mut self, out: &mut String) -> Result<(), Error> {
        let Some(byte) = self.peek() else {
            return Err(self.error("EOF while parsing a string"));
        };
        self.at += 1;
        match byte {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{8}'),
            b'f' => out.push('\u{c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let first = self.hex()?;
                let code = if (0xD800..0xDC00).contains(&first) {
                    if self.text.get(self.at..self.at + 2) != Some(b"\\u") {
                        return Err(self.error("lone leading surrogate in hex escape"));
                    }
                    self.at += 2;
                    let second = self.hex()?;
                    if !(0xDC00..0xE000).contains(&second) {
                        return Err(self.error("lone leading surrogate in hex escape"));
                    }
                    0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
                } else if (0xDC00..0xE000).contains(&first) {
                    return Err(self.error("lone leading surrogate in hex escape"));
                } else {
                    first
                };
                out.push(char::from_u32(code).expect("a scalar value outside the surrogates"));
            }
            _ => return Err(self.error("invalid escape")),
        }
        Ok(())
    }

    fn hex(&mut self) -> Result<u32, Error> {
        let mut code = 0;
        for _ in 0..4 {
            let Some(byte) = self.peek() else {
                return Err(self.error("EOF while parsing a string"));
            };
            self.at += 1;
            let digit = (byte as char)
                .to_digit(16)
                .ok_or_else(|| self.error("invalid escape"))?;
            code = code * 16 + digit;
        }
        Ok(code)
    }
}
