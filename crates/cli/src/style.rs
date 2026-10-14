//! Minimal terminal styling. Colors are disabled with `--no-color` or when
//! `NO_COLOR` is set; everything degrades to plain text.

use std::sync::atomic::{AtomicBool, Ordering};

use console::{Style, style};

static COLOR: AtomicBool = AtomicBool::new(true);

pub fn init(color: bool) {
    let enabled = color && std::env::var_os("NO_COLOR").is_none();
    COLOR.store(enabled, Ordering::Relaxed);
    console::set_colors_enabled(enabled);
    console::set_colors_enabled_stderr(enabled);
}

pub fn ok() -> String {
    style("✓").green().bold().to_string()
}

pub fn fail() -> String {
    style("✗").red().bold().to_string()
}

pub fn warn() -> String {
    style("!").yellow().bold().to_string()
}

pub fn bold(s: &str) -> String {
    style(s).bold().to_string()
}

pub fn dim(s: &str) -> String {
    style(s).dim().to_string()
}

pub fn green(s: &str) -> String {
    style(s).green().to_string()
}

pub fn yellow(s: &str) -> String {
    style(s).yellow().to_string()
}

pub fn accent(s: &str) -> String {
    Style::new().cyan().apply_to(s).to_string()
}

pub fn spinner(msg: &str) -> indicatif::ProgressBar {
    if !COLOR.load(Ordering::Relaxed) || !console::user_attended_stderr() {
        eprintln!("  {msg}");
        return indicatif::ProgressBar::hidden();
    }
    let pb = indicatif::ProgressBar::new_spinner();
    pb.set_style(
        indicatif::ProgressStyle::with_template("  {spinner:.cyan} {msg}")
            .expect("static template"),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= KB * KB {
        format!("{:.2} MiB", b / (KB * KB))
    } else if b >= KB {
        format!("{:.1} KiB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

pub fn human_duration(d: std::time::Duration) -> String {
    let s = d.as_secs_f64();
    if s >= 60.0 {
        format!("{}m {:.0}s", (s / 60.0) as u64, s % 60.0)
    } else if s >= 1.0 {
        format!("{s:.1}s")
    } else {
        format!("{}ms", d.as_millis())
    }
}

/// Left-align `s` to `width` visible columns, then apply `f`. Padding after
/// styling would count escape codes as characters.
pub fn pad_then(s: &str, width: usize, f: fn(&str) -> String) -> String {
    f(&format!("{s:<width$}"))
}

#[derive(Clone, Copy)]
pub enum Align {
    Left,
    Center,
}

/// Plain `+---+` table, no colors inside so widths stay honest.
pub fn table(headers: &[&str], rows: &[Vec<String>], align: &[Align]) -> String {
    let width = |s: &str| s.chars().count();
    let mut widths: Vec<usize> = headers.iter().map(|h| width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            widths[i] = widths[i].max(width(cell));
        }
    }
    let rule = {
        let mut r = String::from("+");
        for w in &widths {
            r.push_str(&"-".repeat(w + 2));
            r.push('+');
        }
        r
    };
    let line = |cells: &[String]| {
        let mut l = String::from("|");
        for (i, cell) in cells.iter().enumerate() {
            let pad = widths[i].saturating_sub(width(cell));
            let (left, right) = match align.get(i).copied().unwrap_or(Align::Left) {
                Align::Left => (0, pad),
                Align::Center => (pad / 2, pad - pad / 2),
            };
            l.push(' ');
            l.push_str(&" ".repeat(left));
            l.push_str(cell);
            l.push_str(&" ".repeat(right));
            l.push_str(" |");
        }
        l
    };
    let mut out = String::new();
    out.push_str(&rule);
    out.push('\n');
    out.push_str(&line(
        &headers.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
    ));
    out.push('\n');
    out.push_str(&rule);
    out.push('\n');
    for row in rows {
        out.push_str(&line(row));
        out.push('\n');
    }
    out.push_str(&rule);
    out.push('\n');
    out
}
