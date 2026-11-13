//! One current source document. Native items are parsed for their exact byte
//! boundaries and bound to code supplied by the build, never interpreted or
//! silently discarded by runtime injection.
use super::{Module, Source, error};
use crate::Error;
use std::borrow::Cow;

pub const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_COUNT: usize = 10_000;
const MAX_DEPTH: usize = 128;
const MAX_NODES: usize = 65_536;
pub(super) fn space_byte(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\r' | b'\n' | 0x0b | 0x0c)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeItemSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug)]
pub struct SourceDocument {
    pub source: Source,
    pub checked_text: String,
    pub native_spans: Vec<NativeItemSpan>,
    pub native_digest: String,
}

impl SourceDocument {
    pub fn parse(source: &str) -> Result<Self, Error> {
        super::agent::parse_unified(source)
    }

    pub fn compile(&self) -> Result<Module, Error> {
        Module::compile_unified(&self.source)
    }

    pub fn has_native_items(&self) -> bool {
        !self.native_spans.is_empty()
    }
}

pub(super) fn is_native_tag(tag: &str) -> bool {
    matches!(
        tag,
        "rust"
            | "native-use"
            | "native-fn"
            | "native-pubfn"
            | "native-asyncfn"
            | "native-pubasyncfn"
    )
}

pub(super) fn skip_native(text: &str, start: usize, nodes: &mut usize) -> Result<usize, Error> {
    let mut parser = NativeCursor {
        text,
        pos: start,
        nodes,
        depth: 0,
    };
    let tag = parser.token()?;
    match tag.as_ref() {
        "rust" => parser.raw()?,
        "native-use" => {
            parser.token()?;
        }
        "native-fn" | "native-pubfn" | "native-asyncfn" | "native-pubasyncfn" => {
            parser.token()?; // The native compiler validates names and types.
            let count = parser.count(MAX_COUNT)?;
            for _ in 0..count {
                parser.token()?;
                parser.token()?;
            }
            parser.token()?;
            parser.body()?;
        }
        _ => return Err(error("unknown native declaration")),
    }
    Ok(parser.pos)
}

struct NativeCursor<'a, 'n> {
    text: &'a str,
    pos: usize,
    nodes: &'n mut usize,
    depth: usize,
}

impl<'a> NativeCursor<'a, '_> {
    fn space(&mut self) {
        while self
            .text
            .as_bytes()
            .get(self.pos)
            .is_some_and(|b| space_byte(*b))
        {
            self.pos += 1;
        }
    }

    fn token(&mut self) -> Result<Cow<'a, str>, Error> {
        self.space();
        let start = self.pos;
        let bytes = self.text.as_bytes();
        if start == bytes.len() {
            return Err(error("unexpected end of native extension"));
        }
        if bytes[start] == b'"' {
            self.pos += 1;
            let mut escaped = false;
            let mut ended = false;
            while let Some(&byte) = bytes.get(self.pos) {
                self.pos += 1;
                if byte == b'"' && !escaped {
                    ended = true;
                    break;
                }
                escaped = byte == b'\\' && !escaped;
            }
            if !ended {
                return Err(error("unterminated native quoted token"));
            }
            if bytes.get(self.pos).is_some_and(|b| !space_byte(*b)) {
                return Err(error("native quoted token must end at whitespace"));
            }
            serde_json::from_str::<String>(&self.text[start..self.pos])
                .map(Cow::Owned)
                .map_err(|_| error("invalid native quoted token"))
        } else {
            while bytes.get(self.pos).is_some_and(|b| !space_byte(*b)) {
                self.pos += 1;
            }
            // The scanner does not retain or transform unquoted tokens. Keep
            // their exact source slice; JSON strings still require decoding.
            Ok(Cow::Borrowed(&self.text[start..self.pos]))
        }
    }

    fn count(&mut self, max: usize) -> Result<usize, Error> {
        let token = self.token()?;
        if token.is_empty()
            || !token.bytes().all(|b| b.is_ascii_digit())
            || token.len() > 8
            || (token.len() > 1 && token.starts_with('0'))
        {
            return Err(error("invalid native count"));
        }
        let value = token
            .parse::<usize>()
            .map_err(|_| error("native count overflow"))?;
        if value > max {
            return Err(error("native count exceeds limit"));
        }
        Ok(value)
    }

    fn raw(&mut self) -> Result<(), Error> {
        let size = self.count(MAX_SOURCE_BYTES)?;
        let bytes = self.text.as_bytes();
        if bytes.get(self.pos..self.pos + 2) == Some(b"\r\n") {
            self.pos += 2;
        } else if bytes.get(self.pos) == Some(&b'\n') {
            self.pos += 1;
        } else {
            return Err(error(
                "rust byte count must be immediately followed by newline",
            ));
        }
        let end = self
            .pos
            .checked_add(size)
            .ok_or_else(|| error("native length overflow"))?;
        if end > bytes.len() || !self.text.is_char_boundary(end) {
            return Err(error("truncated or split UTF-8 native fragment"));
        }
        self.pos = end;
        Ok(())
    }

    fn enter(&mut self) -> Result<(), Error> {
        *self.nodes += 1;
        self.depth += 1;
        if *self.nodes > MAX_NODES || self.depth > MAX_DEPTH {
            return Err(error("native syntax node/depth limit exceeded"));
        }
        Ok(())
    }

    fn expr(&mut self) -> Result<(), Error> {
        self.enter()?;
        let result = self.expr_inner();
        self.depth -= 1;
        result
    }

    fn expr_inner(&mut self) -> Result<(), Error> {
        let tag = self.token()?;
        match tag.as_ref() {
            "rust" => self.raw()?,
            "path" | "int" | "str" | "bool" => {
                self.token()?;
            }
            "unit" => {}
            "call" | "macro" => {
                self.token()?;
                let count = self.count(MAX_COUNT)?;
                for _ in 0..count {
                    self.expr()?;
                }
            }
            "method" => {
                self.expr()?;
                self.token()?;
                let count = self.count(MAX_COUNT)?;
                for _ in 0..count {
                    self.expr()?;
                }
            }
            "ref" | "mutref" | "deref" | "not" | "neg" | "try" | "await" | "unsafe" | "async"
            | "async-move" => self.expr()?,
            "cast" | "field" => {
                self.expr()?;
                self.token()?;
            }
            "index" => {
                self.expr()?;
                self.expr()?;
            }
            "array" | "tuple" => {
                let count = self.count(MAX_COUNT)?;
                for _ in 0..count {
                    self.expr()?;
                }
            }
            "bin" => {
                self.token()?;
                self.expr()?;
                self.expr()?;
            }
            "if" => {
                self.expr()?;
                self.expr()?;
                self.expr()?;
            }
            "block" => {
                self.body()?;
                self.expr()?;
            }
            "move" | "closure" => {
                let count = self.count(MAX_COUNT)?;
                for _ in 0..count {
                    self.token()?;
                }
                self.expr()?;
            }
            _ => return Err(error(format!("unknown native expression {tag}"))),
        }
        Ok(())
    }

    fn statement(&mut self) -> Result<(), Error> {
        self.enter()?;
        let result = self.statement_inner();
        self.depth -= 1;
        result
    }

    fn statement_inner(&mut self) -> Result<(), Error> {
        let tag = self.token()?;
        match tag.as_ref() {
            "rust" => self.raw()?,
            "let" => {
                self.token()?;
                self.expr()?;
            }
            "var" | "let-type" => {
                self.token()?;
                self.token()?;
                self.expr()?;
            }
            "set" => {
                self.expr()?;
                self.expr()?;
            }
            "do" | "return" | "tail" => self.expr()?,
            "break" | "continue" => {}
            "for" => {
                self.token()?;
                self.expr()?;
                self.body()?;
            }
            "while" => {
                self.expr()?;
                self.body()?;
            }
            "if-stmt" => {
                self.expr()?;
                self.body()?;
                self.body()?;
            }
            _ => return Err(error(format!("unknown native statement {tag}"))),
        }
        Ok(())
    }

    fn body(&mut self) -> Result<(), Error> {
        let count = self.count(MAX_COUNT)?;
        for _ in 0..count {
            self.statement()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_tokens_borrow_unquoted_bytes_and_decode_quoted_json() {
        let text = " \tplain\r\n42\u{0b}path::name\u{0c}β \"literal\" \"line\\n\\u03b2\"";
        let mut nodes = 0;
        let mut cursor = NativeCursor {
            text,
            pos: 0,
            nodes: &mut nodes,
            depth: 0,
        };
        for (expected, borrowed) in [
            ("plain", true),
            ("42", true),
            ("path::name", true),
            ("β", true),
            ("literal", false),
            ("line\nβ", false),
        ] {
            let token = cursor.token().unwrap();
            assert_eq!(token.as_ref(), expected);
            assert_eq!(matches!(token, Cow::Borrowed(_)), borrowed);
        }
        assert_eq!(cursor.pos, text.len());
        assert_eq!(cursor.depth, 0);
        assert_eq!(*cursor.nodes, 0);
        assert_eq!(
            cursor.token().unwrap_err().to_string(),
            "unexpected end of native extension"
        );
    }

    #[test]
    fn native_quoted_token_errors_and_count_rules_keep_their_diagnostics() {
        for (text, message) in [
            (r#""unfinished"#, "unterminated native quoted token"),
            (r#""x"tail"#, "native quoted token must end at whitespace"),
            (r#""\q""#, "invalid native quoted token"),
            (r#""\ud800""#, "invalid native quoted token"),
        ] {
            let mut nodes = 0;
            let mut cursor = NativeCursor {
                text,
                pos: 0,
                nodes: &mut nodes,
                depth: 0,
            };
            assert_eq!(cursor.token().unwrap_err().to_string(), message);
        }
        for text in ["3", r#""3""#] {
            let mut nodes = 0;
            let mut cursor = NativeCursor {
                text,
                pos: 0,
                nodes: &mut nodes,
                depth: 0,
            };
            assert_eq!(cursor.count(3).unwrap(), 3);
        }
        for (text, message) in [
            ("04", "invalid native count"),
            (r#""04""#, "invalid native count"),
            ("-1", "invalid native count"),
            ("999999999", "invalid native count"),
            ("4", "native count exceeds limit"),
            (r#""4""#, "native count exceeds limit"),
        ] {
            let mut nodes = 0;
            let mut cursor = NativeCursor {
                text,
                pos: 0,
                nodes: &mut nodes,
                depth: 0,
            };
            assert_eq!(cursor.count(3).unwrap_err().to_string(), message);
        }
    }

    #[test]
    fn native_quoted_and_unquoted_syntax_preserve_node_charges_and_boundaries() {
        let unquoted = "native-fn f 1 x i64 i64 2 let y path x tail bin + path y int 1";
        let quoted = r#"native-fn "f" "1" "x" "i64" "i64" "2" "let" "y" "path" "x" "tail" "bin" "+" "path" "y" "int" "1""#;
        let mut charges = Vec::new();
        for text in [unquoted, quoted] {
            let mut nodes = 0;
            assert_eq!(skip_native(text, 0, &mut nodes).unwrap(), text.len());
            charges.push(nodes);
        }
        assert_eq!(charges[0], charges[1]);
        for text in [unquoted, quoted] {
            let mut nodes = MAX_NODES;
            assert_eq!(
                skip_native(text, 0, &mut nodes).unwrap_err().to_string(),
                "native syntax node/depth limit exceeded"
            );
        }
        let nested = format!("native-fn f 0 unit 1 tail {}unit", "not ".repeat(MAX_DEPTH));
        let mut nodes = 0;
        assert_eq!(
            skip_native(&nested, 0, &mut nodes).unwrap_err().to_string(),
            "native syntax node/depth limit exceeded"
        );
    }
}
