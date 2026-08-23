//! Just enough XML property list support to talk to usbmuxd.
//!
//! It reads the handful of value kinds usbmuxd replies with and writes flat
//! dictionaries of strings and integers, which is all its requests contain.

/// How deep a reply may nest before it is rejected. usbmuxd's own replies are
/// three or four levels deep; the limit is here so that a malformed one cannot
/// run the parser out of stack.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Dict(Vec<(String, Value)>),
    Array(Vec<Value>),
    String(String),
    Integer(i64),
    Bool(bool),
    /// A tag this parser has no interest in, kept so that positions still line
    /// up inside a dictionary or an array.
    Other(String),
}

impl Value {
    /// Looks one key up in a dictionary. Anything that is not a dictionary has
    /// no keys, which keeps callers from having to check the kind first.
    pub fn get(&self, key: &str) -> Option<&Value> {
        let Value::Dict(entries) = self else {
            return None;
        };

        entries.iter().find(|(name, _)| name == key).map(|(_, value)| value)
    }

    pub fn get_string(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(Value::String(value)) => Some(value),
            _ => None,
        }
    }

    pub fn get_integer(&self, key: &str) -> Option<i64> {
        match self.get(key) {
            Some(Value::Integer(value)) => Some(*value),
            _ => None,
        }
    }

    pub fn array(&self) -> &[Value] {
        match self {
            Value::Array(values) => values,
            _ => &[],
        }
    }
}

struct Parser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    fn rest(&self) -> &'a [u8] {
        &self.input[self.pos..]
    }

    fn peek(&self) -> Option<u8> {
        self.input.get(self.pos).copied()
    }

    fn looking_at(&self, text: &str) -> bool {
        self.rest().starts_with(text.as_bytes())
    }

    fn skip_space(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    /// Moves past the next occurrence of `text`, or to the end if there is none.
    fn skip_until(&mut self, text: &str) {
        match self
            .rest()
            .windows(text.len())
            .position(|window| window == text.as_bytes())
        {
            Some(offset) => self.pos += offset + text.len(),
            None => self.pos = self.input.len(),
        }
    }

    /// Skips the XML declaration, the DOCTYPE and any comments in front of a value.
    fn skip_prolog(&mut self) {
        loop {
            self.skip_space();

            if self.looking_at("<?") {
                self.skip_until("?>");
            } else if self.looking_at("<!--") {
                self.skip_until("-->");
            } else if self.looking_at("<!") {
                self.skip_until(">");
            } else {
                return;
            }
        }
    }

    /// Reads the name of the tag the parser is positioned on, without consuming it.
    fn peek_tag(&self) -> Option<String> {
        let mut pos = self.pos;

        if self.input.get(pos) != Some(&b'<') {
            return None;
        }

        pos += 1;

        if self.input.get(pos) == Some(&b'/') {
            pos += 1;
        }

        let start = pos;

        while let Some(byte) = self.input.get(pos) {
            if matches!(byte, b'>' | b'/' | b' ' | b'\t' | b'\r' | b'\n') {
                break;
            }

            pos += 1;
        }

        (pos > start).then(|| String::from_utf8_lossy(&self.input[start..pos]).into_owned())
    }

    /// Consumes the tag the parser is positioned on, reporting whether it
    /// carried its own end, as `<true/>` does.
    fn consume_tag(&mut self) -> Option<bool> {
        if self.peek() != Some(b'<') {
            return None;
        }

        let mut self_closing = false;

        while let Some(byte) = self.peek() {
            if byte == b'>' {
                self.pos += 1;
                return Some(self_closing);
            }

            self_closing = byte == b'/';
            self.pos += 1;
        }

        None
    }

    /// Reads the text up to the next tag, resolving the entities XML requires.
    ///
    /// Bytes are gathered before they are decoded, so that a device name in
    /// UTF-8 survives rather than being read one byte at a time.
    fn parse_text(&mut self) -> String {
        let mut text: Vec<u8> = Vec::new();

        while let Some(byte) = self.peek() {
            if byte == b'<' {
                break;
            }

            if byte != b'&' {
                text.push(byte);
                self.pos += 1;
                continue;
            }

            let name = &self.input[self.pos + 1..];
            let Some(end) = name.iter().position(|byte| *byte == b';') else {
                text.push(b'&');
                self.pos += 1;
                continue;
            };

            match &name[..end] {
                b"amp" => text.push(b'&'),
                b"lt" => text.push(b'<'),
                b"gt" => text.push(b'>'),
                b"quot" => text.push(b'"'),
                b"apos" => text.push(b'\''),
                // Anything else is left as it was written.
                other => {
                    text.push(b'&');
                    text.extend_from_slice(other);
                    text.push(b';');
                }
            }

            self.pos += 1 + end + 1;
        }

        String::from_utf8_lossy(&text).into_owned()
    }

    fn parse_dict(&mut self, depth: usize) -> Option<Value> {
        let mut entries = Vec::new();

        loop {
            self.skip_space();

            let name = self.peek_tag()?;

            if self.looking_at("</") {
                self.consume_tag()?;
                return Some(Value::Dict(entries));
            }

            if name != "key" {
                return None;
            }

            self.consume_tag()?;
            let key = self.parse_text();
            self.consume_tag()?;

            entries.push((key, self.parse_value(depth + 1)?));
        }
    }

    fn parse_array(&mut self, depth: usize) -> Option<Value> {
        let mut values = Vec::new();

        loop {
            self.skip_space();
            self.peek_tag()?;

            if self.looking_at("</") {
                self.consume_tag()?;
                return Some(Value::Array(values));
            }

            values.push(self.parse_value(depth + 1)?);
        }
    }

    fn parse_value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }

        self.skip_space();

        let name = self.peek_tag()?;
        let self_closing = self.consume_tag()?;

        if name == "true" || name == "false" {
            if !self_closing {
                self.consume_tag();
            }

            return Some(Value::Bool(name == "true"));
        }

        if name == "dict" || name == "array" {
            if self_closing {
                return Some(if name == "dict" {
                    Value::Dict(Vec::new())
                } else {
                    Value::Array(Vec::new())
                });
            }

            return if name == "dict" {
                self.parse_dict(depth)
            } else {
                self.parse_array(depth)
            };
        }

        if self_closing {
            return Some(match name.as_str() {
                "string" => Value::String(String::new()),
                "integer" => Value::Integer(0),
                _ => Value::Other(String::new()),
            });
        }

        let text = self.parse_text();

        self.consume_tag()?;

        Some(match name.as_str() {
            "string" => Value::String(text),
            // A number this cannot read is zero, the way strtoll() reported it.
            "integer" => Value::Integer(text.trim().parse().unwrap_or(0)),
            _ => Value::Other(text),
        })
    }
}

pub fn parse(xml: &[u8]) -> Option<Value> {
    let mut parser = Parser::new(xml);

    parser.skip_prolog();

    let name = parser.peek_tag()?;

    // The <plist> element is a wrapper around the one value we want.
    if name == "plist" && parser.consume_tag()? {
        return None;
    }

    parser.parse_value(0)
}

impl Default for Writer {
    fn default() -> Self {
        Self::new()
    }
}

/// Builds the flat dictionaries usbmuxd requests are made of.
pub struct Writer {
    out: String,
}

impl Writer {
    pub fn new() -> Self {
        Self {
            out: String::from(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
                 <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
                 \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
                 <plist version=\"1.0\"><dict>",
            ),
        }
    }

    pub fn string(&mut self, key: &str, value: &str) {
        self.out.push_str("<key>");
        self.escape(key);
        self.out.push_str("</key><string>");
        self.escape(value);
        self.out.push_str("</string>");
    }

    pub fn integer(&mut self, key: &str, value: i64) {
        self.out.push_str("<key>");
        self.escape(key);
        self.out.push_str("</key><integer>");
        self.out.push_str(&value.to_string());
        self.out.push_str("</integer>");
    }

    pub fn finish(mut self) -> String {
        self.out.push_str("</dict></plist>\n");
        self.out
    }

    fn escape(&mut self, value: &str) {
        for character in value.chars() {
            match character {
                '&' => self.out.push_str("&amp;"),
                '<' => self.out.push_str("&lt;"),
                '>' => self.out.push_str("&gt;"),
                _ => self.out.push(character),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const REPLY: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>DeviceList</key>
  <array>
    <dict>
      <key>DeviceID</key><integer>7</integer>
      <key>Properties</key>
      <dict>
        <key>ConnectionType</key><string>USB</string>
        <key>SerialNumber</key><string>00008030-001</string>
      </dict>
    </dict>
  </array>
</dict></plist>"#;

    #[test]
    fn parses_a_device_list() {
        let reply = parse(REPLY.as_bytes()).expect("a well formed reply parses");
        let devices = reply.get("DeviceList").expect("the list is there");

        assert_eq!(devices.array().len(), 1);

        let device = &devices.array()[0];

        assert_eq!(device.get_integer("DeviceID"), Some(7));

        let properties = device.get("Properties").expect("properties are there");

        assert_eq!(properties.get_string("ConnectionType"), Some("USB"));
        assert_eq!(properties.get_string("SerialNumber"), Some("00008030-001"));
    }

    #[test]
    fn reads_the_number_a_connect_replies_with() {
        let reply = parse(b"<plist version=\"1.0\"><dict><key>Number</key><integer>0</integer></dict></plist>")
            .expect("parses");

        assert_eq!(reply.get_integer("Number"), Some(0));
    }

    #[test]
    fn resolves_entities() {
        let reply = parse(b"<dict><key>a</key><string>x &amp; y &lt;z&gt; &quot;q&quot; &apos;p&apos;</string></dict>")
            .expect("parses");

        assert_eq!(reply.get_string("a"), Some("x & y <z> \"q\" 'p'"));
    }

    #[test]
    fn keeps_unknown_entities_as_written() {
        let reply = parse(b"<dict><key>a</key><string>&nbsp;</string></dict>").expect("parses");

        assert_eq!(reply.get_string("a"), Some("&nbsp;"));
    }

    #[test]
    fn reads_self_closing_and_boolean_tags() {
        let reply =
            parse(b"<dict><key>t</key><true/><key>f</key><false/><key>e</key><string/></dict>").expect("parses");

        assert_eq!(reply.get("t"), Some(&Value::Bool(true)));
        assert_eq!(reply.get("f"), Some(&Value::Bool(false)));
        assert_eq!(reply.get_string("e"), Some(""));
    }

    #[test]
    fn a_missing_key_is_not_an_error() {
        let reply = parse(b"<dict><key>a</key><string>b</string></dict>").expect("parses");

        assert_eq!(reply.get_string("absent"), None);
        assert_eq!(reply.get_integer("a"), None, "a string is not an integer");
    }

    /// The daemon is local but its replies are still input, and the C parser
    /// recursed without a limit on them.
    #[test]
    fn deep_nesting_is_rejected_rather_than_overflowing_the_stack() {
        let depth = 10_000;
        let mut xml = String::new();

        for _ in 0..depth {
            xml.push_str("<array>");
        }

        for _ in 0..depth {
            xml.push_str("</array>");
        }

        assert_eq!(parse(xml.as_bytes()), None);
    }

    #[test]
    fn truncated_input_is_rejected_rather_than_looping() {
        assert_eq!(parse(b""), None);
        assert_eq!(parse(b"<dict>"), None);
        assert_eq!(parse(b"<dict><key>a</key>"), None);
        assert_eq!(parse(b"<dict><key>a</key><string>b"), None);
        assert_eq!(parse(b"<plist"), None);
    }

    /// The C parser truncated tag names at 32 bytes, so a long one could be
    /// mistaken for a name it merely started with.
    #[test]
    fn a_long_tag_name_is_not_truncated_into_another() {
        let name = "key".to_string() + &"x".repeat(64);
        let xml = format!("<dict><{name}>a</{name}></dict>");

        assert_eq!(parse(xml.as_bytes()), None, "it is not a <key>");
    }

    #[test]
    fn writes_a_request() {
        let mut writer = Writer::new();

        writer.string("MessageType", "Connect");
        writer.integer("DeviceID", 7);

        let request = writer.finish();

        assert!(request.contains("<key>MessageType</key><string>Connect</string>"));
        assert!(request.contains("<key>DeviceID</key><integer>7</integer>"));
        assert!(request.ends_with("</dict></plist>\n"));
    }

    #[test]
    fn escapes_what_it_writes() {
        let mut writer = Writer::new();

        writer.string("a", "x & y <z>");

        assert!(writer.finish().contains("<string>x &amp; y &lt;z&gt;</string>"));
    }

    #[test]
    fn what_it_writes_it_can_read_back() {
        let mut writer = Writer::new();

        writer.string("MessageType", "ListDevices");
        writer.integer("kLibUSBMuxVersion", 3);

        let request = writer.finish();
        let parsed = parse(request.as_bytes()).expect("its own output parses");

        assert_eq!(parsed.get_string("MessageType"), Some("ListDevices"));
        assert_eq!(parsed.get_integer("kLibUSBMuxVersion"), Some(3));
    }
}
