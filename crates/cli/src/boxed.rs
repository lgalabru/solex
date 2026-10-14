//! 80-column bordered box, the same shape `pay skills ls` uses: a `┌─┐ │ │ └─┘`
//! frame indented two spaces, `├───┤` rules between sections, and every line
//! pre-rendered as `(content, visible_width)` so colors never break the gutter.

use crate::style;

/// Inner content width. Box width is `INNER + 4`; with the indent, 80 columns.
pub const INNER: usize = 74;

pub type Line = (String, usize);

pub fn frame(sections: &[Vec<Line>]) -> String {
    let bar = style::dim("│");
    let rule = |l: char, r: char| style::dim(&format!("{l}{}{r}", "─".repeat(INNER + 2)));
    let mut out = vec![rule('┌', '┐')];
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push(rule('├', '┤'));
        }
        for (content, visible) in section {
            let pad = " ".repeat(INNER.saturating_sub((*visible).min(INNER)));
            out.push(format!("{bar} {content}{pad} {bar}"));
        }
    }
    out.push(rule('└', '┘'));
    out.iter()
        .map(|l| format!("  {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// A plain line: no styling, width is its char count.
pub fn plain(s: impl Into<String>) -> Line {
    let s = s.into();
    let w = s.chars().count();
    (s, w)
}

/// Left part and right-aligned part on one line.
pub fn split(left: Line, right: Line) -> Line {
    let gap = INNER.saturating_sub(left.1 + right.1).max(2);
    (
        format!("{}{}{}", left.0, " ".repeat(gap), right.0),
        left.1 + gap + right.1,
    )
}

/// Join styled fragments with two spaces.
pub fn join(parts: &[Line]) -> Line {
    let mut s = String::new();
    let mut w = 0;
    for (i, (c, v)) in parts.iter().enumerate() {
        if i > 0 {
            s.push_str("  ");
            w += 2;
        }
        s.push_str(c);
        w += v;
    }
    (s, w)
}

/// `label` padded to a fixed column, then `value`.
pub fn field(label: &str, value: Line) -> Line {
    const LABEL: usize = 9;
    let l = format!("{label:<LABEL$}");
    (
        format!("{} {}", style::dim(&l), value.0),
        LABEL + 1 + value.1,
    )
}

/// Greedy word-wrap to `width` columns.
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in s.split_whitespace() {
        let need = if cur.is_empty() {
            word.chars().count()
        } else {
            cur.chars().count() + 1 + word.chars().count()
        };
        if need > width && !cur.is_empty() {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(word);
    }
    if !cur.is_empty() || lines.is_empty() {
        lines.push(cur);
    }
    lines
}

/// `CHNLxYvV…hyGsX`: keep both ends of a long identifier.
pub fn short(id: &str) -> String {
    let n = id.chars().count();
    if n <= 13 {
        return id.to_string();
    }
    let head: String = id.chars().take(4).collect();
    let tail: String = id.chars().skip(n - 4).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_keeps_ends() {
        assert_eq!(
            short("CHNLxYvVA28MJP9PrFuDXccuoGXAx7jBacfLEkahyGsX"),
            "CHNL…yGsX"
        );
        assert_eq!(short("devnet"), "devnet");
    }

    #[test]
    fn frame_is_80_columns() {
        let out = frame(&[vec![plain("a")], vec![plain("b")]]);
        for line in out.lines() {
            let visible: String = console::strip_ansi_codes(line).into();
            assert_eq!(visible.chars().count(), 80, "{visible}");
        }
    }
}
