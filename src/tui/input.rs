use std::cmp::min;

use ratatui::{style::Style, text::Span};

pub(super) struct InputField {
    label: &'static str,
    value: String,
    cursor: usize,
    secret: bool,
    selection_anchor: Option<usize>,
}

impl InputField {
    pub(super) fn new(label: &'static str, value: String, secret: bool) -> Self {
        let cursor = value.chars().count();
        Self {
            label,
            value,
            cursor,
            secret,
            selection_anchor: None,
        }
    }

    pub(super) fn label(&self) -> &'static str {
        self.label
    }

    pub(super) fn value(&self) -> &str {
        &self.value
    }

    pub(super) fn cursor(&self) -> usize {
        self.cursor
    }

    pub(super) fn display_char_width(&self, _show_secret: bool) -> usize {
        self.value_len()
    }

    pub(super) fn display_spans(
        &self,
        show_secret: bool,
        base_style: Style,
        selected_style: Style,
    ) -> Vec<Span<'static>> {
        let rendered = self.display_value(show_secret);
        let Some((start, end)) = self.selection_range() else {
            return vec![Span::styled(rendered, base_style)];
        };

        let max_len = rendered.chars().count();
        let start = min(start, max_len);
        let end = min(end, max_len);
        if start == end {
            return vec![Span::styled(rendered, base_style)];
        }

        let prefix = char_slice(&rendered, 0, start);
        let selected = char_slice(&rendered, start, end);
        let suffix = char_slice(&rendered, end, max_len);

        let mut spans = Vec::new();
        if !prefix.is_empty() {
            spans.push(Span::styled(prefix, base_style));
        }
        if !selected.is_empty() {
            spans.push(Span::styled(selected, selected_style));
        }
        if !suffix.is_empty() {
            spans.push(Span::styled(suffix, base_style));
        }
        if spans.is_empty() {
            spans.push(Span::styled(String::new(), base_style));
        }
        spans
    }

    pub(super) fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    pub(super) fn set_cursor(&mut self, cursor: usize) {
        self.cursor = min(cursor, self.value_len());
        self.clear_selection();
    }

    pub(super) fn set_cursor_with_anchor(&mut self, anchor: usize, cursor: usize) {
        let len = self.value_len();
        let anchor = min(anchor, len);
        let cursor = min(cursor, len);
        self.cursor = cursor;
        self.selection_anchor = (anchor != cursor).then_some(anchor);
    }

    pub(super) fn insert(&mut self, ch: char) {
        self.delete_selection();
        let idx = byte_index(&self.value, self.cursor);
        self.value.insert(idx, ch);
        self.cursor += 1;
        self.clear_selection();
    }

    pub(super) fn backspace(&mut self) {
        if self.delete_selection() || self.cursor == 0 {
            return;
        }
        let end = byte_index(&self.value, self.cursor);
        let start = byte_index(&self.value, self.cursor - 1);
        self.value.replace_range(start..end, "");
        self.cursor -= 1;
        self.clear_selection();
    }

    pub(super) fn delete(&mut self) {
        if self.delete_selection() || self.cursor >= self.value_len() {
            return;
        }
        let start = byte_index(&self.value, self.cursor);
        let end = byte_index(&self.value, self.cursor + 1);
        self.value.replace_range(start..end, "");
        self.clear_selection();
    }

    pub(super) fn move_left(&mut self) {
        if let Some((start, _)) = self.selection_range() {
            self.cursor = start;
            self.clear_selection();
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.clear_selection();
    }

    pub(super) fn move_right(&mut self) {
        if let Some((_, end)) = self.selection_range() {
            self.cursor = end;
            self.clear_selection();
            return;
        }
        self.cursor = min(self.cursor + 1, self.value_len());
        self.clear_selection();
    }

    pub(super) fn move_home(&mut self) {
        self.cursor = 0;
        self.clear_selection();
    }

    pub(super) fn move_end(&mut self) {
        self.cursor = self.value_len();
        self.clear_selection();
    }

    fn value_len(&self) -> usize {
        self.value.chars().count()
    }

    fn display_value(&self, show_secret: bool) -> String {
        if self.secret && !show_secret {
            "*".repeat(self.value_len())
        } else {
            self.value.clone()
        }
    }

    fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        if start == end {
            self.clear_selection();
            return false;
        }

        let start = byte_index(&self.value, start);
        let end = byte_index(&self.value, end);
        self.value.replace_range(start..end, "");
        self.cursor = min(self.cursor, self.value_len());
        self.cursor = byte_offset_to_char_index(&self.value, start);
        self.clear_selection();
        true
    }
}

fn char_slice(value: &str, start: usize, end: usize) -> String {
    value
        .chars()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect()
}

fn byte_offset_to_char_index(value: &str, byte_offset: usize) -> usize {
    value
        .char_indices()
        .take_while(|(offset, _)| *offset < byte_offset)
        .count()
}

fn byte_index(value: &str, char_index: usize) -> usize {
    if char_index == 0 {
        return 0;
    }
    value
        .char_indices()
        .nth(char_index)
        .map(|(index, _)| index)
        .unwrap_or(value.len())
}
