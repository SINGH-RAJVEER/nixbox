//! Within-line change emphasis, adapted from delta's `align.rs` and
//! `edits.rs` (https://github.com/dandavison/delta).
//!
//! Copyright 2020 Dan Davison. Used under the MIT license:
//!
//! Permission is hereby granted, free of charge, to any person obtaining a
//! copy of this software and associated documentation files (the
//! "Software"), to deal in the Software without restriction, including
//! without limitation the rights to use, copy, modify, merge, publish,
//! distribute, sublicense, and/or sell copies of the Software, and to permit
//! persons to whom the Software is furnished to do so, subject to the
//! following conditions:
//!
//! The above copyright notice and this permission notice shall be included
//! in all copies or substantial portions of the Software.
//!
//! THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
//! OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF
//! MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN
//! NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
//! DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR
//! OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
//! USE OR OTHER DEALINGS IN THE SOFTWARE.
//!
//! Delta pairs each removed line with the first sufficiently similar added line of the
//! same change block, aligns the two token by token, and emphasizes the
//! tokens that differ. The pairing and alignment below follow delta's
//! defaults: `\w+` tokens, a maximum line distance of 0.6, and no
//! highlighting for lines longer than 512 bytes.

use std::cmp::max;
use std::collections::VecDeque;
use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Delta's `--word-diff-regex` default.
static TOKENS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\w+").expect("token regex"));

/// Delta's `--max-line-distance` default: pairs further apart stay unpaired.
const MAX_LINE_DISTANCE: f64 = 0.6;

/// Delta's `--max-line-length` default. Longer lines are not aligned.
const MAX_LINE_LENGTH: usize = 512;

/// Alignment is quadratic in tokens and pairing is quadratic in lines, so a
/// block larger than this many candidate pairs is shown without emphasis.
const MAX_PAIRS: usize = 64 * 64;

/// Bound the total alignment work even for punctuation-heavy short lines.
const MAX_CELLS: usize = 2_000_000;

/// Emphasized byte ranges for each line of one side of a change block.
pub type Emphasis = Vec<Vec<Range<usize>>>;

/// Byte ranges to emphasize in each removed and each added line of one
/// change block, a run of removed lines directly followed by added lines.
pub fn emphasis(minus: &[&str], plus: &[&str]) -> (Emphasis, Emphasis) {
	let none = || (vec![Vec::new(); minus.len()], vec![Vec::new(); plus.len()]);
	let too_long = |line: &&str| line.len() > MAX_LINE_LENGTH;
	if minus.is_empty()
		|| plus.is_empty()
		|| minus.len() * plus.len() > MAX_PAIRS
		|| minus.iter().any(too_long)
		|| plus.iter().any(too_long)
	{
		return none();
	}
	let cells = |lines: &[&str]| {
		lines
			.iter()
			.map(|line| tokenize(line).len() + 1)
			.sum::<usize>()
	};
	if cells(minus).saturating_mul(cells(plus)) > MAX_CELLS {
		return none();
	}
	let (minus_lines, plus_lines) = infer_edits(minus, plus);
	(
		minus_lines
			.iter()
			.map(|line| changed(line, Edit::Deletion))
			.collect(),
		plus_lines
			.iter()
			.map(|line| changed(line, Edit::Insertion))
			.collect(),
	)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Edit {
	MinusNoop,
	PlusNoop,
	Deletion,
	Insertion,
}

type Annotated<'a> = Vec<(Edit, &'a str)>;

/// Merged byte ranges of the sections annotated `edit`. Sections concatenate
/// to the line, so offsets accumulate.
fn changed(line: &Annotated<'_>, edit: Edit) -> Vec<Range<usize>> {
	let mut ranges: Vec<Range<usize>> = Vec::new();
	let mut offset = 0;
	for (op, section) in line {
		let end = offset + section.len();
		if *op == edit && !section.is_empty() {
			match ranges.last_mut() {
				Some(last) if last.end == offset => last.end = end,
				_ => ranges.push(offset..end),
			}
		}
		offset = end;
	}
	ranges
}

/// Delta's `infer_edits`: greedily pairs each minus line with the first
/// following plus line within the maximum distance. Unpaired lines are
/// returned unannotated.
fn infer_edits<'a>(
	minus_lines: &[&'a str],
	plus_lines: &[&'a str],
) -> (Vec<Annotated<'a>>, Vec<Annotated<'a>>) {
	let mut annotated_minus_lines = Vec::new();
	let mut annotated_plus_lines = Vec::new();
	let mut plus_index = 0;

	'minus_lines_loop: for minus_line in minus_lines {
		let mut considered = 0;
		for plus_line in &plus_lines[plus_index..] {
			let alignment = Alignment::new(tokenize(minus_line), tokenize(plus_line));
			let (annotated_minus_line, annotated_plus_line, distance) =
				annotate(&alignment, minus_line, plus_line);
			if distance <= MAX_LINE_DISTANCE {
				// Emit as unpaired the plus lines already considered and rejected.
				for plus_line in &plus_lines[plus_index..plus_index + considered] {
					annotated_plus_lines.push(vec![(Edit::PlusNoop, *plus_line)]);
				}
				plus_index += considered;
				annotated_minus_lines.push(annotated_minus_line);
				annotated_plus_lines.push(annotated_plus_line);
				plus_index += 1;
				continue 'minus_lines_loop;
			}
			considered += 1;
		}
		annotated_minus_lines.push(vec![(Edit::MinusNoop, *minus_line)]);
	}
	for plus_line in &plus_lines[plus_index..] {
		annotated_plus_lines.push(vec![(Edit::PlusNoop, *plus_line)]);
	}
	(annotated_minus_lines, annotated_plus_lines)
}

/// Splits a line into `\w+` tokens and single graphemes between them. The
/// leading "" is required by the alignment, as in delta.
fn tokenize(line: &str) -> Vec<&str> {
	let mut tokens = vec![""];
	let mut offset = 0;
	for m in TOKENS.find_iter(line) {
		if offset == 0 && m.start() > 0 {
			tokens.push("");
		}
		tokens.extend(line[offset..m.start()].graphemes(true));
		tokens.push(m.as_str());
		offset = m.end();
	}
	if offset < line.len() {
		if offset == 0 {
			tokens.push("");
		}
		tokens.extend(line[offset..].graphemes(true));
	}
	tokens
}

/// Delta's `annotate`: turns the alignment into sections of both lines and
/// the distance between them, the changed share of their visible width.
fn annotate<'a>(
	alignment: &Alignment<'a>,
	minus_line: &'a str,
	plus_line: &'a str,
) -> (Annotated<'a>, Annotated<'a>, f64) {
	let mut annotated_minus_line = Vec::new();
	let mut annotated_plus_line = Vec::new();
	let (mut x_offset, mut y_offset) = (0, 0);
	let (mut minus_line_offset, mut plus_line_offset) = (0, 0);
	let (mut d_numer, mut d_denom) = (0, 0);

	let section =
		|n: usize, line_offset: &mut usize, offset: &mut usize, tokens: &[&str], line: &'a str| {
			let length: usize = tokens[*offset..*offset + n]
				.iter()
				.map(|token| token.len())
				.sum();
			let start = *line_offset;
			*line_offset += length;
			*offset += n;
			&line[start..*line_offset]
		};
	let width = |section: &str| UnicodeWidthStr::width(section.trim());

	let (mut minus_op_prev, mut plus_op_prev) = (Edit::MinusNoop, Edit::PlusNoop);
	for (op, n) in alignment.coalesced_operations() {
		match op {
			Operation::Deletion => {
				let minus = section(
					n,
					&mut minus_line_offset,
					&mut x_offset,
					&alignment.x,
					minus_line,
				);
				let n_d = width(minus);
				d_denom += n_d;
				d_numer += n_d;
				annotated_minus_line.push((Edit::Deletion, minus));
				minus_op_prev = Edit::Deletion;
			}
			Operation::NoOp => {
				let minus = section(
					n,
					&mut minus_line_offset,
					&mut x_offset,
					&alignment.x,
					minus_line,
				);
				d_denom += 2 * width(minus);
				// Whitespace between two changes is emphasized with them.
				let coalesce = minus.trim().is_empty()
					&& ((minus_op_prev == Edit::Deletion
						&& plus_op_prev == Edit::Insertion
						&& (x_offset < alignment.x.len() - 1 || y_offset < alignment.y.len() - 1))
						|| (minus_op_prev == Edit::MinusNoop && plus_op_prev == Edit::PlusNoop));
				annotated_minus_line.push((
					if coalesce {
						minus_op_prev
					} else {
						Edit::MinusNoop
					},
					minus,
				));
				let plus = section(
					n,
					&mut plus_line_offset,
					&mut y_offset,
					&alignment.y,
					plus_line,
				);
				annotated_plus_line.push((
					if coalesce {
						plus_op_prev
					} else {
						Edit::PlusNoop
					},
					plus,
				));
				minus_op_prev = Edit::MinusNoop;
				plus_op_prev = Edit::PlusNoop;
			}
			Operation::Insertion => {
				let plus = section(
					n,
					&mut plus_line_offset,
					&mut y_offset,
					&alignment.y,
					plus_line,
				);
				let n_d = width(plus);
				d_denom += n_d;
				d_numer += n_d;
				annotated_plus_line.push((Edit::Insertion, plus));
				plus_op_prev = Edit::Insertion;
			}
		}
	}
	let distance = if d_denom > 0 {
		f64::from(u32::try_from(d_numer).unwrap_or(u32::MAX))
			/ f64::from(u32::try_from(d_denom).unwrap_or(u32::MAX))
	} else {
		0.0
	};
	(annotated_minus_line, annotated_plus_line, distance)
}

const DELETION_COST: usize = 2;
const INSERTION_COST: usize = 2;
/// Extra cost for starting a new group of changed tokens.
const INITIAL_MISMATCH_PENALTY: usize = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operation {
	NoOp,
	Deletion,
	Insertion,
}

#[derive(Clone, Debug)]
struct Cell {
	parent: usize,
	operation: Operation,
	cost: usize,
}

/// Delta's Needleman-Wunsch / Wagner-Fischer table for the edit distance
/// between two token sequences and the alignment behind it.
struct Alignment<'a> {
	x: Vec<&'a str>,
	y: Vec<&'a str>,
	table: Vec<Cell>,
	dim: [usize; 2],
}

impl<'a> Alignment<'a> {
	fn new(x: Vec<&'a str>, y: Vec<&'a str>) -> Self {
		let dim = [y.len() + 1, x.len() + 1];
		let table = vec![
			Cell {
				parent: 0,
				operation: Operation::NoOp,
				cost: 0,
			};
			dim[0] * dim[1]
		];
		let mut alignment = Self { x, y, table, dim };
		alignment.fill();
		alignment
	}

	fn fill(&mut self) {
		for i in 1..self.dim[1] {
			self.table[i] = Cell {
				parent: 0,
				operation: Operation::Deletion,
				cost: i * DELETION_COST + INITIAL_MISMATCH_PENALTY,
			};
		}
		for j in 1..self.dim[0] {
			self.table[j * self.dim[1]] = Cell {
				parent: 0,
				operation: Operation::Insertion,
				cost: j * INSERTION_COST + INITIAL_MISMATCH_PENALTY,
			};
		}
		for (i, x_i) in self.x.iter().enumerate() {
			for (j, y_j) in self.y.iter().enumerate() {
				let (left, diag, up) =
					(self.index(i, j + 1), self.index(i, j), self.index(i + 1, j));
				// Ties go to the first candidate: insertions and deletions
				// before matches, to group changes, and insertions before
				// deletions, so a moved token reads as delete then insert.
				let candidates = [
					Cell {
						parent: up,
						operation: Operation::Insertion,
						cost: self.mismatch_cost(up, INSERTION_COST),
					},
					Cell {
						parent: left,
						operation: Operation::Deletion,
						cost: self.mismatch_cost(left, DELETION_COST),
					},
					Cell {
						parent: diag,
						operation: Operation::NoOp,
						cost: if x_i == y_j {
							self.table[diag].cost
						} else {
							usize::MAX
						},
					},
				];
				let index = self.index(i + 1, j + 1);
				if let Some(best) = candidates.into_iter().min_by_key(|cell| cell.cost) {
					self.table[index] = best;
				}
			}
		}
	}

	fn mismatch_cost(&self, parent: usize, basic_cost: usize) -> usize {
		self.table[parent].cost
			+ basic_cost
			+ if self.table[parent].operation == Operation::NoOp {
				INITIAL_MISMATCH_PENALTY
			} else {
				0
			}
	}

	fn operations(&self) -> Vec<Operation> {
		let mut ops = VecDeque::with_capacity(max(self.x.len(), self.y.len()));
		let mut cell = &self.table[self.index(self.x.len(), self.y.len())];
		loop {
			ops.push_front(cell.operation);
			if cell.parent == 0 {
				break;
			}
			cell = &self.table[cell.parent];
		}
		Vec::from(ops)
	}

	fn coalesced_operations(&self) -> Vec<(Operation, usize)> {
		let mut encoded: Vec<(Operation, usize)> = Vec::new();
		for op in self.operations() {
			match encoded.last_mut() {
				Some((last, n)) if *last == op => *n += 1,
				_ => encoded.push((op, 1)),
			}
		}
		encoded
	}

	/// Row-major storage of the 2D table.
	fn index(&self, i: usize, j: usize) -> usize {
		j * self.dim[1] + i
	}
}

#[cfg(test)]
mod tests {
	use super::{Alignment, Operation, emphasis, tokenize};

	fn emphasized<'a>(line: &'a str, ranges: &[std::ops::Range<usize>]) -> Vec<&'a str> {
		ranges.iter().map(|range| &line[range.clone()]).collect()
	}

	#[test]
	fn tokens_are_words_and_single_characters_after_a_leading_empty_token() {
		assert_eq!(tokenize(""), [""]);
		assert_eq!(tokenize(";"), ["", "", ";"]);
		assert_eq!(tokenize("pkgs.ripgrep"), ["", "pkgs", ".", "ripgrep"]);
		assert_eq!(tokenize(" a"), ["", "", " ", "a"]);
	}

	#[test]
	fn a_changed_word_aligns_as_one_deletion_and_one_insertion() {
		let alignment = Alignment::new(tokenize("a bc d"), tokenize("a xy d"));
		let ops = alignment.coalesced_operations();
		let count = |op| {
			ops.iter()
				.filter(|(o, _)| *o == op)
				.map(|(_, n)| n)
				.sum::<usize>()
		};
		assert_eq!(count(Operation::Deletion), 1);
		assert_eq!(count(Operation::Insertion), 1);
	}

	#[test]
	fn similar_lines_emphasize_only_the_changed_word() {
		let minus = "  programs.git.enable = false;";
		let plus = "  programs.git.enable = true;";
		let (removed, added) = emphasis(&[minus], &[plus]);
		assert_eq!(emphasized(minus, &removed[0]), ["false"]);
		assert_eq!(emphasized(plus, &added[0]), ["true"]);
	}

	#[test]
	fn dissimilar_lines_stay_unpaired_and_unemphasized() {
		let (removed, added) = emphasis(&["home.stateVersion = 1;"], &["imports = [ ./x.nix ];"]);
		assert!(removed[0].is_empty());
		assert!(added[0].is_empty());
	}

	#[test]
	fn pairing_skips_an_inserted_line_to_reach_the_homolog() {
		let (removed, added) = emphasis(
			&["    ripgrep"],
			&["    something entirely different here", "    ripgrep-all"],
		);
		assert!(added[0].is_empty());
		assert_eq!(emphasized("    ripgrep-all", &added[1]), ["-all"]);
		assert!(removed[0].is_empty(), "nothing was removed from ripgrep");
	}

	#[test]
	fn oversized_blocks_and_lines_are_not_aligned() {
		let long = "x".repeat(600);
		let (removed, added) = emphasis(&[long.as_str()], &["x"]);
		assert_eq!((removed[0].len(), added[0].len()), (0, 0));
		let many = vec!["a"; 100];
		let (removed, _) = emphasis(&many, &many);
		assert!(removed.iter().all(Vec::is_empty));
		let punctuation = ". ".repeat(250);
		let dense = vec![punctuation.as_str(); 4];
		let (removed, added) = emphasis(&dense, &dense);
		assert!(removed.iter().chain(&added).all(Vec::is_empty));
	}

	#[test]
	fn unicode_changes_produce_valid_byte_ranges() {
		let minus = "  label = \"café ☕\";";
		let plus = "  label = \"茶 ☕\";";
		let (removed, added) = emphasis(&[minus], &[plus]);
		assert_eq!(emphasized(minus, &removed[0]), ["café"]);
		assert_eq!(emphasized(plus, &added[0]), ["茶"]);
		assert_eq!(emphasis(&[""], &[""]), (vec![vec![]], vec![vec![]]));
	}
}
