//! Human-readable report formatting.

use crate::optimize::OptimizeReport;

/// Format bytes as a short mebibyte string.
pub fn format_mib(bytes: u64) -> String {
    let mib = bytes as f64 / (1024.0 * 1024.0);
    format!("{mib:.1} MiB")
}

/// Render a full optimize/scan report for the terminal.
pub fn render_report(report: &OptimizeReport) -> String {
    let mut lines = Vec::new();
    lines.push(String::from("=== Game Optimizer (GPC) ==="));
    lines.push(String::from(
        "Scope: GPC on games + optional WS trim for large non-games (Chrome/…)",
    ));
    if report.dry_run {
        lines.push(String::from("Mode: dry-run (no changes applied)"));
    }

    lines.push(String::new());
    lines.push(format!("Games detected: {}", report.games.len()));
    if report.games.is_empty() {
        lines.push(String::from(
            "  (none — start a game, or add its process name to games.toml)",
        ));
    } else {
        for game in &report.games {
            lines.push(format!(
                "  pid={:<6} {:<24} {}",
                game.pid,
                game.name,
                format_mib(game.memory_bytes)
            ));
        }
    }

    if !report.contracts.is_empty() {
        lines.push(String::new());
        lines.push(String::from("GPC session contracts:"));
        for contract in &report.contracts {
            lines.push(format!(
                "  pid={:<6} role={:<8?} sets={:<3} affinity=0x{:<8x} floor={}",
                contract.pid,
                contract.role,
                contract.cpu_set_ids.len(),
                contract.affinity_mask,
                format_mib(contract.residency_floor_bytes)
            ));
        }
    }

    lines.push(String::new());
    lines.push(format!(
        "Non-game reclaim candidates: {}",
        report.reclaim_candidates.len()
    ));
    for proc in report.reclaim_candidates.iter().take(12) {
        lines.push(format!(
            "  pid={:<6} {:<24} {}",
            proc.pid,
            proc.name,
            format_mib(proc.memory_bytes)
        ));
    }

    lines.push(String::new());
    lines.push(format!("Planned actions: {}", report.actions.len()));
    if report.results.is_empty() {
        for action in &report.actions {
            lines.push(format!("  plan: {action:?}"));
        }
    } else {
        for result in &report.results {
            let mark = if result.ok { "ok" } else { "fail" };
            lines.push(format!("  [{mark}] {}", result.detail));
        }
    }

    lines.push(String::new());
    lines.push(String::from(
        "Safety: games are never closed. Cursor/VS Code are not trimmed by default.",
    ));
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_mib_reasonable() {
        assert_eq!(format_mib(1024 * 1024), "1.0 MiB");
    }
}
