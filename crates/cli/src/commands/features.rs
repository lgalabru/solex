use std::path::PathBuf;

use clap::Args;
use solex_core::{
    Channels, CoreError, Result,
    scan::{self, Cell},
};

use crate::{commands, style};

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

fn cell(c: &Cell) -> String {
    match c {
        Cell::Excluded => "-".into(),
        Cell::Included => "✓".into(),
        Cell::IncludedFrom(e) => format!("✓@{e}"),
    }
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

/// One row per feature (or per distinct channel gate for unnamed markers).
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
        println!("no gated items");
        return;
    }
    let mut headers = vec!["feature", "channels", "gates"];
    headers.extend(channels.iter().map(String::as_str));
    let mut align = vec![style::Align::Left, style::Align::Left, style::Align::Right];
    align.extend(channels.iter().map(|_| style::Align::Center));
    let rows: Vec<Vec<String>> = groups
        .iter()
        .map(|g| {
            let where_ = match (g.gate, g.feature) {
                (scan::Gate::Cfg(_), Some(f)) => config
                    .features
                    .get(f)
                    .map(|feat| feat.channels.join(", "))
                    .unwrap_or_else(|| g.spec.to_string()),
                (scan::Gate::Channel(_), _) => g.spec.to_string(),
                (scan::Gate::Cfg(_), None) => format!("cfg({})", g.spec),
            };
            let mut row = vec![
                g.feature.unwrap_or("-").to_string(),
                where_,
                g.count.to_string(),
            ];
            row.extend(g.cells.iter().map(cell));
            row
        })
        .collect();
    print!("{}", style::table(&headers, &rows, &align));
}

/// Every marker, with its location.
fn print_items(rows: &[&(scan::GatedItem, Vec<Cell>)], channels: &[String], show_gate: bool) {
    let mut headers = vec!["item"];
    if show_gate {
        headers.push("gate");
    }
    headers.extend(channels.iter().map(String::as_str));
    headers.push("location");
    let mut align = vec![style::Align::Left];
    if show_gate {
        align.push(style::Align::Left);
    }
    align.extend(channels.iter().map(|_| style::Align::Center));
    align.push(style::Align::Left);
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|(item, cells)| {
            let mut row = vec![format!("{} {}", item.kind, item.name)];
            if show_gate {
                row.push(item.gate_text.clone());
            }
            row.extend(cells.iter().map(cell));
            row.push(format!("{}:{}", item.file.display(), item.line));
            row
        })
        .collect();
    print!("{}", style::table(&headers, &table_rows, &align));
}
