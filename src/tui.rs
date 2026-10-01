use crossterm::{
    cursor::{Hide, MoveTo, Show},
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind,
        KeyModifiers,
    },
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use nocur::{editor::Editor, MAX_EXPRESSION_BYTES};
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
};
use unicode_width::UnicodeWidthChar;

const TEXT: Color = Color::Rgb {
    r: 229,
    g: 235,
    b: 242,
};
const MUTED: Color = Color::Rgb {
    r: 155,
    g: 172,
    b: 192,
};
const ACCENT: Color = Color::Rgb {
    r: 104,
    g: 209,
    b: 224,
};
const SUCCESS: Color = Color::Rgb {
    r: 132,
    g: 218,
    b: 159,
};
const ERROR: Color = Color::Rgb {
    r: 255,
    g: 126,
    b: 139,
};
const WARNING: Color = Color::Rgb {
    r: 255,
    g: 202,
    b: 117,
};

struct Terminal;
impl Terminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        let guard = Self;
        execute!(
            io::stdout(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        )?;
        Ok(guard)
    }
}
impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = execute!(
            io::stdout(),
            Show,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

// Render controls as visible glyphs so buffer contents cannot issue terminal commands.
fn visual(ch: char) -> char {
    if ch == '\t' {
        '→'
    } else if ch.is_control() {
        '�'
    } else {
        ch
    }
}

fn rows(text: &str, width: usize, caret: usize) -> (Vec<String>, (usize, usize)) {
    let width = width.max(2);
    let mut lines = vec![String::new()];
    let (mut row, mut col) = (0, 0);
    let mut cursor = (0, 0);
    let mut wrapped = false;
    for (byte, ch) in text.char_indices() {
        let glyph = visual(ch);
        let size = glyph.width().unwrap_or(1);
        if ch != '\n' && col + size > width {
            lines.push(String::new());
            row += 1;
            col = 0;
        }
        if byte == caret {
            cursor = (row, col);
        }
        if ch == '\n' {
            if !wrapped {
                lines.push(String::new());
                row += 1;
                col = 0;
            }
            wrapped = false;
        } else {
            lines[row].push(glyph);
            col += size;
            wrapped = false;
            if col == width {
                lines.push(String::new());
                row += 1;
                col = 0;
                wrapped = true;
            }
        }
    }
    if caret == text.len() {
        cursor = (row, col);
    }
    (lines, cursor)
}

fn prompt_prefix(prompt: &str, width: usize) -> (String, usize) {
    let limit = width.saturating_sub(3);
    let mut shown = String::new();
    let mut used = 0;
    let mut truncated = false;
    for ch in prompt.chars() {
        let glyph = visual(ch);
        let size = glyph.width().unwrap_or(1);
        if used + size > limit {
            truncated = true;
            break;
        }
        shown.push(glyph);
        used += size;
    }
    if truncated {
        while used + 1 > limit {
            if let Some(ch) = shown.pop() {
                used -= ch.width().unwrap_or(1);
            }
        }
        shown.push('…');
        used += 1;
    }
    shown.push(' ');
    (shown, used + 1)
}

#[derive(Clone, Copy)]
struct Completion {
    name: &'static str,
}

const COMPLETIONS: [Completion; 7] = [
    Completion { name: "replace" },
    Completion { name: "delete" },
    Completion { name: "insert" },
    Completion { name: "trim" },
    Completion { name: "lines" },
    Completion { name: "filter" },
    Completion { name: "map" },
];

#[derive(Clone, Copy)]
struct CallFrame {
    name: Option<&'static str>,
    argument: usize,
    argument_start: usize,
}

fn call_context(source: &str, caret: usize) -> Option<CallFrame> {
    let bytes = &source.as_bytes()[..caret];
    let mut stack: Vec<CallFrame> = Vec::new();
    let (mut i, mut pending, mut mode) = (0, None, 0u8);
    while i < bytes.len() {
        let byte = bytes[i];
        if mode != 0 {
            match mode {
                1 if byte == b'\\' => i = (i + 2).min(bytes.len()),
                1 if byte == b'"' => {
                    mode = 0;
                    i += 1;
                }
                2 if byte == b'\\' => i = (i + 2).min(bytes.len()),
                2 if byte == b'/' => {
                    mode = 0;
                    i += 1;
                }
                _ => i += 1,
            }
            continue;
        }
        match byte {
            b'"' => {
                mode = 1;
                pending = None;
                i += 1;
            }
            b'/' => {
                mode = 2;
                pending = None;
                i += 1;
            }
            b'(' => {
                let name = pending.take();
                stack.push(CallFrame {
                    name,
                    argument: 0,
                    argument_start: i + 1,
                });
                i += 1;
            }
            b')' => {
                stack.pop();
                pending = None;
                i += 1;
            }
            b',' => {
                if let Some(frame) = stack.last_mut() {
                    frame.argument += 1;
                    frame.argument_start = i + 1;
                }
                pending = None;
                i += 1;
            }
            b if b.is_ascii_alphabetic() => {
                let start = i;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                pending = match &bytes[start..i] {
                    b"replace" => Some("replace"),
                    b"delete" => Some("delete"),
                    b"insert" => Some("insert"),
                    b"trim" => Some("trim"),
                    b"lines" => Some("lines"),
                    b"filter" => Some("filter"),
                    b"map" => Some("map"),
                    _ => None,
                };
            }
            b if b.is_ascii_whitespace() => i += 1,
            _ => {
                pending = None;
                i += 1;
            }
        }
    }
    stack.last().copied()
}

fn operation_context(source: &str, caret: usize) -> Option<(usize, usize, &str)> {
    let before = &source[..caret];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, ch)| !ch.is_ascii_alphabetic())
        .map_or(0, |(i, ch)| i + ch.len_utf8());
    let prefix = &before[start..];
    let end = source[caret..]
        .char_indices()
        .find(|(_, ch)| !ch.is_ascii_alphabetic())
        .map_or(source.len(), |(offset, _)| caret + offset);
    let before_token = before[..start].trim_end();
    let at_pipeline = before_token.ends_with("|>");
    let inside_map = call_context(before_token, before_token.len()).map_or(false, |frame| {
        if frame.name != Some("map") {
            return false;
        }
        let body = before_token[frame.argument_start..].trim_end();
        body.is_empty() || body.ends_with("|>")
    });
    if prefix.chars().all(|ch| ch.is_ascii_alphabetic())
        && source[start..end]
            .chars()
            .all(|ch| ch.is_ascii_alphabetic())
        && (before_token.is_empty() || at_pipeline || inside_map)
    {
        Some((start, end, prefix))
    } else {
        None
    }
}

fn matching_completions(prefix: &str) -> Vec<Completion> {
    COMPLETIONS
        .iter()
        .copied()
        .filter(|item| item.name.starts_with(prefix))
        .collect()
}

fn completion_line(matches: &[Completion], selected: usize) -> String {
    let mut line = String::from(" Functions: ");
    for (index, item) in matches.iter().enumerate() {
        if index > 0 {
            line.push_str(" · ");
        }
        if index == selected {
            line.push('[');
        }
        line.push_str(item.name);
        line.push_str("()");
        if index == selected {
            line.push(']');
        }
    }
    line.push_str("  ·  Tab complete  ↑↓ select");
    line
}

fn argument_hint(source: &str, caret: usize) -> Option<String> {
    let frame = call_context(source, caret)?;
    let hint = match (frame.name?, frame.argument) {
        ("replace", 0) => "replace · regex pattern, e.g. /foo/",
        ("replace", 1) => "replace · replacement text in quotes, e.g. \"bar\"",
        ("delete", 0) => "delete · regex pattern, e.g. /foo/",
        ("insert", 0) => "insert · line number, gg (first), or G (last)",
        ("insert", 1) => "insert · text to place before the selected line",
        ("trim", _) => "trim · takes no arguments",
        ("lines", 0) => {
            let current_arg = &source[frame.argument_start..caret];
            if current_arg.contains("..") {
                "lines · inclusive end line number"
            } else {
                "lines · inclusive range, e.g. 1..10"
            }
        }
        ("filter", 0) => "filter · regex matched against each line",
        ("map", _) => "map · pipeline to apply to each line",
        _ => return None,
    };
    Some(hint.into())
}

pub fn run(
    mut editor: Editor,
    output: PathBuf,
    prompt: String,
) -> Result<(), Box<dyn std::error::Error>> {
    let _terminal = Terminal::enter()?;
    let mut caret = 0;
    let mut scroll: usize = 0;
    let mut completion_index = 0usize;
    let mut history_open = false;
    let mut history_selection = 0usize;
    let mut status = String::from("Ready. Save writes committed buffer.");
    loop {
        let (width, height) = terminal::size()?;
        if width < 20 || height < 8 {
            execute!(
                io::stdout(),
                Hide,
                MoveTo(0, 0),
                Clear(ClearType::All),
                Print("Resize terminal")
            )?;
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('q') && key.modifiers.contains(KeyModifiers::CONTROL) {
                    return Ok(());
                }
            }
            continue;
        }
        let width = width.max(2);
        let upper = ((height - 4) / 2).max(2);
        let lower = (height - upper - 4) as usize;
        // Preview is a local derived value, never stored in Editor or history.
        let preview = editor.state.preview();
        let text = match &preview {
            Ok(s) => s.as_str(),
            Err(_) => editor.state.committed(),
        };
        let (preview_rows, _) = rows(text, width as usize, 0);
        scroll = scroll.min(
            preview_rows
                .len()
                .saturating_sub(upper.saturating_sub(1) as usize),
        );
        let (prefix, prefix_width) = prompt_prefix(&prompt, width as usize);
        let content_width = width as usize - prefix_width;
        let (draft_rows, (cy, cx)) = rows(editor.draft(), content_width, caret);
        let input_scroll = cy.saturating_sub(lower - 1);
        let completion_context = operation_context(editor.draft(), caret);
        let completions = completion_context
            .map(|(_, _, prefix)| matching_completions(prefix))
            .unwrap_or_default();
        let (head, total) = editor.position();
        let mut out = io::stdout();
        queue!(
            out,
            Hide,
            SetBackgroundColor(Color::Reset),
            MoveTo(0, 0),
            Clear(ClearType::All)
        )?;
        let header = if history_open {
            " History · ↑↓ move · Home/End jump · Enter select · Esc close".to_owned()
        } else {
            format!(
                " nocur  ·  Preview [{}]  ·  History {head}/{total}  ·  F2 commit{}",
                if preview.is_ok() { "valid" } else { "error" },
                if preview.is_ok() {
                    ""
                } else {
                    "  ·  committed buffer"
                },
            )
        };
        draw_styled(
            &mut out,
            0,
            &header,
            width,
            if preview.is_ok() { SUCCESS } else { ERROR },
            Color::Reset,
            ' ',
        )?;
        if history_open {
            let visible = upper.saturating_sub(1) as usize;
            let max_start = (total + 1).saturating_sub(visible);
            let start = history_selection.saturating_sub(visible / 2).min(max_start);
            for (row, position) in (start..=total).take(visible).enumerate() {
                let selected = position == history_selection;
                let active = position == head;
                let source = editor.history_source(position).unwrap_or("");
                let source = if position > 0 && source.trim().is_empty() {
                    "Identity"
                } else {
                    source
                };
                let source = source.replace('\n', " ").replace('\r', " ");
                let line = format!(
                    "{} {} {}{}",
                    if selected { "›" } else { " " },
                    position,
                    source,
                    if active { "  [active]" } else { "" }
                );
                draw_styled(
                    &mut out,
                    row as u16 + 1,
                    &line,
                    width,
                    if selected { ACCENT } else { TEXT },
                    Color::Reset,
                    ' ',
                )?;
            }
        } else {
            for (i, line) in preview_rows
                .iter()
                .skip(scroll)
                .take(upper.saturating_sub(1) as usize)
                .enumerate()
            {
                draw_styled(&mut out, i as u16 + 1, line, width, TEXT, Color::Reset, ' ')?;
            }
        }
        draw_styled(
            &mut out,
            upper,
            "─ Function Input  ·  Enter newline  ·  Ctrl+U clear ",
            width,
            ACCENT,
            Color::Reset,
            '─',
        )?;
        for (i, line) in draft_rows.iter().skip(input_scroll).take(lower).enumerate() {
            let y = upper + 1 + i as u16;
            let row_prefix = if input_scroll + i == 0 { &prefix } else { "" };
            draw_styled(
                &mut out,
                y,
                row_prefix,
                prefix_width as u16,
                ACCENT,
                Color::Reset,
                ' ',
            )?;
            draw_syntax_line(&mut out, prefix_width as u16, y, line, content_width as u16)?;
        }
        if let Err(error) = &preview {
            draw_styled(
                &mut out,
                height - 3,
                &format!(" Error: {error}"),
                width,
                ERROR,
                Color::Reset,
                ' ',
            )?;
        }
        if height >= 2 {
            let assist_line = if !completions.is_empty() {
                Some(completion_line(
                    &completions,
                    completion_index % completions.len(),
                ))
            } else {
                argument_hint(editor.draft(), caret).map(|hint| format!(" {hint}"))
            };
            let status_color = if status.contains("failed") || status.contains("blocked") {
                ERROR
            } else if status.starts_with("Saved")
                || status.starts_with("Exported")
                || status.starts_with("Committed")
            {
                SUCCESS
            } else if status.starts_with("Expression exceeds") {
                WARNING
            } else {
                MUTED
            };
            let show_status = status != "Ready. Save writes committed buffer.";
            let has_assist = assist_line.is_some() && !show_status;
            let status_text = if show_status {
                format!("─ {status} ")
            } else {
                assist_line.unwrap_or_else(|| format!("─ {status} "))
            };
            draw_styled(
                &mut out,
                height - 2,
                &status_text,
                width,
                if has_assist { ACCENT } else { status_color },
                Color::Reset,
                '─',
            )?;
            draw_styled(
                &mut out,
                height - 1,
                " F3/^R history · Tab complete · ↑↓ select · ^X commit · ^S commit/save · ^Z undo · ^Y redo · ^Q quit",
                width,
                MUTED,
                Color::Reset,
                ' ',
            )?;
        }
        let cursor_y = upper + 1 + (cy - input_scroll) as u16;
        if cursor_y < height.saturating_sub(2) {
            queue!(out, MoveTo((prefix_width + cx) as u16, cursor_y), Show)?;
        }
        out.flush()?;
        let event = event::read()?;
        if history_open {
            if let Event::Key(key) = &event {
                if key.kind != KeyEventKind::Release {
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('q')
                    {
                        return Ok(());
                    }
                    match key.code {
                        KeyCode::Esc => history_open = false,
                        KeyCode::Up => history_selection = history_selection.saturating_sub(1),
                        KeyCode::Down => history_selection = (history_selection + 1).min(total),
                        KeyCode::Home => history_selection = 0,
                        KeyCode::End => history_selection = total,
                        KeyCode::PageUp => {
                            history_selection = history_selection.saturating_sub(upper as usize - 1)
                        }
                        KeyCode::PageDown => {
                            history_selection = (history_selection + upper as usize - 1).min(total)
                        }
                        KeyCode::Enter => {
                            if history_selection != head {
                                editor.select_history(history_selection);
                                caret = 0;
                                scroll = 0;
                            }
                            history_open = false;
                            status = if history_selection == head {
                                "History unchanged".into()
                            } else {
                                format!("Selected history {history_selection}/{total}")
                            };
                        }
                        _ => {}
                    }
                }
            }
            continue;
        }
        let mut draft = editor.draft().to_owned();
        match event {
            Event::Paste(text) => {
                let text = text.replace("\r\n", "\n").replace('\r', "\n");
                if draft.len().saturating_add(text.len()) <= MAX_EXPRESSION_BYTES {
                    draft.insert_str(caret, &text);
                    caret += text.len();
                } else {
                    status = "Expression exceeds 16 KiB".into();
                }
            }
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                if key.modifiers.contains(KeyModifiers::CONTROL) {
                    match key.code {
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('r') => {
                            history_selection = head;
                            history_open = true;
                            continue;
                        }
                        KeyCode::Char('z') => {
                            editor.undo();
                            caret = 0;
                            scroll = 0;
                            status = "Undo".into();
                            continue;
                        }
                        KeyCode::Char('y') => {
                            editor.redo();
                            caret = 0;
                            scroll = 0;
                            status = "Redo".into();
                            continue;
                        }
                        KeyCode::Char('s') => {
                            if draft.trim().is_empty() {
                                status = match fs::write(&output, editor.state.committed()) {
                                    Ok(()) => format!("Saved {}", output.display()),
                                    Err(e) => format!("Save failed: {e}"),
                                };
                            } else {
                                match editor.commit() {
                                    Ok(()) => {
                                        caret = 0;
                                        scroll = 0;
                                        status = match fs::write(&output, editor.state.committed())
                                        {
                                            Ok(()) => {
                                                format!("Committed and saved {}", output.display())
                                            }
                                            Err(e) => format!("Committed; save failed: {e}"),
                                        };
                                    }
                                    Err(e) => status = format!("Commit blocked: {e}"),
                                }
                            }
                            continue;
                        }
                        KeyCode::Char('x') => {
                            if draft.trim().is_empty() {
                                status = "Nothing to commit".into();
                            } else {
                                status = match editor.commit() {
                                    Ok(()) => {
                                        caret = 0;
                                        scroll = 0;
                                        "Committed".into()
                                    }
                                    Err(e) => format!("Commit blocked: {e}"),
                                };
                            }
                            continue;
                        }
                        KeyCode::Char('e') => {
                            let path = PathBuf::from(format!("{}.transform", output.display()));
                            status = match fs::write(&path, editor.export_script()) {
                                Ok(()) => format!("Exported {}", path.display()),
                                Err(e) => format!("Export failed: {e}"),
                            };
                        }
                        KeyCode::Char('u') => {
                            draft.clear();
                            caret = 0;
                        }
                        _ => {}
                    }
                } else {
                    match key.code {
                        KeyCode::F(2) => {
                            status = match editor.commit() {
                                Ok(()) => {
                                    caret = 0;
                                    scroll = 0;
                                    "Committed".into()
                                }
                                Err(e) => format!("Commit blocked: {e}"),
                            };
                            continue;
                        }
                        KeyCode::F(3) => {
                            history_selection = head;
                            history_open = true;
                            continue;
                        }
                        KeyCode::Tab => {
                            if let Some((start, end, _)) = completion_context {
                                if !completions.is_empty() {
                                    let item = completions[completion_index % completions.len()];
                                    let suffix = draft[end..].to_owned();
                                    let open_paren = suffix
                                        .char_indices()
                                        .find(|(_, ch)| !ch.is_whitespace())
                                        .filter(|(_, ch)| *ch == '(')
                                        .map(|(offset, _)| offset);
                                    draft.replace_range(start..end, item.name);
                                    caret = start + item.name.len();
                                    if let Some(offset) = open_paren {
                                        caret += offset + 1;
                                    } else {
                                        draft.insert_str(caret, "()");
                                        caret += 1;
                                    }
                                }
                            }
                        }
                        KeyCode::Up | KeyCode::Down if !completions.is_empty() => {
                            completion_index = if key.code == KeyCode::Down {
                                completion_index + 1
                            } else {
                                completion_index + completions.len() - 1
                            } % completions.len();
                        }
                        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::ALT) => {
                            if draft.len() + c.len_utf8() <= MAX_EXPRESSION_BYTES {
                                draft.insert(caret, c);
                                caret += c.len_utf8();
                            }
                        }
                        KeyCode::Enter => {
                            if draft.len() < MAX_EXPRESSION_BYTES {
                                draft.insert(caret, '\n');
                                caret += 1;
                            }
                        }
                        KeyCode::Backspace if caret > 0 => {
                            let prev = draft[..caret].char_indices().last().unwrap().0;
                            draft.drain(prev..caret);
                            caret = prev;
                        }
                        KeyCode::Delete if caret < draft.len() => {
                            draft.remove(caret);
                        }
                        KeyCode::Left if caret > 0 => {
                            caret = draft[..caret].char_indices().last().unwrap().0
                        }
                        KeyCode::Right if caret < draft.len() => {
                            caret += draft[caret..].chars().next().unwrap().len_utf8()
                        }
                        KeyCode::Home => caret = draft[..caret].rfind('\n').map_or(0, |i| i + 1),
                        KeyCode::End => {
                            caret += draft[caret..].find('\n').unwrap_or(draft.len() - caret)
                        }
                        KeyCode::PageUp => scroll = scroll.saturating_sub(upper as usize - 1),
                        KeyCode::PageDown => scroll = scroll.saturating_add(upper as usize - 1),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if draft != editor.draft() {
            editor.set_expression(draft);
            completion_index = 0;
            status = "Ready. Save writes committed buffer.".into();
        }
    }
}

fn draw_styled(
    out: &mut impl Write,
    y: u16,
    text: &str,
    width: u16,
    foreground: Color,
    background: Color,
    fill: char,
) -> io::Result<()> {
    draw_at_styled(out, 0, y, text, width, foreground, background, fill)
}

fn draw_at_styled(
    out: &mut impl Write,
    x: u16,
    y: u16,
    text: &str,
    width: u16,
    foreground: Color,
    background: Color,
    fill: char,
) -> io::Result<()> {
    let (lines, _) = rows(text, width as usize, 0);
    let line = &lines[0];
    let used: usize = line.chars().map(|ch| ch.width().unwrap_or(1)).sum();
    queue!(
        out,
        MoveTo(x, y),
        SetForegroundColor(foreground),
        SetBackgroundColor(background),
        Print(line),
        Print(
            fill.to_string()
                .repeat((width as usize).saturating_sub(used))
        ),
        ResetColor
    )
}

fn draw_syntax_line(
    out: &mut impl Write,
    x: u16,
    y: u16,
    line: &str,
    width: u16,
) -> io::Result<()> {
    let chars: Vec<char> = line.chars().collect();
    let mut runs: Vec<(Color, String)> = Vec::new();
    let (mut in_string, mut in_regex, mut escaped) = (false, false, false);
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        let color = if in_string {
            if ch == '"' && !escaped {
                in_string = false;
            }
            if ch == '\\' && !escaped {
                escaped = true;
            } else {
                escaped = false;
            }
            SUCCESS
        } else if in_regex {
            if ch == '/' && !escaped {
                in_regex = false;
            }
            if ch == '\\' && !escaped {
                escaped = true;
            } else {
                escaped = false;
            }
            WARNING
        } else if ch == '"' {
            in_string = true;
            escaped = false;
            SUCCESS
        } else if ch == '/' {
            in_regex = true;
            escaped = false;
            WARNING
        } else if ch.is_ascii_alphabetic() || ch == '_' {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let name: String = chars[start..i].iter().collect();
            let operation = COMPLETIONS.iter().any(|item| item.name == name);
            let token_color = if operation { ACCENT } else { TEXT };
            for token_char in &chars[start..i] {
                push_color(&mut runs, token_color, *token_char);
            }
            continue;
        } else if ch.is_ascii_digit() {
            WARNING
        } else if ch == '|' || ch == '>' || ch == '(' || ch == ')' || ch == ',' {
            MUTED
        } else {
            TEXT
        };
        push_color(&mut runs, color, ch);
        i += 1;
    }

    let used: usize = chars.iter().map(|ch| ch.width().unwrap_or(1)).sum();
    queue!(out, MoveTo(x, y))?;
    for (color, run) in runs {
        queue!(out, SetForegroundColor(color), Print(run))?;
    }
    queue!(
        out,
        SetForegroundColor(TEXT),
        Print(" ".repeat((width as usize).saturating_sub(used))),
        ResetColor
    )
}

fn push_color(runs: &mut Vec<(Color, String)>, color: Color, ch: char) {
    if let Some((last_color, text)) = runs.last_mut() {
        if *last_color == color {
            text.push(ch);
            return;
        }
    }
    runs.push((color, ch.to_string()));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_cursor_and_controls() {
        let (lines, cursor) = rows("あb\x1b", 4, "あb".len());
        assert_eq!(cursor, (0, 3));
        assert_eq!(lines[0], "あb�");
    }
}
