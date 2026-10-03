//! Display layout: tab expansion and soft wrapping.
//!
//! The editor stores positions in **characters** within a line. Rendering (and
//! wrap-aware cursor movement) needs a second coordinate: the **display
//! column**, where a tab occupies several cells and a soft wrap splits one
//! logical line into several visual rows.
//!
//! [`LineLayout`] owns the character ↔ display-column mapping for a single
//! line; [`wrap_segments`] turns it into the display ranges of each visual row.
//! Both the renderer and the cursor movement code use these, so the two can
//! never disagree about where a wrapped line breaks.

/// A line expanded for display, with a mapping back to character indices.
pub struct LineLayout {
    /// One entry per display cell: the character shown and its character index.
    cells: Vec<(char, usize)>,
    /// `map[character_index]` is the display column where that character begins.
    /// The final entry is one past the last character (the line's display width).
    map: Vec<usize>,
    len: usize,
}

impl LineLayout {
    /// Expand `text` for display, treating `\t` as advancing to the next
    /// multiple of `tab_width`.
    pub fn new(text: &str, tab_width: usize) -> Self {
        let tab_width = tab_width.max(1);
        let mut cells = Vec::with_capacity(text.len());
        let mut map = Vec::with_capacity(text.chars().count() + 1);
        let mut column = 0usize;

        for (index, ch) in text.chars().enumerate() {
            map.push(column);
            if ch == '\t' {
                let spaces = tab_width - (column % tab_width);
                for _ in 0..spaces {
                    cells.push((' ', index));
                    column += 1;
                }
            } else {
                cells.push((ch, index));
                column += 1;
            }
        }
        map.push(column);
        LineLayout {
            cells,
            map,
            len: column,
        }
    }

    /// The width of the line in display columns.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The number of characters in the original line.
    pub fn char_count(&self) -> usize {
        self.map.len().saturating_sub(1)
    }

    /// The display column where the character at `original` begins.
    pub fn display_col(&self, original: usize) -> usize {
        self.map.get(original).copied().unwrap_or(self.len)
    }

    /// The character index at `display`, clamped to the line.
    ///
    /// A display column inside an expanded tab maps to the tab character, so a
    /// cursor never lands between the spaces of one tab.
    pub fn char_col(&self, display: usize) -> usize {
        if self.map.is_empty() {
            return 0;
        }
        let count = self.char_count();
        if display >= self.len {
            return count;
        }
        // The last index whose start column is <= display.
        let index = self.map.partition_point(|&start| start <= display);
        index.saturating_sub(1).min(count)
    }

    /// The cell at `display`, if the column exists.
    pub fn cell(&self, display: usize) -> Option<(char, usize)> {
        self.cells.get(display).copied()
    }
}

/// Split a line into the display ranges of its visual rows.
///
/// When the line fits (or `width` is zero) there is a single segment covering
/// the whole line. Otherwise the line is broken greedily at whitespace when one
/// is available within the row, and mid-word only for a run longer than the
/// width, so progress is always made.
pub fn wrap_segments(layout: &LineLayout, width: usize) -> Vec<(usize, usize)> {
    let len = layout.len();
    if width == 0 || len <= width {
        // A zero-width line still occupies one (empty) visual row.
        return vec![(0, len)];
    }

    let mut segments = Vec::new();
    let mut start = 0;
    while start < len {
        let hard_end = (start + width).min(len);
        let mut end = hard_end;
        if hard_end < len {
            // Prefer a whitespace at the boundary, then the last one inside the
            // row, so the space stays on the previous visual row and the next
            // one starts at a word.
            if layout
                .cell(hard_end)
                .is_some_and(|(ch, _)| ch.is_whitespace())
            {
                end = hard_end + 1;
            } else {
                let mut cell = hard_end;
                while cell > start + 1 {
                    if layout
                        .cell(cell - 1)
                        .is_some_and(|(ch, _)| ch.is_whitespace())
                    {
                        end = cell;
                        break;
                    }
                    cell -= 1;
                }
            }
        }
        if end <= start {
            end = (start + 1).min(len);
        }
        segments.push((start, end));
        start = end;
    }
    segments
}

/// The index of the visual row that contains display column `display`.
pub fn segment_index(segments: &[(usize, usize)], display: usize) -> usize {
    segments
        .iter()
        .position(|&(start, end)| display >= start && display < end)
        .unwrap_or_else(|| segments.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_tabs_to_display_columns() {
        let layout = LineLayout::new("a\tb", 4);
        assert_eq!(layout.len(), 5);
        assert_eq!(layout.display_col(0), 0);
        assert_eq!(layout.display_col(1), 1); // tab starts at column 1
        assert_eq!(layout.display_col(2), 4); // b after the tab stops
        assert_eq!(layout.char_col(3), 1); // inside the tab expansion
        assert_eq!(layout.char_col(4), 2);
    }

    #[test]
    fn char_and_display_roundtrip() {
        let layout = LineLayout::new("ab\tcd", 4);
        for original in 0..=layout.char_count() {
            let display = layout.display_col(original);
            assert_eq!(
                layout.char_col(display),
                original,
                "roundtrip at {original}"
            );
        }
    }

    #[test]
    fn short_lines_are_one_segment() {
        let layout = LineLayout::new("hello", 4);
        assert_eq!(wrap_segments(&layout, 10), vec![(0, 5)]);
        assert_eq!(wrap_segments(&layout, 0), vec![(0, 5)]);
        assert_eq!(wrap_segments(&LineLayout::new("", 4), 10), vec![(0, 0)]);
    }

    #[test]
    fn wraps_at_whitespace_when_possible() {
        let layout = LineLayout::new("hello world again", 4);
        // Width 12: "hello world " (12) then "again".
        assert_eq!(wrap_segments(&layout, 12), vec![(0, 12), (12, 17)]);
    }

    #[test]
    fn breaks_after_a_space_at_the_boundary() {
        let layout = LineLayout::new("abcdefghij klmno", 4);
        // The space sits exactly at column 10, so it ends the first row.
        assert_eq!(wrap_segments(&layout, 10), vec![(0, 11), (11, 16)]);
    }

    #[test]
    fn breaks_long_words_at_the_width() {
        let layout = LineLayout::new("abcdefghij", 4);
        assert_eq!(wrap_segments(&layout, 4), vec![(0, 4), (4, 8), (8, 10)]);
    }

    #[test]
    fn never_makes_a_zero_width_segment() {
        let layout = LineLayout::new("a b", 4);
        let segments = wrap_segments(&layout, 1);
        assert!(segments.iter().all(|&(start, end)| end > start));
        assert_eq!(segments.first().unwrap().0, 0);
        assert_eq!(segments.last().unwrap().1, 3);
        // Every display column is covered exactly once.
        for column in 0..layout.len() {
            let hits = segments
                .iter()
                .filter(|&&(start, end)| column >= start && column < end)
                .count();
            assert_eq!(hits, 1, "column {column} covered once");
        }
    }

    #[test]
    fn segment_index_picks_the_containing_row() {
        let segments = vec![(0, 4), (4, 8), (8, 10)];
        assert_eq!(segment_index(&segments, 0), 0);
        assert_eq!(segment_index(&segments, 3), 0);
        assert_eq!(segment_index(&segments, 4), 1);
        assert_eq!(segment_index(&segments, 9), 2);
        // Past the end clamps to the last visual row.
        assert_eq!(segment_index(&segments, 99), 2);
    }
}
