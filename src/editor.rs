use crate::{
    dsl::{eval, parse, Expr},
    Buffer, Error,
};

pub struct EditorState {
    committed: Buffer,
    expression: Result<Expr, Error>,
}

impl EditorState {
    pub fn committed(&self) -> &str {
        &self.committed
    }
    pub fn preview(&self) -> Result<Buffer, Error> {
        match &self.expression {
            Ok(expr) => eval(expr, &self.committed),
            Err(error) => Err(error.clone()),
        }
    }
}

struct Commit {
    source: String,
    expr: Expr,
    after: Buffer,
}

pub struct Editor {
    pub state: EditorState,
    initial: Buffer,
    draft: String,
    commits: Vec<Commit>,
    head: usize,
}

impl Editor {
    pub fn new(buffer: Buffer) -> Self {
        Self {
            initial: buffer.clone(),
            state: EditorState {
                committed: buffer,
                expression: Ok(Expr(vec![])),
            },
            draft: String::new(),
            commits: Vec::new(),
            head: 0,
        }
    }
    pub fn set_expression(&mut self, source: String) {
        self.state.expression = parse(&source);
        self.draft = source;
    }
    pub fn draft(&self) -> &str {
        &self.draft
    }
    pub fn position(&self) -> (usize, usize) {
        (self.head, self.commits.len())
    }
    /// Returns the source expression at a history position, with position 0 as the initial buffer.
    pub fn history_source(&self, position: usize) -> Option<&str> {
        if position == 0 {
            Some("Initial buffer")
        } else {
            self.commits
                .get(position - 1)
                .map(|commit| commit.source.as_str())
        }
    }
    /// Restores the snapshot at a history position and clears the current draft.
    pub fn select_history(&mut self, position: usize) -> bool {
        if position > self.commits.len() {
            return false;
        }
        self.head = position;
        self.restore();
        true
    }
    pub fn commit(&mut self) -> Result<(), Error> {
        let after = self.state.preview()?;
        let expr = self
            .state
            .expression
            .as_ref()
            .map_err(Clone::clone)?
            .clone();
        self.commits.truncate(self.head);
        self.commits.push(Commit {
            source: self.draft.clone(),
            expr,
            after: after.clone(),
        });
        self.head += 1;
        self.state.committed = after;
        // Prevent accidental repeated application immediately after commit.
        self.set_expression(String::new());
        Ok(())
    }
    pub fn undo(&mut self) -> bool {
        if self.head == 0 {
            return false;
        }
        self.head -= 1;
        self.restore();
        true
    }
    pub fn redo(&mut self) -> bool {
        if self.head == self.commits.len() {
            return false;
        }
        self.head += 1;
        self.restore();
        true
    }
    fn restore(&mut self) {
        self.state.committed = if self.head == 0 {
            self.initial.clone()
        } else {
            self.commits[self.head - 1].after.clone()
        };
        self.set_expression(String::new());
    }
    /// A parseable pipeline of the active branch; replay requires the original B0.
    pub fn export_script(&self) -> String {
        self.commits[..self.head]
            .iter()
            .map(|c| c.source.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n|> ")
    }
    pub fn replay(&self) -> Result<Buffer, Error> {
        self.commits[..self.head]
            .iter()
            .try_fold(self.initial.clone(), |buffer, c| eval(&c.expr, &buffer))
    }
}
