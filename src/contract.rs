//! Game Performance Contract (GPC).
//!
//! Session-aware planner that only targets detected game PIDs. When several
//! processes share a game identity, GPC partitions CPU resources so instances
//! stop fighting over the same cores — a common stutter source — and sets a
//! soft residency floor so Windows is less eager to page the game out.
//!
//! On hybrid CPUs, GPC prefers performance CPU sets. On homogeneous CPUs it
//! falls back to disjoint affinity masks for multi-instance sessions.

use std::collections::BTreeMap;

use crate::detect::ProcessSnapshot;

/// Role of a game process inside a same-name session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContractRole {
    /// Largest instance in the session (gets the larger CPU slice).
    Primary,
    /// Additional same-name instance (gets a disjoint CPU slice when possible).
    Peer,
    /// Lone game process.
    Solo,
}

/// Per-game contract produced by the GPC planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameContract {
    /// Target process id.
    pub pid: u32,
    /// Display name.
    pub name: String,
    /// Normalized identity used for session grouping.
    pub normalized_name: String,
    /// Session role.
    pub role: ContractRole,
    /// Disjoint performance CPU set ids (hybrid CPUs).
    pub cpu_set_ids: Vec<u32>,
    /// Fallback affinity mask when CPU sets are unavailable (0 = unused).
    pub affinity_mask: usize,
    /// Soft minimum working-set floor in bytes (0 = disabled).
    pub residency_floor_bytes: u64,
}

/// Build contracts for all game snapshots.
pub fn build_contracts(
    games: &[ProcessSnapshot],
    performance_cpu_sets: &[u32],
    residency_floor_percent: u8,
    logical_cpus: usize,
) -> Vec<GameContract> {
    let floor_percent = residency_floor_percent.min(90);

    let mut by_name: BTreeMap<&str, Vec<&ProcessSnapshot>> = BTreeMap::new();
    for game in games {
        by_name
            .entry(game.normalized_name.as_str())
            .or_default()
            .push(game);
    }

    let mut contracts = Vec::with_capacity(games.len());

    for (normalized_name, mut members) in by_name {
        members.sort_by(|a, b| {
            b.memory_bytes
                .cmp(&a.memory_bytes)
                .then_with(|| a.pid.cmp(&b.pid))
        });

        let member_count = members.len();
        let slices = partition_cpu_sets(performance_cpu_sets, member_count);
        let masks = if performance_cpu_sets.is_empty() && member_count > 1 {
            partition_affinity_masks(logical_cpus, member_count)
        } else {
            vec![0usize; member_count]
        };

        for (index, member) in members.into_iter().enumerate() {
            let role = match member_count {
                1 => ContractRole::Solo,
                _ if index == 0 => ContractRole::Primary,
                _ => ContractRole::Peer,
            };

            let floor = if floor_percent == 0 {
                0
            } else {
                member.memory_bytes.saturating_mul(u64::from(floor_percent)) / 100
            };

            contracts.push(GameContract {
                pid: member.pid,
                name: member.name.clone(),
                normalized_name: normalized_name.to_string(),
                role,
                cpu_set_ids: slices.get(index).cloned().unwrap_or_default(),
                affinity_mask: masks.get(index).copied().unwrap_or(0),
                residency_floor_bytes: floor,
            });
        }
    }

    contracts.sort_by_key(|a| a.pid);
    contracts
}

/// Split performance CPU sets across `count` session members.
pub fn partition_cpu_sets(performance_cpu_sets: &[u32], count: usize) -> Vec<Vec<u32>> {
    if count == 0 {
        return Vec::new();
    }
    if performance_cpu_sets.is_empty() {
        return vec![Vec::new(); count];
    }
    if count == 1 {
        return vec![performance_cpu_sets.to_vec()];
    }

    let n = performance_cpu_sets.len();
    if n < count {
        let mut out = vec![Vec::new(); count];
        for (index, id) in performance_cpu_sets.iter().enumerate() {
            out[index % count].push(*id);
        }
        return out;
    }

    let base = n / count;
    let rem = n % count;
    let mut out = Vec::with_capacity(count);
    let mut offset = 0usize;
    for i in 0..count {
        let take = base + usize::from(i < rem);
        out.push(performance_cpu_sets[offset..offset + take].to_vec());
        offset += take;
    }
    out
}

/// Split logical processors into disjoint affinity masks.
pub fn partition_affinity_masks(logical_cpus: usize, count: usize) -> Vec<usize> {
    if count == 0 {
        return Vec::new();
    }
    let usable = logical_cpus.min(usize::BITS as usize).max(1);
    if usable < count {
        let mut out = vec![0usize; count];
        for i in 0..usable {
            out[i % count] |= 1usize << i;
        }
        return out;
    }

    let base = usable / count;
    let rem = usable % count;
    let mut out = Vec::with_capacity(count);
    let mut bit = 0usize;
    for i in 0..count {
        let take = base + usize::from(i < rem);
        let mut mask = 0usize;
        for _ in 0..take {
            mask |= 1usize << bit;
            bit += 1;
        }
        out.push(mask);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::{ProcessClass, ProcessSnapshot};

    fn sample_game(pid: u32, mem: u64) -> ProcessSnapshot {
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
    fn dual_instance_gets_disjoint_cpu_slices() {
        let sets: Vec<u32> = (1..=8).collect();
        let contracts = build_contracts(
            &[
                sample_game(2240, 2_000_000_000),
                sample_game(16548, 2_800_000_000),
            ],
            &sets,
            60,
            16,
        );
        let primary = contracts
            .iter()
            .find(|c| c.role == ContractRole::Primary)
            .expect("primary");
        let peer = contracts
            .iter()
            .find(|c| c.role == ContractRole::Peer)
            .expect("peer");
        assert_eq!(primary.pid, 16548);
        let overlap: Vec<_> = primary
            .cpu_set_ids
            .iter()
            .filter(|id| peer.cpu_set_ids.contains(id))
            .collect();
        assert!(overlap.is_empty());
        assert_eq!(primary.cpu_set_ids.len() + peer.cpu_set_ids.len(), 8);
    }

    #[test]
    fn dual_instance_affinity_fallback_when_no_cpu_sets() {
        let contracts = build_contracts(
            &[
                sample_game(2240, 2_000_000_000),
                sample_game(16548, 2_800_000_000),
            ],
            &[],
            60,
            8,
        );
        let primary = contracts
            .iter()
            .find(|c| c.role == ContractRole::Primary)
            .unwrap();
        let peer = contracts
            .iter()
            .find(|c| c.role == ContractRole::Peer)
            .unwrap();
        assert_ne!(primary.affinity_mask, 0);
        assert_ne!(peer.affinity_mask, 0);
        assert_eq!(primary.affinity_mask & peer.affinity_mask, 0);
    }

    #[test]
    fn partition_prefers_contiguous_blocks() {
        let parts = partition_cpu_sets(&[10, 11, 12, 13, 14, 15], 2);
        assert_eq!(parts[0], vec![10, 11, 12]);
        assert_eq!(parts[1], vec![13, 14, 15]);
    }

    #[test]
    fn residency_floor_is_percent_of_working_set() {
        let contracts = build_contracts(&[sample_game(1, 1000)], &[], 60, 8);
        assert_eq!(contracts[0].residency_floor_bytes, 600);
        assert_eq!(contracts[0].role, ContractRole::Solo);
    }
}
