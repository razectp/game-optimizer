//! Orchestrate game-only Game Performance Contract (GPC) actions.

use crate::config::AppConfig;
use crate::contract::{build_contracts, ContractRole, GameContract};
use crate::detect::{snapshot_processes, ProcessClass, ProcessSnapshot};
use crate::win_process;

/// Caller request for one optimization pass.
#[derive(Debug, Clone)]
pub struct OptimizeRequest {
    /// Loaded configuration.
    pub config: AppConfig,
    /// When true, compute actions but do not apply them.
    pub dry_run: bool,
}

/// Planned mutation against a single **game** process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptimizeAction {
    /// Set High priority class on a game process.
    BoostPriority {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
    /// Disable execution-speed power throttling on a game process.
    DisablePowerThrottling {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
    /// Raise memory priority so the OS prefers keeping game pages resident.
    RaiseMemoryPriority {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
    /// Keep dynamic priority boosts enabled for the game.
    EnablePriorityBoost {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
    /// Assign an explicit performance CPU-set slice (GPC session partition).
    AssignCpuSets {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
        /// Session role label for reporting.
        role: ContractRole,
        /// CPU set identifiers for this instance.
        cpu_set_ids: Vec<u32>,
    },
    /// Assign a disjoint affinity mask (homogeneous CPU fallback).
    AssignAffinityMask {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
        /// Session role label for reporting.
        role: ContractRole,
        /// Bitmask of logical processors.
        mask: usize,
    },
    /// Soft working-set floor so Windows is less eager to page the game out.
    ProtectResidencyFloor {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
        /// Minimum working set in bytes.
        floor_bytes: u64,
    },
    /// Empty the game working set (optional; may hitch when pages return).
    TrimGameWorkingSet {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
    /// Empty a large non-game working set (Chrome, Edge, …).
    TrimNonGameWorkingSet {
        /// Target process id.
        pid: u32,
        /// Process display name.
        name: String,
    },
}

impl OptimizeAction {
    /// Process id targeted by this action.
    pub fn pid(&self) -> u32 {
        match self {
            Self::BoostPriority { pid, .. }
            | Self::DisablePowerThrottling { pid, .. }
            | Self::RaiseMemoryPriority { pid, .. }
            | Self::EnablePriorityBoost { pid, .. }
            | Self::AssignCpuSets { pid, .. }
            | Self::AssignAffinityMask { pid, .. }
            | Self::ProtectResidencyFloor { pid, .. }
            | Self::TrimGameWorkingSet { pid, .. }
            | Self::TrimNonGameWorkingSet { pid, .. } => *pid,
        }
    }

    /// True when this action mutates a game process.
    pub fn targets_game(&self) -> bool {
        !matches!(self, Self::TrimNonGameWorkingSet { .. })
    }
}

/// Outcome of applying one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionResult {
    /// Whether the OS call succeeded.
    pub ok: bool,
    /// Human-readable detail line.
    pub detail: String,
}

/// Full report for one optimize/scan pass.
#[derive(Debug, Clone)]
pub struct OptimizeReport {
    /// Detected game processes.
    pub games: Vec<ProcessSnapshot>,
    /// Non-game reclaim candidates (Chrome, …).
    pub reclaim_candidates: Vec<ProcessSnapshot>,
    /// GPC contracts computed for this pass.
    pub contracts: Vec<GameContract>,
    /// Actions planned for this pass.
    pub actions: Vec<OptimizeAction>,
    /// Results after apply (empty on dry-run / scan).
    pub results: Vec<ActionResult>,
    /// Whether mutations were skipped.
    pub dry_run: bool,
}

/// Build the action plan from GPC contracts plus optional non-game reclaim.
pub fn plan_actions(
    config: &AppConfig,
    contracts: &[GameContract],
    processes: &[ProcessSnapshot],
) -> Vec<OptimizeAction> {
    let mut actions = Vec::new();

    for contract in contracts {
        if config.boost_priority {
            actions.push(OptimizeAction::BoostPriority {
                pid: contract.pid,
                name: contract.name.clone(),
            });
        }
        if config.disable_power_throttling {
            actions.push(OptimizeAction::DisablePowerThrottling {
                pid: contract.pid,
                name: contract.name.clone(),
            });
        }
        if config.raise_memory_priority {
            actions.push(OptimizeAction::RaiseMemoryPriority {
                pid: contract.pid,
                name: contract.name.clone(),
            });
        }
        if config.enable_priority_boost {
            actions.push(OptimizeAction::EnablePriorityBoost {
                pid: contract.pid,
                name: contract.name.clone(),
            });
        }
        if config.prefer_performance_cores && !contract.cpu_set_ids.is_empty() {
            actions.push(OptimizeAction::AssignCpuSets {
                pid: contract.pid,
                name: contract.name.clone(),
                role: contract.role,
                cpu_set_ids: contract.cpu_set_ids.clone(),
            });
        } else if config.prefer_performance_cores && contract.affinity_mask != 0 {
            actions.push(OptimizeAction::AssignAffinityMask {
                pid: contract.pid,
                name: contract.name.clone(),
                role: contract.role,
                mask: contract.affinity_mask,
            });
        }
        if config.trim_game_memory {
            actions.push(OptimizeAction::TrimGameWorkingSet {
                pid: contract.pid,
                name: contract.name.clone(),
            });
        } else if contract.residency_floor_bytes > 0 {
            actions.push(OptimizeAction::ProtectResidencyFloor {
                pid: contract.pid,
                name: contract.name.clone(),
                floor_bytes: contract.residency_floor_bytes,
            });
        }
    }

    if config.trim_non_game_memory {
        for proc in processes
            .iter()
            .filter(|p| p.class == ProcessClass::ReclaimCandidate)
            .take(config.max_trim_candidates)
        {
            debug_assert_ne!(proc.class, ProcessClass::Game);
            actions.push(OptimizeAction::TrimNonGameWorkingSet {
                pid: proc.pid,
                name: proc.name.clone(),
            });
        }
    }

    actions
}

/// Scan processes, build GPC contracts, and optionally apply actions.
pub fn optimize_system(request: &OptimizeRequest) -> OptimizeReport {
    if !request.dry_run {
        win_process::enable_debug_privilege();
    }

    let processes = snapshot_processes(&request.config);
    let games: Vec<_> = processes
        .iter()
        .filter(|p| p.class == ProcessClass::Game)
        .cloned()
        .collect();
    let reclaim_candidates: Vec<_> = processes
        .iter()
        .filter(|p| p.class == ProcessClass::ReclaimCandidate)
        .cloned()
        .collect();

    let perf_sets = if request.config.prefer_performance_cores {
        win_process::list_performance_cpu_sets()
    } else {
        Vec::new()
    };

    let contracts = build_contracts(
        &games,
        &perf_sets,
        request.config.residency_floor_percent,
        win_process::logical_processor_count(),
    );
    let actions = plan_actions(&request.config, &contracts, &processes);

    let game_pids: std::collections::BTreeSet<u32> = games.iter().map(|g| g.pid).collect();
    for action in &actions {
        if action.targets_game() {
            assert!(
                game_pids.contains(&action.pid()),
                "refusing non-game target for game action pid={}",
                action.pid()
            );
        } else {
            assert!(
                !game_pids.contains(&action.pid()),
                "refusing to trim game via non-game path pid={}",
                action.pid()
            );
        }
    }

    let results = if request.dry_run {
        Vec::new()
    } else {
        actions.iter().map(win_process::apply_action).collect()
    };

    OptimizeReport {
        games,
        reclaim_candidates,
        contracts,
        actions,
        results,
        dry_run: request.dry_run,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::ProcessClass;

    fn game(pid: u32, mem: u64) -> ProcessSnapshot {
        ProcessSnapshot {
            pid,
            name: "SampleGame.exe".into(),
            normalized_name: "samplegame".into(),
            memory_bytes: mem,
            exe_path: None,
            class: ProcessClass::Game,
        }
    }

    #[test]
    fn plan_partitions_dual_instance_and_ignores_empty_cpu_when_no_sets() {
        let config = AppConfig::default();
        let contracts = build_contracts(
            &[game(2240, 2_000_000_000), game(16548, 2_800_000_000)],
            &[1, 2, 3, 4],
            60,
            16,
        );
        let actions = plan_actions(&config, &contracts, &[]);
        assert!(actions.iter().all(|a| a.pid() == 2240 || a.pid() == 16548));
        let assigns: Vec<_> = actions
            .iter()
            .filter_map(|a| match a {
                OptimizeAction::AssignCpuSets {
                    pid,
                    role,
                    cpu_set_ids,
                    ..
                } => Some((*pid, *role, cpu_set_ids.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(assigns.len(), 2);
        assert!(assigns
            .iter()
            .any(|(_, role, _)| *role == ContractRole::Primary));
        assert!(assigns
            .iter()
            .any(|(_, role, _)| *role == ContractRole::Peer));
    }

    #[test]
    fn residency_floor_actions_created() {
        let config = AppConfig {
            residency_floor_percent: 50,
            prefer_performance_cores: false,
            trim_game_memory: false,
            boost_priority: false,
            disable_power_throttling: false,
            raise_memory_priority: false,
            enable_priority_boost: false,
            ..AppConfig::default()
        };
        let contracts = build_contracts(&[game(7, 2000)], &[], 50, 8);
        let actions = plan_actions(&config, &contracts, &[]);
        assert!(actions.iter().any(|a| matches!(
            a,
            OptimizeAction::ProtectResidencyFloor {
                floor_bytes: 1000,
                ..
            }
        )));
    }

    #[test]
    fn trim_game_memory_skips_residency_floor() {
        let config = AppConfig {
            trim_game_memory: true,
            prefer_performance_cores: false,
            raise_memory_priority: false,
            boost_priority: false,
            disable_power_throttling: false,
            enable_priority_boost: false,
            residency_floor_percent: 60,
            trim_non_game_memory: false,
            ..AppConfig::default()
        };
        let contracts = build_contracts(&[game(7, 2000)], &[], 60, 8);
        let actions = plan_actions(&config, &contracts, &[]);
        assert!(actions
            .iter()
            .any(|a| matches!(a, OptimizeAction::TrimGameWorkingSet { .. })));
        assert!(actions
            .iter()
            .all(|a| !matches!(a, OptimizeAction::ProtectResidencyFloor { .. })));
    }

    #[test]
    fn chrome_reclaim_is_planned() {
        let config = AppConfig {
            boost_priority: false,
            disable_power_throttling: false,
            raise_memory_priority: false,
            enable_priority_boost: false,
            prefer_performance_cores: false,
            residency_floor_percent: 0,
            trim_game_memory: false,
            trim_non_game_memory: true,
            ..AppConfig::default()
        };
        let chrome = ProcessSnapshot {
            pid: 100,
            name: "chrome.exe".into(),
            normalized_name: "chrome".into(),
            memory_bytes: 600 * 1024 * 1024,
            exe_path: None,
            class: ProcessClass::ReclaimCandidate,
        };
        let actions = plan_actions(&config, &[], &[chrome]);
        assert!(actions
            .iter()
            .any(|a| matches!(a, OptimizeAction::TrimNonGameWorkingSet { pid: 100, .. })));
    }
}
