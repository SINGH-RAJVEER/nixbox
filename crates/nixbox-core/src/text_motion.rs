//! Character-indexed Vim word motions shared by the terminal and desktop inputs.

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
pub fn next_word_start(chars: &[char], cursor: usize) -> usize {
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
pub fn next_big_word_start(chars: &[char], cursor: usize) -> usize {
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
pub fn prev_word_start(chars: &[char], cursor: usize) -> usize {
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
pub fn prev_big_word_start(chars: &[char], cursor: usize) -> usize {
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
pub fn word_end(chars: &[char], cursor: usize) -> usize {
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
pub fn big_word_end(chars: &[char], cursor: usize) -> usize {
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
