use crossterm::event::Event as CtEvent;
use tui_input::backend::crossterm::EventHandler;
use tui_input::{Input, InputRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VimMode {
    Normal,
    Insert,
    Visual,
}

#[derive(Debug, Clone)]
pub(crate) struct VimInput {
    input: Input,
    mode: VimMode,
    visual_anchor: Option<usize>,
    pending_d: bool,
}

impl Default for VimInput {
    fn default() -> Self {
        Self {
            input: Input::default(),
            mode: VimMode::Normal,
            visual_anchor: None,
            pending_d: false,
        }
    }
}

impl VimInput {
    #[cfg(test)]
    pub(crate) fn new(value: String) -> Self {
        let input = Input::new(value);
        let mut this = Self {
            input,
            ..Self::default()
        };
        this.clamp_normal_cursor();
        this
    }

    pub(crate) fn value(&self) -> &str {
        self.input.value()
    }

    pub(crate) fn cursor(&self) -> usize {
        self.input.cursor()
    }

    pub(crate) fn visual_cursor(&self) -> usize {
        self.input.visual_cursor()
    }

    pub(crate) fn visual_scroll(&self, width: usize) -> usize {
        self.input.visual_scroll(width)
    }

    pub(crate) fn mode(&self) -> VimMode {
        self.mode
    }

    /// Whether a `d` operator is waiting for its second keypress (`dd`).
    pub(crate) fn has_pending_d(&self) -> bool {
        self.pending_d
    }

    pub(crate) fn set_pending_d(&mut self) {
        self.pending_d = true;
    }

    pub(crate) fn clear_pending_d(&mut self) {
        self.pending_d = false;
    }

    pub(crate) fn selection_range(&self) -> Option<(usize, usize)> {
        let anchor = self.visual_anchor?;
        if self.value().is_empty() {
            return None;
        }
        Some((anchor.min(self.cursor()), anchor.max(self.cursor())))
    }

    pub(crate) fn enter_normal(&mut self) {
        if self.mode == VimMode::Insert && self.cursor() > 0 {
            self.input.handle(InputRequest::GoToPrevChar);
        }
        self.mode = VimMode::Normal;
        self.visual_anchor = None;
        self.pending_d = false;
        self.clamp_normal_cursor();
    }

    pub(crate) fn enter_insert_before(&mut self) {
        self.mode = VimMode::Insert;
        self.visual_anchor = None;
        self.pending_d = false;
    }

    pub(crate) fn enter_insert_after(&mut self) {
        if !self.value().is_empty() {
            self.input.handle(InputRequest::GoToNextChar);
        }
        self.enter_insert_before();
    }

    pub(crate) fn enter_insert_start(&mut self) {
        self.input.handle(InputRequest::GoToStart);
        self.enter_insert_before();
    }

    pub(crate) fn enter_insert_end(&mut self) {
        self.input.handle(InputRequest::GoToEnd);
        self.enter_insert_before();
    }

    pub(crate) fn enter_visual(&mut self) {
        if !self.value().is_empty() {
            self.visual_anchor = Some(self.cursor());
            self.mode = VimMode::Visual;
            self.pending_d = false;
        }
    }

    pub(crate) fn handle_insert_event(&mut self, event: &CtEvent) -> bool {
        let before = self.value().to_string();
        self.input.handle_event(event);
        self.value() != before
    }

    pub(crate) fn move_left(&mut self) {
        self.input.handle(InputRequest::GoToPrevChar);
    }

    pub(crate) fn move_right(&mut self) {
        self.input.handle(InputRequest::GoToNextChar);
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_prev_word(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = prev_word_start(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_prev_big_word(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = prev_big_word_start(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_next_word(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = next_word_start(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_next_big_word(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = next_big_word_start(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_word_end(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = word_end(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_big_word_end(&mut self) {
        let chars: Vec<char> = self.value().chars().collect();
        let target = big_word_end(&chars, self.cursor());
        self.input.handle(InputRequest::SetCursor(target));
        self.clamp_normal_cursor();
    }

    pub(crate) fn move_start(&mut self) {
        self.input.handle(InputRequest::GoToStart);
    }

    pub(crate) fn move_end(&mut self) {
        self.input.handle(InputRequest::GoToEnd);
        self.clamp_normal_cursor();
    }

    pub(crate) fn delete_char(&mut self) -> bool {
        if self.value().is_empty() {
            return false;
        }
        let before = self.value().to_string();
        self.input.handle(InputRequest::DeleteNextChar);
        self.pending_d = false;
        self.clamp_normal_cursor();
        self.value() != before
    }

    pub(crate) fn delete_to_end(&mut self) -> bool {
        if self.value().is_empty() {
            return false;
        }
        let before = self.value().to_string();
        self.input.handle(InputRequest::DeleteTillEnd);
        self.pending_d = false;
        self.clamp_normal_cursor();
        self.value() != before
    }

    /// Deletes the entire line, mirroring vim's `dd` in normal mode.
    pub(crate) fn delete_line(&mut self) -> bool {
        if self.value().is_empty() {
            return false;
        }
        let before = self.value().to_string();
        self.input.handle(InputRequest::DeleteLine);
        self.visual_anchor = None;
        self.pending_d = false;
        self.clamp_normal_cursor();
        self.value() != before
    }

    pub(crate) fn delete_selection(&mut self, enter_insert: bool) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        let value: String = self
            .value()
            .chars()
            .enumerate()
            .filter_map(|(index, ch)| (!(start..=end).contains(&index)).then_some(ch))
            .collect();
        self.input = Input::new(value).with_cursor(start);
        self.visual_anchor = None;
        self.pending_d = false;
        if enter_insert {
            self.mode = VimMode::Insert;
        } else {
            self.mode = VimMode::Normal;
            self.clamp_normal_cursor();
        }
        true
    }

    fn clamp_normal_cursor(&mut self) {
        if self.mode == VimMode::Insert {
            return;
        }
        let len = self.value().chars().count();
        if len > 0 && self.cursor() >= len {
            self.input.handle(InputRequest::SetCursor(len - 1));
        }
    }
}

/// Character class for vim small-word (`w`/`b`/`e`) motions: runs of word
/// characters (`a-z`, `0-9`, `_`) and runs of punctuation each count as one
/// word, mirroring nvim's default `iskeyword` behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WordClass {
    Blank,
    Word,
    Punct,
}

fn is_blank(ch: char) -> bool {
    ch.is_whitespace()
}

fn is_word_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

fn small_class(ch: char) -> WordClass {
    if is_blank(ch) {
        WordClass::Blank
    } else if is_word_char(ch) {
        WordClass::Word
    } else {
        WordClass::Punct
    }
}

/// Start of the next small word (`w`). Falls back to the last character when
/// no further word exists, matching nvim on a single line.
fn next_word_start(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let mut index = cursor.min(last);
    let start = chars
        .get(index)
        .map_or(WordClass::Blank, |ch| small_class(*ch));
    if start == WordClass::Blank {
        while index < last {
            let next = index.saturating_add(1);
            if chars.get(next).is_some_and(|ch| is_blank(*ch)) {
                index = next;
            } else {
                return next;
            }
        }
        return last;
    }
    while index < last {
        let next = index.saturating_add(1);
        if chars.get(next).is_some_and(|ch| small_class(*ch) == start) {
            index = next;
        } else {
            break;
        }
    }
    while index < last {
        let next = index.saturating_add(1);
        if chars.get(next).is_some_and(|ch| is_blank(*ch)) {
            index = next;
        } else {
            break;
        }
    }
    if index < last {
        index.saturating_add(1)
    } else {
        last
    }
}

/// Start of the next blank-delimited WORD (`W`).
fn next_big_word_start(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let mut index = cursor.min(last);
    if chars.get(index).is_some_and(|ch| !is_blank(*ch)) {
        while index < last {
            let next = index.saturating_add(1);
            if chars.get(next).is_some_and(|ch| !is_blank(*ch)) {
                index = next;
            } else {
                break;
            }
        }
    }
    while index < last {
        let next = index.saturating_add(1);
        if chars.get(next).is_some_and(|ch| is_blank(*ch)) {
            index = next;
        } else {
            break;
        }
    }
    if index < last {
        index.saturating_add(1)
    } else {
        last
    }
}

/// Start of the current or previous small word (`b`).
fn prev_word_start(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let cursor = cursor.min(last);
    if cursor == 0 {
        return 0;
    }
    let mut index = cursor.saturating_sub(1);
    while index > 0 && chars.get(index).is_some_and(|ch| is_blank(*ch)) {
        index = index.saturating_sub(1);
    }
    let class = chars
        .get(index)
        .map_or(WordClass::Blank, |ch| small_class(*ch));
    if class == WordClass::Blank {
        return index;
    }
    while index > 0 {
        let prev = index.saturating_sub(1);
        if chars.get(prev).is_some_and(|ch| small_class(*ch) == class) {
            index = prev;
        } else {
            break;
        }
    }
    index
}

/// Start of the current or previous blank-delimited WORD (`B`).
fn prev_big_word_start(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let cursor = cursor.min(last);
    if cursor == 0 {
        return 0;
    }
    let mut index = cursor.saturating_sub(1);
    while index > 0 && chars.get(index).is_some_and(|ch| is_blank(*ch)) {
        index = index.saturating_sub(1);
    }
    if chars.get(index).is_some_and(|ch| is_blank(*ch)) {
        return index;
    }
    while index > 0 {
        let prev = index.saturating_sub(1);
        if chars.get(prev).is_some_and(|ch| !is_blank(*ch)) {
            index = prev;
        } else {
            break;
        }
    }
    index
}

/// End of the current or next small word (`e`).
fn word_end(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let mut index = cursor.min(last);
    if index >= last {
        return last;
    }
    let current = chars
        .get(index)
        .map_or(WordClass::Blank, |ch| small_class(*ch));
    let at_word_end = current != WordClass::Blank && {
        let next = index.saturating_add(1);
        chars.get(next).is_none_or(|ch| {
            let next_class = small_class(*ch);
            next_class == WordClass::Blank || next_class != current
        })
    };
    if current == WordClass::Blank || at_word_end {
        index = index.saturating_add(1);
        while index < last && chars.get(index).is_some_and(|ch| is_blank(*ch)) {
            index = index.saturating_add(1);
        }
        let next_class = chars
            .get(index)
            .map_or(WordClass::Blank, |ch| small_class(*ch));
        if next_class == WordClass::Blank {
            return last;
        }
        while index < last {
            let next = index.saturating_add(1);
            if chars
                .get(next)
                .is_some_and(|ch| small_class(*ch) == next_class)
            {
                index = next;
            } else {
                break;
            }
        }
        return index;
    }
    while index < last {
        let next = index.saturating_add(1);
        if chars
            .get(next)
            .is_some_and(|ch| small_class(*ch) == current)
        {
            index = next;
        } else {
            break;
        }
    }
    index
}

/// End of the current or next blank-delimited WORD (`E`).
fn big_word_end(chars: &[char], cursor: usize) -> usize {
    if chars.is_empty() {
        return 0;
    }
    let last = chars.len().saturating_sub(1);
    let mut index = cursor.min(last);
    if index >= last {
        return last;
    }
    let on_word = chars.get(index).is_some_and(|ch| !is_blank(*ch));
    let at_word_end = on_word && {
        let next = index.saturating_add(1);
        chars.get(next).is_none_or(|ch| is_blank(*ch))
    };
    if !on_word || at_word_end {
        index = index.saturating_add(1);
        while index < last && chars.get(index).is_some_and(|ch| is_blank(*ch)) {
            index = index.saturating_add(1);
        }
        if chars.get(index).is_some_and(|ch| is_blank(*ch)) {
            return last;
        }
        while index < last {
            let next = index.saturating_add(1);
            if chars.get(next).is_some_and(|ch| !is_blank(*ch)) {
                index = next;
            } else {
                break;
            }
        }
        return index;
    }
    while index < last {
        let next = index.saturating_add(1);
        if chars.get(next).is_some_and(|ch| !is_blank(*ch)) {
            index = next;
        } else {
            break;
        }
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_mode_keeps_cursor_on_a_character() {
        let mut input = VimInput::new("abc".into());
        assert_eq!(input.cursor(), 2);

        input.move_right();
        assert_eq!(input.cursor(), 2);
        input.move_start();
        input.move_left();
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn visual_delete_removes_the_inclusive_selection() {
        let mut input = VimInput::new("abcdef".into());
        input.move_start();
        input.move_right();
        input.enter_visual();
        input.move_right();
        input.move_right();

        assert_eq!(input.selection_range(), Some((1, 3)));
        assert!(input.delete_selection(false));
        assert_eq!(input.value(), "aef");
        assert_eq!(input.cursor(), 1);
        assert_eq!(input.mode(), VimMode::Normal);
    }

    #[test]
    fn append_and_escape_follow_vim_cursor_semantics() {
        let mut input = VimInput::new("ab".into());
        input.enter_insert_after();
        input.handle_insert_event(&CtEvent::Key(crossterm::event::KeyEvent::from(
            crossterm::event::KeyCode::Char('c'),
        )));
        input.enter_normal();

        assert_eq!(input.value(), "abc");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn delete_line_clears_the_whole_value() {
        let mut input = VimInput::new("ripgrep".into());
        assert!(!input.has_pending_d());
        input.set_pending_d();
        assert!(input.has_pending_d());

        assert!(input.delete_line());
        assert_eq!(input.value(), "");
        assert_eq!(input.cursor(), 0);
        assert!(!input.has_pending_d());
        assert!(!input.delete_line());
    }

    #[test]
    fn pending_delete_clears_on_mode_transitions() {
        let mut input = VimInput::new("abc".into());
        input.set_pending_d();
        input.enter_visual();
        assert!(!input.has_pending_d());

        input.set_pending_d();
        input.enter_insert_before();
        assert!(!input.has_pending_d());
    }

    #[test]
    fn small_word_motions_treat_punctuation_as_words() {
        let mut input = VimInput::new("foo,bar baz".into());
        input.move_start();
        assert_eq!(input.cursor(), 0);

        input.move_next_word();
        assert_eq!(input.cursor(), 3);
        input.move_next_word();
        assert_eq!(input.cursor(), 4);
        input.move_word_end();
        assert_eq!(input.cursor(), 6);
        input.move_next_word();
        assert_eq!(input.cursor(), 8);

        input.move_prev_word();
        assert_eq!(input.cursor(), 4);
        input.move_prev_word();
        assert_eq!(input.cursor(), 3);
        input.move_prev_word();
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn big_word_motions_skip_to_blank_boundaries() {
        let mut input = VimInput::new("foo,bar baz".into());
        input.move_start();

        input.move_next_big_word();
        assert_eq!(input.cursor(), 8);
        input.move_big_word_end();
        assert_eq!(input.cursor(), 10);

        input.move_prev_big_word();
        assert_eq!(input.cursor(), 8);
        input.move_prev_big_word();
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn word_end_stays_within_the_current_word() {
        let mut input = VimInput::new("foo bar".into());
        input.move_start();
        assert_eq!(input.cursor(), 0);

        input.move_word_end();
        assert_eq!(input.cursor(), 2);
        input.move_word_end();
        assert_eq!(input.cursor(), 6);

        input.move_start();
        input.move_big_word_end();
        assert_eq!(input.cursor(), 2);
    }
}
