use std::path::PathBuf;

use clap::Args;
use solex_core::{
    CoreError, Result,
    scan::{self, Cell},
};

use crate::{commands, style};

#[derive(Args, Debug, Default)]
pub struct FeaturesCommand {
    /// Only show this channel's column
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Emit JSON
    #[arg(long)]
    pub json: bool,

    /// Exit non-zero if any attribute names a tag no channel defines
    #[arg(long)]
    pub check: bool,

    /// Path to the manifest
    #[arg(long)]
    pub manifest: Option<PathBuf>,
}

impl FeaturesCommand {
    pub fn execute(&self) -> Result<()> {
        let project = commands::load_project(self.manifest.as_deref())?;
        let m = project.channels();
        let channels = match &self.channel {
            Some(c) => {
                m.channel(c)?;
                vec![c.clone()]
            }
            None => m.names(),
        };
        let items = scan::scan_program(&project.program_dir())?;
        let unknown = scan::unknown_tags(m, &items);
        let matrix = scan::matrix(m, items, &channels)?;

        if self.json {
            let rows: Vec<_> = matrix
                .rows
                .iter()
                .map(|(item, cells)| {
                    let per: serde_json::Map<String, serde_json::Value> = channels
                        .iter()
                        .zip(cells)
                        .map(|(ch, cell)| {
                            let v = match cell {
                                Cell::Excluded => serde_json::json!({"ships": false}),
                                Cell::Included => serde_json::json!({"ships": true}),
                                Cell::IncludedFrom(e) => {
                                    serde_json::json!({"ships": true, "from_epoch": e})
                                }
                            };
                            (ch.clone(), v)
                        })
                        .collect();
                    serde_json::json!({
                        "file": item.file,
                        "line": item.line,
                        "kind": item.kind,
                        "name": item.name,
                        "channel_attr": item.spec_text,
                        "channels": per,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&rows).unwrap_or_default()
            );
        } else {
            print_table(&matrix, &channels);
        }

        if !unknown.is_empty() {
            eprintln!();
            for (tag, item) in &unknown {
                eprintln!(
                    "  {} `{tag}` in #[channel] on {} {} ({}:{}) is not a channel; this item \
                     ships nowhere",
                    style::warn(),
                    item.kind,
                    style::bold(&item.name),
                    item.file.display(),
                    item.line
                );
            }
            if self.check {
                return Err(CoreError::message(format!(
                    "{} unknown feature tag(s)",
                    unknown.len()
                )));
            }
        }
        Ok(())
    }
}

fn print_table(matrix: &scan::Matrix, channels: &[String]) {
    if matrix.rows.is_empty() {
        println!("  {}", style::dim("no #[channel] attributes found"));
        return;
    }
    let name_w = matrix
        .rows
        .iter()
        .map(|(i, _)| i.name.len() + i.kind.len() + 1)
        .max()
        .unwrap_or(4)
        .max(4);
    let spec_w = matrix
        .rows
        .iter()
        .map(|(i, _)| i.spec_text.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let col_w = channels.iter().map(|c| c.len()).max().unwrap_or(6).max(8);

    println!();
    print!(
        "  {}  {} ",
        style::pad_then("item", name_w, style::bold),
        style::pad_then("#[channel]", spec_w, style::bold)
    );
    for ch in channels {
        let left = col_w.saturating_sub(ch.len()) / 2;
        let right = col_w.saturating_sub(ch.len() + left);
        print!(
            " {}{}{}",
            " ".repeat(left),
            style::accent(ch),
            " ".repeat(right)
        );
    }
    println!();
    print!("  {}  {} ", "─".repeat(name_w), "─".repeat(spec_w));
    for _ in channels {
        print!(" {}", "─".repeat(col_w));
    }
    println!();
    for (item, cells) in &matrix.rows {
        let label = format!("{} {}", style::dim(item.kind), item.name);
        // `label` carries ANSI codes, so pad by visible width.
        let pad = name_w.saturating_sub(item.kind.len() + 1 + item.name.len());
        print!("  {label}{}  {:<spec_w$} ", " ".repeat(pad), item.spec_text);
        for cell in cells {
            let text = match cell {
                Cell::Excluded => style::dim("–"),
                Cell::Included => style::green("✓"),
                Cell::IncludedFrom(e) => style::yellow(&format!("✓@{e}")),
            };
            let visible = match cell {
                Cell::Excluded | Cell::Included => 1,
                Cell::IncludedFrom(e) => 2 + e.to_string().len(),
            };
            let left = (col_w.saturating_sub(visible)) / 2;
            let right = col_w.saturating_sub(visible + left);
            print!(" {}{text}{}", " ".repeat(left), " ".repeat(right));
        }
        println!(
            "   {}",
            style::dim(&format!("{}:{}", item.file.display(), item.line))
        );
    }
    println!();
    println!(
        "  {}  {}  {}",
        style::green("✓ ships"),
        style::yellow("✓@N ships, active from epoch N"),
        style::dim("– not compiled")
    );
    println!();
}
