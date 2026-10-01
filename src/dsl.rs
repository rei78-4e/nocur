use crate::{Buffer, Error, MAX_BUFFER_BYTES, MAX_EXPRESSION_BYTES};
use regex::{NoExpand, Regex};

#[derive(Clone, Debug)]
pub struct Expr(pub Vec<Op>);

#[derive(Clone, Debug)]
pub enum Op {
    Replace(Regex, String),
    Delete(Regex),
    Insert(InsertAddress, String),
    Trim,
    Lines(usize, usize),
    Filter(Regex),
    Map(Expr),
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
                let re = self.regex()?;
                self.expect(",")?;
                Op::Replace(re, self.string()?)
            }
            "delete" => Op::Delete(self.regex()?),
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
            "filter" => Op::Filter(self.regex()?),
            "map" => Op::Map(self.pipeline(depth + 1)?),
            _ => return self.fail("Unknown function"),
        };
        self.expect(")")?;
        Ok(op)
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
            Op::Replace(re, text) => replace(re, &buffer, text)?,
            Op::Delete(re) => replace(re, &buffer, "")?,
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
            Op::Filter(re) => buffer
                .split_inclusive('\n')
                .filter(|s| re.is_match(line_parts(s).0))
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
