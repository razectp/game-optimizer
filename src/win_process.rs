//! Windows process control adapters (game PIDs only).
//!
//! All `unsafe` is confined here and documented. This module never terminates
//! processes and never mutates non-game targets at the call sites in `optimize`.

#![allow(unsafe_code)]

use std::sync::Once;

use windows::core::{HRESULT, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::ProcessStatus::EmptyWorkingSet;
use windows::Win32::System::SystemInformation::{
    GetSystemCpuSetInformation, GetSystemInfo, SYSTEM_CPU_SET_INFORMATION, SYSTEM_INFO,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, ProcessMemoryPriority,
    ProcessPowerThrottling, SetPriorityClass, SetProcessAffinityMask, SetProcessDefaultCpuSets,
    SetProcessInformation, SetProcessPriorityBoost, SetProcessWorkingSetSize, HIGH_PRIORITY_CLASS,
    MEMORY_PRIORITY_INFORMATION, MEMORY_PRIORITY_NORMAL, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
    PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SET_QUOTA,
};

use crate::optimize::{ActionResult, OptimizeAction};

static PRIVILEGE_INIT: Once = Once::new();

/// Best-effort enable of `SeDebugPrivilege` (helps open game processes when elevated).
pub fn enable_debug_privilege() {
    PRIVILEGE_INIT.call_once(|| {
        let _ = try_enable_debug_privilege();
    });
}

fn try_enable_debug_privilege() -> windows::core::Result<()> {
    let mut token = HANDLE::default();
    // SAFETY: current process handle is valid for the lifetime of this call.
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
    }

    let mut luid = LUID::default();
    // SAFETY: LookupPrivilegeValueW writes into `luid`; privilege name is static.
    unsafe {
        LookupPrivilegeValueW(
            PCWSTR::null(),
            windows::core::w!("SeDebugPrivilege"),
            &mut luid,
        )?;
    }

    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };

    // SAFETY: token owned by us; privileges points to valid TOKEN_PRIVILEGES.
    let adjust = unsafe { AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None) };
    // SAFETY: token opened above and not used afterward.
    unsafe {
        let _ = CloseHandle(token);
    }
    adjust?;
    let status = unsafe { GetLastError() };
    if status.is_ok() {
        Ok(())
    } else {
        Err(windows::core::Error::from(HRESULT::from_win32(status.0)))
    }
}

/// Apply a single optimization action to a process.
pub fn apply_action(action: &OptimizeAction) -> ActionResult {
    enable_debug_privilege();
    match action {
        OptimizeAction::BoostPriority { pid, name } => boost_priority(*pid, name),
        OptimizeAction::DisablePowerThrottling { pid, name } => {
            disable_power_throttling(*pid, name)
        }
        OptimizeAction::RaiseMemoryPriority { pid, name } => raise_memory_priority(*pid, name),
        OptimizeAction::EnablePriorityBoost { pid, name } => enable_priority_boost(*pid, name),
        OptimizeAction::AssignCpuSets {
            pid,
            name,
            role,
            cpu_set_ids,
        } => assign_cpu_sets(*pid, name, *role, cpu_set_ids),
        OptimizeAction::AssignAffinityMask {
            pid,
            name,
            role,
            mask,
        } => assign_affinity_mask(*pid, name, *role, *mask),
        OptimizeAction::ProtectResidencyFloor {
            pid,
            name,
            floor_bytes,
        } => protect_residency_floor(*pid, name, *floor_bytes),
        OptimizeAction::TrimGameWorkingSet { pid, name } => trim_working_set(*pid, name, true),
        OptimizeAction::TrimNonGameWorkingSet { pid, name } => trim_working_set(*pid, name, false),
    }
}

fn access_hint(err: &windows::core::Error) -> String {
    let text = err.to_string();
    if text.contains("0x80070005") || text.to_lowercase().contains("denied") {
        format!("{text} — re-run game_optimizer as Administrator so game processes can be changed")
    } else {
        text
    }
}

fn boost_priority(pid: u32, name: &str) -> ActionResult {
    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: handle opened with set-information rights.
            unsafe { SetPriorityClass(handle, HIGH_PRIORITY_CLASS) }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!("priority=High pid={pid} name={name}"),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "priority failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn disable_power_throttling(pid: u32, name: &str) -> ActionResult {
    let mut state = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: 0,
    };

    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: state is valid; handle has set rights.
            unsafe {
                SetProcessInformation(
                    handle,
                    ProcessPowerThrottling,
                    std::ptr::from_mut(&mut state).cast(),
                    std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                )
            }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!("power_throttling=off pid={pid} name={name}"),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "power_throttling failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn raise_memory_priority(pid: u32, name: &str) -> ActionResult {
    let mut info = MEMORY_PRIORITY_INFORMATION {
        MemoryPriority: MEMORY_PRIORITY_NORMAL,
    };

    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: official MEMORY_PRIORITY_INFORMATION + ProcessMemoryPriority (class 0).
            unsafe {
                SetProcessInformation(
                    handle,
                    ProcessMemoryPriority,
                    std::ptr::from_mut(&mut info).cast(),
                    std::mem::size_of::<MEMORY_PRIORITY_INFORMATION>() as u32,
                )
            }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!("memory_priority=Normal(resident) pid={pid} name={name}"),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "memory_priority failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn enable_priority_boost(pid: u32, name: &str) -> ActionResult {
    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: FALSE disables the "disable boosts" flag → boosts stay on.
            unsafe { SetProcessPriorityBoost(handle, false) }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!("priority_boost=enabled pid={pid} name={name}"),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "priority_boost failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn assign_cpu_sets(
    pid: u32,
    name: &str,
    role: crate::contract::ContractRole,
    cpu_set_ids: &[u32],
) -> ActionResult {
    if cpu_set_ids.is_empty() {
        return ActionResult {
            ok: true,
            detail: format!("cpu_sets=skipped(empty) role={role:?} pid={pid} name={name}"),
        };
    }

    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: cpu_set_ids is a contiguous ULONG array; handle has set rights.
            unsafe { SetProcessDefaultCpuSets(handle, Some(cpu_set_ids)).ok() }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!(
                "gpc_cpu_sets role={role:?} sets={} pid={pid} name={name}",
                cpu_set_ids.len()
            ),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "gpc_cpu_sets failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn assign_affinity_mask(
    pid: u32,
    name: &str,
    role: crate::contract::ContractRole,
    mask: usize,
) -> ActionResult {
    if mask == 0 {
        return ActionResult {
            ok: true,
            detail: format!("affinity=skipped(empty) role={role:?} pid={pid} name={name}"),
        };
    }

    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
        |handle| {
            // SAFETY: mask is a valid group-0 affinity bitfield for this process.
            unsafe { SetProcessAffinityMask(handle, mask) }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!("gpc_affinity role={role:?} mask=0x{mask:x} pid={pid} name={name}"),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "gpc_affinity failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn protect_residency_floor(pid: u32, name: &str, floor_bytes: u64) -> ActionResult {
    let floor = usize::try_from(floor_bytes).unwrap_or(usize::MAX);
    match with_process(
        pid,
        PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA,
        |handle| {
            // SAFETY: sets a soft minimum WS; max left unbounded (usize::MAX).
            unsafe { SetProcessWorkingSetSize(handle, floor, usize::MAX) }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: format!(
                "residency_floor={}MiB pid={pid} name={name}",
                floor_bytes / (1024 * 1024)
            ),
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!(
                "residency_floor failed pid={pid} name={name}: {}",
                access_hint(&err)
            ),
        },
    }
}

fn trim_working_set(pid: u32, name: &str, is_game: bool) -> ActionResult {
    match with_process(
        pid,
        PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA,
        |handle| {
            // SAFETY: EmptyWorkingSet only pages out unused WS; does not terminate.
            unsafe { EmptyWorkingSet(handle) }
        },
    ) {
        Ok(()) => ActionResult {
            ok: true,
            detail: if is_game {
                format!("game_working_set_trimmed pid={pid} name={name}")
            } else {
                format!("non_game_working_set_trimmed pid={pid} name={name}")
            },
        },
        Err(err) => ActionResult {
            ok: false,
            detail: format!("trim failed pid={pid} name={name}: {}", access_hint(&err)),
        },
    }
}

/// List CPU set ids belonging to the highest EfficiencyClass (P-cores).
///
/// Returns an empty list on homogeneous CPUs or query failure.
pub fn list_performance_cpu_sets() -> Vec<u32> {
    performance_cpu_set_ids().unwrap_or_default()
}

/// Number of logical processors reported by the OS (group 0 affinity width).
pub fn logical_processor_count() -> usize {
    let mut info = SYSTEM_INFO::default();
    // SAFETY: writes into stack-allocated SYSTEM_INFO.
    unsafe { GetSystemInfo(&mut info) };
    info.dwNumberOfProcessors.max(1) as usize
}

fn performance_cpu_set_ids() -> windows::core::Result<Vec<u32>> {
    let mut required = 0u32;
    // SAFETY: size probe; null buffer is allowed to query length.
    let probe = unsafe { GetSystemCpuSetInformation(None, 0, &mut required, None, Some(0)) };
    if !probe.as_bool() && required == 0 {
        return Err(windows::core::Error::from(HRESULT::from_win32(
            unsafe { GetLastError() }.0,
        )));
    }

    let mut buffer = vec![0u8; required as usize];
    let mut returned = 0u32;
    // SAFETY: buffer is large enough for required bytes.
    let filled = unsafe {
        GetSystemCpuSetInformation(
            Some(buffer.as_mut_ptr().cast()),
            required,
            &mut returned,
            None,
            Some(0),
        )
    };
    if !filled.as_bool() {
        return Err(windows::core::Error::from(HRESULT::from_win32(
            unsafe { GetLastError() }.0,
        )));
    }

    let mut max_efficiency = 0u8;
    let mut entries: Vec<(u8, u32)> = Vec::new();
    let mut offset = 0usize;
    while offset + std::mem::size_of::<SYSTEM_CPU_SET_INFORMATION>() <= returned as usize {
        // SAFETY: offset walks Size-prefixed SYSTEM_CPU_SET_INFORMATION records.
        let info = unsafe { &*(buffer.as_ptr().add(offset) as *const SYSTEM_CPU_SET_INFORMATION) };
        let cpu = unsafe { info.Anonymous.CpuSet };
        let efficiency = cpu.EfficiencyClass;
        let id = cpu.Id;
        if efficiency > max_efficiency {
            max_efficiency = efficiency;
        }
        entries.push((efficiency, id));
        let size = info.Size as usize;
        if size == 0 {
            break;
        }
        offset += size;
    }

    let distinct: std::collections::BTreeSet<u8> = entries.iter().map(|(e, _)| *e).collect();
    if distinct.len() <= 1 {
        return Ok(Vec::new());
    }

    Ok(entries
        .into_iter()
        .filter(|(efficiency, _)| *efficiency == max_efficiency)
        .map(|(_, id)| id)
        .collect())
}

fn with_process<F>(
    pid: u32,
    access: windows::Win32::System::Threading::PROCESS_ACCESS_RIGHTS,
    f: F,
) -> windows::core::Result<()>
where
    F: FnOnce(HANDLE) -> windows::core::Result<()>,
{
    // SAFETY: OpenProcess with explicit access mask; handle closed on all paths.
    let handle = unsafe { OpenProcess(access, false, pid)? };
    let result = f(handle);
    // SAFETY: handle was returned by OpenProcess and is not used after this call.
    unsafe {
        let _ = CloseHandle(handle);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimize::OptimizeAction;

    #[test]
    fn refuse_pid_zero_open() {
        let result = apply_action(&OptimizeAction::BoostPriority {
            pid: 0,
            name: "system".into(),
        });
        assert!(!result.ok);
    }
}
