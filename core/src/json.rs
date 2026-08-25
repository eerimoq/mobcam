//! Just enough JSON to read the string fields out of Moblin's device hello.

/// Reads the top level string fields of a JSON object. Nested objects, arrays
/// and non-string values are skipped, since the device hello has none.
pub fn string_fields(json: &str) -> Option<Vec<(String, String)>> {
    let mut parser = Parser {
        bytes: json.as_bytes(),
        position: 0,
    };
    let fields = parser.object()?;
    parser.skip_whitespace();
    parser.at_end().then_some(fields)
}

pub fn field(fields: &[(String, String)], name: &str) -> String {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.clone())
        .unwrap_or_default()
}

struct Parser<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl Parser<'_> {
    fn at_end(&self) -> bool {
        self.position >= self.bytes.len()
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.position).copied()
    }

    fn take(&mut self) -> Option<u8> {
        let byte = self.peek()?;
        self.position += 1;
        Some(byte)
    }

    fn expect(&mut self, byte: u8) -> Option<()> {
        (self.take()? == byte).then_some(())
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.position += 1;
        }
    }

    fn object(&mut self) -> Option<Vec<(String, String)>> {
        let mut fields = Vec::new();
        self.skip_whitespace();
        self.expect(b'{')?;
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.position += 1;
            return Some(fields);
        }
        loop {
            self.skip_whitespace();
            let key = self.string()?;
            self.skip_whitespace();
            self.expect(b':')?;
            self.skip_whitespace();
            if self.peek() == Some(b'"') {
                fields.push((key, self.string()?));
            } else {
                self.skip_value()?;
            }
            self.skip_whitespace();
            match self.take()? {
                b',' => continue,
                b'}' => return Some(fields),
                _ => return None,
            }
        }
    }

    /// Skips a value that is not a string, leaving the position right after it.
    fn skip_value(&mut self) -> Option<()> {
        match self.peek()? {
            b'{' | b'[' => self.skip_nested(),
            _ => {
                let start = self.position;
                while !matches!(
                    self.peek(),
                    None | Some(b',' | b'}' | b']' | b' ' | b'\t' | b'\n' | b'\r')
                ) {
                    self.position += 1;
                }
                (self.position > start).then_some(())
            }
        }
    }

    fn skip_nested(&mut self) -> Option<()> {
        let mut depth = 0usize;
        loop {
            match self.take()? {
                b'"' => {
                    self.position -= 1;
                    self.string()?;
                }
                b'{' | b'[' => depth += 1,
                b'}' | b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(());
                    }
                }
                _ => (),
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        self.expect(b'"')?;
        let mut value = String::new();
        loop {
            match self.take()? {
                b'"' => return Some(value),
                b'\\' => value.push(self.escape()?),
                byte if byte < 0x20 => return None,
                byte if byte < 0x80 => value.push(byte as char),
                byte => {
                    // Copy the rest of the UTF-8 sequence the source already
                    // holds; the payload was validated as UTF-8 before parsing.
                    let start = self.position - 1;
                    let length = match byte {
                        0xc0..=0xdf => 2,
                        0xe0..=0xef => 3,
                        _ => 4,
                    };
                    let end = start.checked_add(length)?;
                    value.push_str(std::str::from_utf8(self.bytes.get(start..end)?).ok()?);
                    self.position = end;
                }
            }
        }
    }

    fn escape(&mut self) -> Option<char> {
        Some(match self.take()? {
            b'"' => '"',
            b'\\' => '\\',
            b'/' => '/',
            b'b' => '\u{8}',
            b'f' => '\u{c}',
            b'n' => '\n',
            b'r' => '\r',
            b't' => '\t',
            b'u' => return self.unicode_escape(),
            _ => return None,
        })
    }

    fn unicode_escape(&mut self) -> Option<char> {
        let high = self.hex4()?;
        if !(0xd800..0xdc00).contains(&high) {
            return char::from_u32(u32::from(high));
        }
        self.expect(b'\\')?;
        self.expect(b'u')?;
        let low = self.hex4()?;
        if !(0xdc00..0xe000).contains(&low) {
            return None;
        }
        let code = 0x10000 + ((u32::from(high) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
        char::from_u32(code)
    }

    fn hex4(&mut self) -> Option<u16> {
        let mut value = 0u16;
        for _ in 0..4 {
            let digit = char::from(self.take()?).to_digit(16)?;
            value = (value << 4) | digit as u16;
        }
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(json: &str) -> Vec<(String, String)> {
        string_fields(json).expect("valid JSON")
    }

    #[test]
    fn device_hello() {
        let parsed = fields(r#"{"name":"Erik's iPhone","version":"2.7.0"}"#);
        assert_eq!(field(&parsed, "name"), "Erik's iPhone");
        assert_eq!(field(&parsed, "version"), "2.7.0");
        assert_eq!(field(&parsed, "missing"), "");
    }

    #[test]
    fn whitespace_and_other_types() {
        let parsed = fields(r#" { "a" : 1 , "b" : true , "c" : null , "d" : "x" } "#);
        assert_eq!(parsed, [(String::from("d"), String::from("x"))]);
    }

    #[test]
    fn nested_values_are_skipped() {
        let parsed = fields(r#"{"nested":{"name":"inner"},"list":[1,{"a":"}"}],"name":"outer"}"#);
        assert_eq!(field(&parsed, "name"), "outer");
    }

    #[test]
    fn escapes() {
        let parsed = fields(r#"{"a":"\" \\ \/ \b \f \n \r \t å 😀"}"#);
        assert_eq!(field(&parsed, "a"), "\" \\ / \u{8} \u{c} \n \r \t å 😀");
    }

    #[test]
    fn non_ascii_is_kept() {
        assert_eq!(field(&fields(r#"{"a":"Ärik 😀"}"#), "a"), "Ärik 😀");
    }

    #[test]
    fn empty_object() {
        assert!(fields("{}").is_empty());
    }

    #[test]
    fn malformed() {
        for json in [
            "",
            "{",
            "{\"a\"}",
            "{\"a\":}",
            "{\"a\":\"b\"",
            "{\"a\":\"b\"}}",
            "[]",
            "null",
        ] {
            assert!(string_fields(json).is_none(), "{json} parsed");
        }
    }
}
