use crate::{Buffer, Error, MAX_BUFFER_BYTES, MAX_EXPRESSION_BYTES};
use regex::{NoExpand, Regex};

#[derive(Clone, Debug)]
pub struct Expr(pub Vec<Op>);

#[derive(Clone, Debug)]
pub enum Op {
    Replace(Selector, String),
    Delete(Selector),
    Insert(InsertAddress, String),
    Trim,
    Lines(usize, usize),
    Filter(Selector),
    Map(Expr),
}

#[derive(Clone, Debug)]
pub enum Selector {
    Regex(Regex),
    Text(String),
    Lines(usize, usize),
    Range(usize, usize, usize, usize),
}

#[derive(Clone, Debug)]
pub enum InsertAddress {
    Line(usize),
    First,
    Last,
}

pub fn parse(source: &str) -> Result<Expr, Error> {
    if source.len() > MAX_EXPRESSION_BYTES {
        return Err("Expression exceeds 16 KiB".into());
    }
    let mut parser = Parser { source, pos: 0 };
    let expr = parser.pipeline(0)?;
    parser.space();
    if parser.pos != source.len() {
        return parser.fail("Unexpected input");
    }
    Ok(expr)
}

struct Parser<'a> {
    source: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn fail<T>(&self, message: &str) -> Result<T, Error> {
        Err(format!("{message} at byte {}", self.pos + 1))
    }
    fn space(&mut self) {
        while self.peek().map_or(false, char::is_whitespace) {
            self.bump();
        }
    }
    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }
    fn bump(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        Some(ch)
    }
    fn take(&mut self, token: &str) -> bool {
        self.space();
        if self.source[self.pos..].starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }
    fn expect(&mut self, token: &str) -> Result<(), Error> {
        if self.take(token) {
            Ok(())
        } else {
            self.fail(&format!("Expected {token}"))
        }
    }
    fn number(&mut self) -> Result<usize, Error> {
        self.space();
        let start = self.pos;
        while self.peek().map_or(false, |c| c.is_ascii_digit()) {
            self.bump();
        }
        let n = self.source[start..self.pos]
            .parse::<usize>()
            .map_err(|_| format!("Expected positive integer at byte {}", start + 1))?;
        if n == 0 {
            return self.fail("Addresses start at 1");
        }
        Ok(n)
    }
    fn insert_address(&mut self) -> Result<InsertAddress, Error> {
        self.space();
        if self.source[self.pos..].starts_with("gg") {
            self.pos += 2;
            Ok(InsertAddress::First)
        } else if self.source[self.pos..].starts_with('G') {
            self.pos += 1;
            Ok(InsertAddress::Last)
        } else {
            self.number().map(InsertAddress::Line)
        }
    }
    fn string(&mut self) -> Result<String, Error> {
        self.expect("\"")?;
        let mut out = String::new();
        loop {
            match self.bump() {
                Some('"') => return Ok(out),
                Some('\\') => out.push(match self.bump() {
                    Some('n') => '\n',
                    Some('r') => '\r',
                    Some('t') => '\t',
                    Some('"') => '"',
                    Some('\\') => '\\',
                    _ => return self.fail("Unknown string escape"),
                }),
                Some(c) => out.push(c),
                None => return self.fail("Unclosed string"),
            }
        }
    }
    fn regex(&mut self) -> Result<Regex, Error> {
        self.expect("/")?;
        let mut pattern = String::new();
        loop {
            match self.bump() {
                Some('/') => break,
                Some('\\') => match self.bump() {
                    Some('/') => pattern.push('/'),
                    Some(c) => {
                        pattern.push('\\');
                        pattern.push(c);
                    }
                    None => return self.fail("Unclosed regex"),
                },
                Some(c) => pattern.push(c),
                None => return self.fail("Unclosed regex"),
            }
        }
        Regex::new(&pattern).map_err(|e| format!("Invalid regex: {e}"))
    }
    fn selector(&mut self) -> Result<Selector, Error> {
        self.space();
        match self.peek() {
            Some('/') => Ok(Selector::Regex(self.regex()?)),
            Some('"') => Ok(Selector::Text(self.string()?)),
            Some(c) if c.is_ascii_digit() => {
                let sl = self.number()?;
                self.expect(":")?;
                let sc = self.number()?;
                self.expect("-")?;
                let el = self.number()?;
                self.expect(":")?;
                let ec = self.number()?;
                if sl == 0 || sc == 0 || el == 0 || ec == 0 {
                    return self.fail("Line and column addresses start at 1");
                }
                if (el, ec) < (sl, sc) {
                    return self.fail("Range end precedes start");
                }
                Ok(Selector::Range(sl, sc, el, ec))
            }
            Some(_) => {
                let start = self.pos;
                while self.peek().map_or(false, |c| c.is_ascii_alphabetic()) {
                    self.bump();
                }
                if &self.source[start..self.pos] != "lines" {
                    return self.fail("Expected selector");
                }
                self.expect("(")?;
                let a = self.number()?;
                self.expect("..")?;
                let b = self.number()?;
                if a > b {
                    return self.fail("Range start exceeds end");
                }
                self.expect(")")?;
                Ok(Selector::Lines(a, b))
            }
            None => self.fail("Expected selector"),
        }
    }
    fn pipeline(&mut self, depth: usize) -> Result<Expr, Error> {
        if depth > 16 {
            return self.fail("Map nesting exceeds 16");
        }
        self.space();
        if self.pos == self.source.len() && depth == 0 {
            return Ok(Expr(vec![]));
        }
        let mut ops = vec![self.operation(depth)?];
        while self.take("|>") {
            ops.push(self.operation(depth)?);
        }
        Ok(Expr(ops))
    }
    fn operation(&mut self, depth: usize) -> Result<Op, Error> {
        self.space();
        let start = self.pos;
        while self.peek().map_or(false, |c| c.is_ascii_alphabetic()) {
            self.bump();
        }
        let name = &self.source[start..self.pos];
        self.expect("(")?;
        let op = match name {
            "replace" => {
                let selector = self.selector()?;
                self.expect(",")?;
                Op::Replace(selector, self.string()?)
            }
            "delete" => Op::Delete(self.match_selector()?),
            "insert" => {
                let address = self.insert_address()?;
                self.expect(",")?;
                Op::Insert(address, self.string()?)
            }
            "trim" => Op::Trim,
            "lines" => {
                let a = self.number()?;
                self.expect("..")?;
                let b = self.number()?;
                if a > b {
                    return self.fail("Range start exceeds end");
                }
                Op::Lines(a, b)
            }
            "filter" => Op::Filter(self.match_selector()?),
            "map" => Op::Map(self.pipeline(depth + 1)?),
            _ => return self.fail("Unknown function"),
        };
        self.expect(")")?;
        Ok(op)
    }
    fn match_selector(&mut self) -> Result<Selector, Error> {
        match self.selector()? {
            s @ Selector::Regex(_) | s @ Selector::Text(_) => Ok(s),
            _ => self.fail("Expected regex or quoted text selector"),
        }
    }
}

/// All operations allocate a new value. No capabilities other than text are available.
pub fn eval(expr: &Expr, input: &str) -> Result<Buffer, Error> {
    eval_at(expr, input, 0)
}

fn eval_at(expr: &Expr, input: &str, depth: usize) -> Result<Buffer, Error> {
    if depth > 16 {
        return Err("Map nesting exceeds 16".into());
    }
    check_size(input)?;
    let mut buffer = input.to_owned();
    for op in &expr.0 {
        buffer = match op {
            Op::Replace(selector, text) => replace_selector(selector, &buffer, text)?,
            Op::Delete(selector) => replace_selector(selector, &buffer, "")?,
            Op::Trim => buffer.trim().to_owned(),
            Op::Lines(a, b) => {
                if *a == 0 || a > b {
                    return Err("Invalid line range".into());
                }
                buffer
                    .split_inclusive('\n')
                    .enumerate()
                    .filter(|(i, _)| i + 1 >= *a && i + 1 <= *b)
                    .map(|(_, s)| s)
                    .collect()
            }
            Op::Filter(selector) => buffer
                .split_inclusive('\n')
                .filter(|s| matches_selector(selector, line_parts(s).0))
                .collect(),
            Op::Insert(address, text) => {
                let chunks: Vec<_> = buffer.split_inclusive('\n').collect();
                let line = match address {
                    InsertAddress::Line(n) if *n > 0 && *n <= chunks.len() + 1 => *n,
                    InsertAddress::Line(_) => {
                        return Err("Insert address outside 1..line count + 1".into())
                    }
                    InsertAddress::First => 1,
                    InsertAddress::Last => chunks.len().max(1),
                };
                if buffer.len().saturating_add(text.len()) > MAX_BUFFER_BYTES {
                    return Err("Buffer exceeds 8 MiB".into());
                }
                let offset: usize = chunks.iter().take(line - 1).map(|s| s.len()).sum();
                format!("{}{}{}", &buffer[..offset], text, &buffer[offset..])
            }
            Op::Map(inner) => {
                let mut result = String::new();
                for line in buffer.split_inclusive('\n') {
                    let (content, ending) = line_parts(line);
                    let mapped = eval_at(inner, content, depth + 1)?;
                    if result
                        .len()
                        .saturating_add(mapped.len())
                        .saturating_add(ending.len())
                        > MAX_BUFFER_BYTES
                    {
                        return Err("Buffer exceeds 8 MiB".into());
                    }
                    result.push_str(&mapped);
                    result.push_str(ending);
                }
                result
            }
        };
        check_size(&buffer)?;
    }
    Ok(buffer)
}

fn line_parts(line: &str) -> (&str, &str) {
    if let Some(content) = line.strip_suffix("\r\n") {
        (content, "\r\n")
    } else if let Some(content) = line.strip_suffix('\n') {
        (content, "\n")
    } else {
        (line, "")
    }
}

fn check_size(s: &str) -> Result<(), Error> {
    if s.len() > MAX_BUFFER_BYTES {
        Err("Buffer exceeds 8 MiB".into())
    } else {
        Ok(())
    }
}

fn replace(re: &Regex, input: &str, replacement: &str) -> Result<String, Error> {
    // Preflight output size before allocation, including zero-length matches.
    let mut size = input.len();
    for m in re.find_iter(input) {
        size = size
            .checked_sub(m.len())
            .and_then(|n| n.checked_add(replacement.len()))
            .ok_or_else(|| "Buffer size overflow".to_string())?;
        if size > MAX_BUFFER_BYTES {
            return Err("Buffer exceeds 8 MiB".into());
        }
    }
    Ok(re.replace_all(input, NoExpand(replacement)).into_owned())
}

fn replace_selector(selector: &Selector, input: &str, replacement: &str) -> Result<String, Error> {
    match selector {
        Selector::Regex(re) => replace(re, input, replacement),
        Selector::Text(text) => replace_text(text, input, replacement),
        Selector::Lines(a, b) => {
            let spans = line_spans(input, *a, *b);
            if spans.len() != b - a + 1 {
                return Err("Line range outside buffer".into());
            }
            let (start, _) = spans[0];
            let (_, end) = spans[spans.len() - 1];
            replace_spans(input, &[(start, end)], replacement)
        }
        Selector::Range(sl, sc, el, ec) => {
            let starts = line_spans(input, *sl, *sl);
            let ends = line_spans(input, *el, *el);
            let Some((line_start, line_end)) = starts.first().copied() else {
                return Err("Range line outside buffer".into());
            };
            let Some((end_start, end_end)) = ends.first().copied() else {
                return Err("Range line outside buffer".into());
            };
            let start_len = line_parts(&input[line_start..line_end]).0.chars().count();
            let end_len = line_parts(&input[end_start..end_end]).0.chars().count();
            if *sc > start_len || *ec > end_len {
                return Err("Range column outside line".into());
            }
            let start = char_offset(input, line_start, line_end, *sc - 1);
            let end = char_offset(input, end_start, end_end, *ec);
            if start > end || end > input.len() {
                return Err("Range outside buffer".into());
            }
            replace_spans(input, &[(start, end)], replacement)
        }
    }
}

fn matches_selector(selector: &Selector, value: &str) -> bool {
    match selector {
        Selector::Regex(re) => re.is_match(value),
        Selector::Text(text) => value.contains(text),
        _ => false,
    }
}

fn replace_text(text: &str, input: &str, replacement: &str) -> Result<String, Error> {
    if text.is_empty() {
        return Ok(input.to_owned());
    }
    let spans: Vec<_> = input
        .match_indices(text)
        .map(|(i, s)| (i, i + s.len()))
        .collect();
    replace_spans(input, &spans, replacement)
}

fn line_spans(input: &str, first: usize, last: usize) -> Vec<(usize, usize)> {
    let mut offset = 0;
    input
        .split_inclusive('\n')
        .enumerate()
        .filter_map(|(i, line)| {
            let start = offset;
            offset += line.len();
            ((i + 1 >= first) && (i + 1 <= last)).then_some((start, offset))
        })
        .collect()
}

fn char_offset(input: &str, start: usize, end: usize, chars: usize) -> usize {
    let segment = &input[start..end];
    let content = line_parts(segment).0;
    content
        .char_indices()
        .nth(chars)
        .map(|(i, _)| start + i)
        .unwrap_or(start + content.len())
}

fn replace_spans(
    input: &str,
    spans: &[(usize, usize)],
    replacement: &str,
) -> Result<String, Error> {
    let removed: usize = spans.iter().map(|(a, b)| b - a).sum();
    let size = input
        .len()
        .saturating_sub(removed)
        .saturating_add(spans.len().saturating_mul(replacement.len()));
    if size > MAX_BUFFER_BYTES {
        return Err("Buffer exceeds 8 MiB".into());
    }
    let mut out = String::with_capacity(size);
    let mut pos = 0;
    for &(start, end) in spans {
        if start < pos || end > input.len() {
            continue;
        }
        out.push_str(&input[pos..start]);
        out.push_str(replacement);
        pos = end;
    }
    out.push_str(&input[pos..]);
    Ok(out)
}
