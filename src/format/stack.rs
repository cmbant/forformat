use crate::classify::StatementKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: StatementKind,
    pub indent: usize,
    pub name: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct IndentStack {
    pub base: usize,
    pub values: Vec<usize>,
    raw_values: Vec<usize>,
    pub frames: Vec<Frame>,
    pub labeled_do: Vec<(u32, usize)>,
    orphan_procedure: Option<StatementKind>,
}

impl IndentStack {
    pub fn new(base: usize) -> Self {
        Self {
            base,
            values: vec![base],
            raw_values: vec![base],
            frames: Vec::new(),
            labeled_do: Vec::new(),
            orphan_procedure: None,
        }
    }
    pub fn current(&self) -> usize {
        *self.values.last().unwrap_or(&self.base)
    }
    pub fn raw_current(&self) -> usize {
        *self.raw_values.last().unwrap_or(&self.base)
    }
    pub fn set_base(&mut self, base: usize) {
        self.base = base;
        if let Some(value) = self.values.first_mut() {
            *value = base;
        }
        if let Some(raw) = self.raw_values.first_mut() {
            *raw = base;
        }
    }
    pub fn push(&mut self, kind: StatementKind, amount: usize, name: Option<Vec<u8>>, max: usize) {
        let raw = self.raw_current().saturating_add(amount);
        self.raw_values.push(raw);
        self.values.push(clamp(raw, max));
        self.frames.push(Frame {
            kind,
            indent: clamp(raw, max),
            name,
        });
    }
    pub fn pop(&mut self) {
        if self.values.len() > 1 {
            self.values.pop();
        }
        if self.raw_values.len() > 1 {
            self.raw_values.pop();
        }
        if !self.frames.is_empty() {
            let removed = self.frames.len() - 1;
            self.frames.pop();
            self.labeled_do.retain(|(_, index)| *index != removed);
        }
    }
    /// Close the top frame only when it is the construct named by an END
    /// statement.  This deliberately does not search down the stack: a
    /// partially typed or malformed END must not consume a valid inner
    /// construct and corrupt all following indentation.
    pub fn pop_kind(&mut self, kind: StatementKind) -> Option<usize> {
        if self.frames.last().is_some_and(|frame| frame.kind == kind) {
            self.pop();
            Some(self.raw_current())
        } else {
            None
        }
    }

    pub fn pop_definition(&mut self) -> Option<usize> {
        if self.frames.last().is_some_and(|frame| {
            matches!(
                frame.kind,
                StatementKind::Program
                    | StatementKind::Module
                    | StatementKind::Submodule
                    | StatementKind::Subroutine
                    | StatementKind::Function
                    | StatementKind::BlockData
                    | StatementKind::Interface
                    | StatementKind::AbstractInterface
                    | StatementKind::Type
                    | StatementKind::Procedure
                    | StatementKind::Structure
                    | StatementKind::Union
                    | StatementKind::Map
            )
        }) {
            self.pop();
            Some(self.raw_current())
        } else {
            None
        }
    }

    pub fn mark_orphan_procedure(&mut self, kind: StatementKind) {
        self.orphan_procedure = Some(kind);
    }

    pub fn pop_orphan_procedure(
        &mut self,
        kind: StatementKind,
        amount: usize,
        max: usize,
    ) -> Option<usize> {
        if self.orphan_procedure != Some(kind) {
            return None;
        }
        self.orphan_procedure = None;
        let raw = self.raw_current().saturating_sub(amount);
        if let Some(value) = self.raw_values.last_mut() {
            *value = raw;
        }
        if let Some(value) = self.values.last_mut() {
            *value = clamp(raw, max);
        }
        Some(raw)
    }

    pub fn recover_definition_end(&mut self, amount: usize, max: usize) -> Option<usize> {
        let index = self.frames.iter().rposition(|frame| {
            matches!(
                frame.kind,
                StatementKind::Program
                    | StatementKind::Module
                    | StatementKind::Submodule
                    | StatementKind::Subroutine
                    | StatementKind::Function
                    | StatementKind::BlockData
                    | StatementKind::Interface
                    | StatementKind::AbstractInterface
                    | StatementKind::Type
                    | StatementKind::Procedure
                    | StatementKind::Structure
                    | StatementKind::Union
                    | StatementKind::Map
            )
        })?;
        self.frames.remove(index);
        self.values.remove(index + 1);
        self.raw_values.remove(index + 1);
        for (_, other) in &mut self.labeled_do {
            if *other > index {
                *other -= 1;
            }
        }
        let raw = self.raw_current().saturating_sub(amount);
        if let Some(value) = self.raw_values.last_mut() {
            *value = raw;
        }
        if let Some(value) = self.values.last_mut() {
            *value = clamp(raw, max);
        }
        Some(raw)
    }
    pub fn branch(&mut self, amount: usize) {
        let n = self.raw_current().saturating_sub(amount);
        if let Some(raw) = self.raw_values.last_mut() {
            *raw = n;
        }
        if let Some(v) = self.values.last_mut() {
            *v = n;
        }
    }
    /// Replace the active indentation entry with the initial level while
    /// retaining definition/construct frames.  The legacy formatter's
    /// CONTAINS restart does exactly this (`pop_indent` followed by
    /// `push_indent`); clearing frames would make later END statements lose
    /// their enclosing definitions.
    pub fn restart_at_base(&mut self, max: usize) {
        if let Some(raw) = self.raw_values.last_mut() {
            *raw = self.base;
        }
        if let Some(value) = self.values.last_mut() {
            *value = clamp(self.base, max);
        }
    }
    pub fn snapshot(&self) -> Self {
        self.clone()
    }
    pub fn close_label(&mut self, label: u32) {
        let mut indices: Vec<usize> = self
            .labeled_do
            .iter()
            .filter_map(|(value, index)| (*value == label).then_some(*index))
            .collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        for index in indices {
            if index < self.frames.len() {
                self.frames.remove(index);
                self.values.remove(index + 1);
                self.raw_values.remove(index + 1);
                for (_, other) in &mut self.labeled_do {
                    if *other > index {
                        *other -= 1;
                    }
                }
            }
        }
        self.labeled_do.retain(|(value, _)| *value != label);
    }
    pub fn label_do(&mut self, label: u32) {
        self.labeled_do
            .push((label, self.frames.len().saturating_sub(1)));
    }
}
pub fn clamp(n: usize, max: usize) -> usize {
    if max == 0 {
        n
    } else {
        n.min(max)
    }
}

#[cfg(test)]
mod tests {
    use super::IndentStack;
    use crate::classify::StatementKind;

    #[test]
    fn empty_and_mismatched_closes_never_underflow() {
        let mut stack = IndentStack::new(0);
        for _ in 0..1024 {
            stack.pop();
            assert_eq!(stack.current(), 0);
            assert_eq!(stack.raw_current(), 0);
        }
        assert_eq!(stack.pop_kind(StatementKind::If), None);
        assert_eq!(stack.recover_definition_end(3, 100), None);
        stack.close_label(123);
        assert!(stack.frames.is_empty());
    }

    #[test]
    fn raw_depth_and_visible_max_indent_remain_separate() {
        let mut stack = IndentStack::new(2);
        stack.push(StatementKind::If, 5, None, 6);
        stack.push(StatementKind::Do, 5, None, 6);
        assert_eq!(stack.current(), 6);
        assert_eq!(stack.raw_current(), 12);
        stack.branch(100);
        assert_eq!(stack.current(), 0);
        assert_eq!(stack.raw_current(), 0);
        stack.pop();
        stack.pop();
        assert!(stack.frames.is_empty());
        assert_eq!(stack.current(), 2);
    }

    #[test]
    fn contains_restart_keeps_definition_frames() {
        let mut stack = IndentStack::new(0);
        stack.push(StatementKind::Module, 3, Some(b"m".to_vec()), 100);
        stack.push(StatementKind::Type, 3, Some(b"t".to_vec()), 100);
        stack.restart_at_base(100);
        assert_eq!(stack.current(), 0);
        assert_eq!(stack.frames.len(), 2);
        assert_eq!(stack.pop_definition(), Some(3));
        assert_eq!(
            stack.frames.last().map(|frame| frame.kind),
            Some(StatementKind::Module)
        );
        assert_eq!(stack.pop_definition(), Some(0));
    }
}
