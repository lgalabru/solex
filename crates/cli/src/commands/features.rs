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
        let unmanaged = scan.unmanaged_features.clone();
        let matrix = scan::matrix(m, scan.items, &channels)?;

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
                        "feature": item.feature,
                        "gate": item.gate_text,
                        "channels": per,
                    })
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(&rows).unwrap_or_default()
            );
        } else if let Some(name) = &self.feature {
            let rows: Vec<_> = matrix
                .rows
                .iter()
                .filter(|(i, _)| i.feature.as_deref() == Some(name) || i.gate_text == *name)
                .cloned()
                .collect();
            if rows.is_empty() {
                return Err(CoreError::message(format!(
                    "no gated items for `{name}`; features: {}",
                    m.features.keys().cloned().collect::<Vec<_>>().join(", ")
                )));
            }
            if let Some(desc) = m.features.get(name).and_then(|f| f.description.as_deref()) {
                println!();
                println!("  {}  {}", style::bold(name), style::dim(desc));
            }
            print_table(
                &scan::Matrix {
                    channels: matrix.channels.clone(),
                    rows,
                },
                &channels,
                false,
            );
        } else if self.items {
            print_table(&matrix, &channels, true);
        } else {
            print_gates(&project.name(), m, &matrix, &channels);
        }

        let mut problems = 0;
        for f in &undeclared {
            eprintln!(
                "  {} feature `{f}` is in the manifest but not in {} [features]; builds \
                 enabling it will fail",
                style::fail(),
                project.program_manifest().display()
            );
            problems += 1;
        }
        for f in &unused {
            eprintln!(
                "  {} feature `{f}` is declared in the manifest but no #[cfg(feature = \"{f}\")] \
                 uses it",
                style::warn(),
                f = f
            );
            problems += 1;
        }
        let untracked = scan::untracked_env(&project.program_dir());
        if !untracked.is_empty() {
            eprintln!(
                "  {} build.rs does not declare rerun-if-env-changed for {}; cargo may reuse \
                 another channel's compilation",
                style::fail(),
                untracked.join(", ")
            );
            problems += 1;
        }
        if !unmanaged.is_empty() {
            eprintln!(
                "  {} cargo features not managed by solex (never enabled by channel builds): {}",
                style::dim("i"),
                unmanaged.iter().cloned().collect::<Vec<_>>().join(", ")
            );
        }

        if !unknown.is_empty() {
            eprintln!();
            for (tag, item) in &unknown {
                eprintln!(
                    "  {} `{tag}` on {} {} ({}:{}) is neither a channel nor a declared \
                     feature; this item ships nowhere",
                    style::warn(),
                    item.kind,
                    style::bold(&item.name),
                    item.file.display(),
                    item.line
                );
            }
        }
        if self.check && (problems > 0 || !unknown.is_empty()) {
            return Err(CoreError::message(format!(
                "{} problem(s) in channel gating",
                problems + unknown.len()
            )));
        }
        Ok(())
    }
}

fn print_table(matrix: &scan::Matrix, channels: &[String], show_gate: bool) {
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
        .map(|(i, _)| i.gate_text.len())
        .max()
        .unwrap_or(10)
        .max(10);
    let col_w = channels.iter().map(|c| c.len()).max().unwrap_or(6).max(8);

    println!();
    print!("  {}", style::pad_then("item", name_w, style::bold));
    if show_gate {
        print!("  {}", style::pad_then("gate", spec_w, style::bold));
    }
    print!(" ");
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
    print!("  {}", "─".repeat(name_w));
    if show_gate {
        print!("  {}", "─".repeat(spec_w));
    }
    print!(" ");
    for _ in channels {
        print!(" {}", "─".repeat(col_w));
    }
    println!();
    for (item, cells) in &matrix.rows {
        let label = format!("{} {}", style::dim(item.kind), item.name);
        // `label` carries ANSI codes, so pad by visible width.
        let pad = name_w.saturating_sub(item.kind.len() + 1 + item.name.len());
        print!("  {label}{}", " ".repeat(pad));
        if show_gate {
            print!("  {:<spec_w$}", item.gate_text);
        }
        print!(" ");
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

/// Default view: one row per feature (the quoted name in the attribute), or
/// per distinct channel attribute for unnamed markers, so the table answers
/// "what ships where" rather than listing dispatch plumbing.
fn print_gates(name: &str, config: &Channels, matrix: &scan::Matrix, channels: &[String]) {
    if matrix.rows.is_empty() {
        println!("  {}", style::dim("no #[channel] attributes found"));
        return;
    }
    struct Group<'a> {
        feature: Option<&'a str>,
        spec: &'a str,
        cells: &'a Vec<Cell>,
        items: Vec<&'a scan::GatedItem>,
    }
    let mut groups: Vec<Group> = Vec::new();
    for (item, cells) in &matrix.rows {
        let same = groups
            .iter_mut()
            .find(|g| g.feature == item.feature.as_deref() && g.spec == item.gate_text);
        match same {
            Some(g) => g.items.push(item),
            None => groups.push(Group {
                feature: item.feature.as_deref(),
                spec: item.gate_text.as_str(),
                cells,
                items: vec![item],
            }),
        }
    }
    let files: std::collections::BTreeSet<&std::path::Path> =
        matrix.rows.iter().map(|(i, _)| i.file.as_path()).collect();

    println!();
    println!(
        "  {}  {}",
        style::bold(name),
        style::dim(&format!(
            "{} gated item{} in {} file{}, {} channel{}",
            matrix.rows.len(),
            if matrix.rows.len() == 1 { "" } else { "s" },
            files.len(),
            if files.len() == 1 { "" } else { "s" },
            channels.len(),
            if channels.len() == 1 { "" } else { "s" },
        ))
    );
    println!();

    let feat_w = groups
        .iter()
        .map(|g| g.feature.map(str::len).unwrap_or(1))
        .max()
        .unwrap_or(7)
        .max(7);
    let gate_w = groups
        .iter()
        .map(|g| match g.feature.and_then(|f| config.features.get(f)) {
            Some(feat) => feat.channels.join(", ").len(),
            None => g.spec.len() + "cfg()".len(),
        })
        .max()
        .unwrap_or(8)
        .max("channels".len());
    let col_w = channels.iter().map(|c| c.len()).max().unwrap_or(6).max(8);

    print!(
        "  {}  {}  {}",
        style::pad_then("feature", feat_w, style::bold),
        style::pad_then("channels", gate_w, style::bold),
        style::bold("gates")
    );
    for ch in channels {
        let left = col_w.saturating_sub(ch.len()) / 2;
        let right = col_w.saturating_sub(ch.len() + left);
        print!(
            "  {}{}{}",
            " ".repeat(left),
            style::accent(ch),
            " ".repeat(right)
        );
    }
    println!();
    print!(
        "  {}  {}  {}",
        "─".repeat(feat_w),
        "─".repeat(gate_w),
        "─".repeat(5)
    );
    for _ in channels {
        print!("  {}", "─".repeat(col_w));
    }
    println!();

    for g in &groups {
        let feature = g.feature.unwrap_or("–");
        // Where the feature ships, as declared: the manifest's channel list
        // for a feature, the tags for a #[channel] gate, the predicate for
        // anything more involved.
        let label = match (&g.items[0].gate, g.feature) {
            (scan::Gate::Cfg(_), Some(f)) => config
                .features
                .get(f)
                .map(|feat| feat.channels.join(", "))
                .unwrap_or_else(|| g.spec.to_string()),
            (scan::Gate::Channel(_), _) => g.spec.to_string(),
            (scan::Gate::Cfg(_), None) => format!("cfg({})", g.spec),
        };
        print!(
            "  {}  {label:<gate_w$}  {:>5}",
            style::pad_then(feature, feat_w, style::bold),
            g.items.len()
        );
        for cell in g.cells.iter() {
            let (text, visible) = match cell {
                Cell::Excluded => (style::dim("–"), 1),
                Cell::Included => (style::green("✓"), 1),
                Cell::IncludedFrom(e) => {
                    (style::yellow(&format!("✓@{e}")), 2 + e.to_string().len())
                }
            };
            let left = col_w.saturating_sub(visible) / 2;
            let right = col_w.saturating_sub(visible + left);
            print!("  {}{text}{}", " ".repeat(left), " ".repeat(right));
        }
        println!();
    }

    // One feature, several gates: usually one marker was left behind.
    let mut seen: Vec<&str> = Vec::new();
    for g in &groups {
        let Some(f) = g.feature else { continue };
        if seen.contains(&f) {
            continue;
        }
        seen.push(f);
        let specs: Vec<String> = groups
            .iter()
            .filter(|o| o.feature == Some(f))
            .map(|o| o.spec.to_string())
            .collect();
        if specs.len() > 1 {
            println!();
            println!(
                "  {} feature `{f}` is gated inconsistently: {}",
                style::warn(),
                specs.join(" vs ")
            );
        }
    }
    println!();
    println!(
        "  {}  {}  {}   {}",
        style::green("✓ ships"),
        style::yellow("✓@N ships, active from epoch N"),
        style::dim("– not compiled"),
        style::dim("solex features <name> lists a feature's markers")
    );
    println!();
}
