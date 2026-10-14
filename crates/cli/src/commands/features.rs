use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Channels, CoreError, Result,
    scan::{self, Cell},
};

use crate::{boxed, commands, style};

#[derive(Args, Debug, Default)]
pub struct FeaturesCommand {
    /// Show every marker of one feature, with file and line
    #[arg(value_name = "FEATURE")]
    pub feature: Option<String>,

    /// Only show this channel's column
    #[arg(long, short)]
    pub channel: Option<String>,

    /// Emit JSON
    #[arg(long)]
    pub json: bool,

    /// Exit non-zero if any attribute names a tag no channel defines
    #[arg(long)]
    pub check: bool,

    /// List every #[channel] marker with its file and line instead of one
    /// row per gate
    #[arg(long)]
    pub items: bool,

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
        let scan = scan::scan_program(&project.program_dir(), m)?;
        let unknown = scan::unknown_tags(m, &scan.items);
        let unused = scan::unused_features(m, &scan.items);
        let undeclared =
            scan::undeclared_in_cargo(m, &project.program_manifest()).unwrap_or_default();
        let untracked = scan::untracked_env(&project.program_dir());
        let matrix = scan::matrix(m, scan.items, &channels)?;

        if self.json {
            let rows: Vec<_> = matrix
                .rows
                .iter()
                .map(|(item, cells)| {
                    let per: serde_json::Map<String, serde_json::Value> = channels
                        .iter()
                        .zip(cells)
                        .map(|(ch, c)| {
                            let v = match c {
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
                        "file": item.file, "line": item.line, "kind": item.kind,
                        "name": item.name, "feature": item.feature, "gate": item.gate_text,
                        "channels": per,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&rows).unwrap_or_default()
            );
        } else if let Some(name) = &self.feature {
            let rows: Vec<&(scan::GatedItem, Vec<Cell>)> = matrix
                .rows
                .iter()
                .filter(|(i, _)| i.feature.as_deref() == Some(name) || i.gate_text == *name)
                .collect();
            if rows.is_empty() {
                return Err(CoreError::message(format!(
                    "no gated items for `{name}`; features: {}",
                    m.features.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
            print_items(&rows, &channels, false);
        } else if self.items {
            let rows: Vec<_> = matrix.rows.iter().collect();
            print_items(&rows, &channels, true);
        } else {
            print_features(m, &matrix, &channels);
        }

        if self.check {
            let mut problems = 0;
            for (tag, item) in &unknown {
                eprintln!(
                    "{} `{tag}` on {} {} ({}:{}) is not a channel; the item ships nowhere",
                    style::fail(),
                    item.kind,
                    item.name,
                    item.file.display(),
                    item.line
                );
                problems += 1;
            }
            for f in &undeclared {
                eprintln!(
                    "{} feature `{f}` is in the manifest but not in {} [features]",
                    style::fail(),
                    project.program_manifest().display()
                );
                problems += 1;
            }
            for f in &unused {
                eprintln!(
                    "{} feature `{f}` is declared but no #[cfg(feature = \"{f}\")] uses it",
                    style::fail()
                );
                problems += 1;
            }
            if !untracked.is_empty() {
                eprintln!(
                    "{} build.rs does not declare rerun-if-env-changed for {}",
                    style::fail(),
                    untracked.join(", ")
                );
                problems += 1;
            }
            if problems > 0 {
                return Err(CoreError::message(format!(
                    "{problems} problem(s) in channel gating"
                )));
            }
        }
        Ok(())
    }
}

fn status(c: &Cell) -> boxed::Line {
    match c {
        Cell::Excluded => (style::dim("-"), 1),
        Cell::Included => (style::green("✓"), 1),
        Cell::IncludedFrom(e) => {
            let t = format!("✓ from epoch {e}");
            let w = t.chars().count();
            (style::yellow(&t), w)
        }
    }
}

/// One section per feature (or per distinct channel gate for unnamed
/// markers): name, where it ships, description, and a status per channel.
fn print_features(config: &Channels, matrix: &scan::Matrix, channels: &[String]) {
    struct Group<'a> {
        feature: Option<&'a str>,
        spec: &'a str,
        gate: &'a scan::Gate,
        cells: &'a Vec<Cell>,
        count: usize,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (item, cells) in &matrix.rows {
        match groups
            .iter_mut()
            .find(|g| g.feature == item.feature.as_deref() && g.spec == item.gate_text)
        {
            Some(g) => g.count += 1,
            None => groups.push(Group {
                feature: item.feature.as_deref(),
                spec: &item.gate_text,
                gate: &item.gate,
                cells,
                count: 1,
            }),
        }
    }
    if groups.is_empty() {
        println!("  {}", style::dim("no gated items"));
        return;
    }
    let sections: Vec<Vec<boxed::Line>> = groups
        .iter()
        .map(|g| {
            let mut lines = Vec::new();
            let name = match (g.gate, g.feature) {
                (_, Some(f)) => f.to_string(),
                (scan::Gate::Channel(_), None) => format!("#[channel({})]", g.spec),
                (scan::Gate::Cfg(_), None) => format!("cfg({})", g.spec),
            };
            let where_ = match (g.gate, g.feature) {
                (scan::Gate::Cfg(_), Some(f)) => config
                    .features
                    .get(f)
                    .map(|feat| feat.channels.join(", "))
                    .unwrap_or_default(),
                _ => String::new(),
            };
            let mut head = vec![(style::bold(&name), name.chars().count())];
            if !where_.is_empty() {
                head.push(boxed::plain(where_));
            }
            let gates = format!("{} gate{}", g.count, if g.count == 1 { "" } else { "s" });
            let gw = gates.chars().count();
            lines.push(boxed::split(boxed::join(&head), (style::dim(&gates), gw)));
            if let Some(d) = g
                .feature
                .and_then(|f| config.features.get(f))
                .and_then(|f| f.description.as_deref())
            {
                for seg in boxed::wrap(d, boxed::INNER) {
                    let w = seg.chars().count();
                    lines.push((style::dim(&seg), w));
                }
            }
            let statuses: Vec<boxed::Line> = channels
                .iter()
                .zip(g.cells.iter())
                .map(|(ch, c)| {
                    let (s, w) = status(c);
                    (format!("{ch} {s}"), ch.chars().count() + 1 + w)
                })
                .collect();
            lines.push(boxed::join(&statuses));
            lines
        })
        .collect();
    println!("{}", boxed::frame(&sections));
}

/// Every marker, with its location. One header section with the status per
/// channel (identical for a single feature), then one line per marker.
fn print_items(rows: &[&(scan::GatedItem, Vec<Cell>)], channels: &[String], show_gate: bool) {
    let mut sections: Vec<Vec<boxed::Line>> = Vec::new();
    if !show_gate {
        if let Some((first, cells)) = rows.first() {
            let mut head = vec![(
                style::bold(first.feature.as_deref().unwrap_or(&first.gate_text)),
                first
                    .feature
                    .as_deref()
                    .unwrap_or(&first.gate_text)
                    .chars()
                    .count(),
            )];
            let statuses: Vec<boxed::Line> = channels
                .iter()
                .zip(cells.iter())
                .map(|(ch, c)| {
                    let (s, w) = status(c);
                    (format!("{ch} {s}"), ch.chars().count() + 1 + w)
                })
                .collect();
            head.push(boxed::join(&statuses));
            let gates = format!(
                "{} gate{}",
                rows.len(),
                if rows.len() == 1 { "" } else { "s" }
            );
            let gw = gates.chars().count();
            sections.push(vec![boxed::split(
                boxed::join(&head),
                (style::dim(&gates), gw),
            )]);
        }
    }
    let lines: Vec<boxed::Line> = rows
        .iter()
        .map(|(item, cells)| {
            let mut left = vec![(
                format!("{} {}", style::dim(item.kind), item.name),
                item.kind.chars().count() + 1 + item.name.chars().count(),
            )];
            if show_gate {
                let statuses: Vec<boxed::Line> = channels
                    .iter()
                    .zip(cells.iter())
                    .map(|(ch, c)| {
                        let (s, w) = status(c);
                        (format!("{ch} {s}"), ch.chars().count() + 1 + w)
                    })
                    .collect();
                left.push(boxed::join(&statuses));
            }
            let loc = format!("{}:{}", item.file.display(), item.line);
            let lw = loc.chars().count();
            boxed::split(boxed::join(&left), (style::dim(&loc), lw))
        })
        .collect();
    sections.push(lines);
    println!("{}", boxed::frame(&sections));
}
