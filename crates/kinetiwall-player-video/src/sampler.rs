//! Platform-specific process resource sampler (working set, private memory, CPU %)
//! for child worker processes on Windows.

use kinetiwall_playback::ProcessResourceSample;
use std::time::Instant;

#[cfg(windows)]
mod sys {
    use kinetiwall_playback::ProcessResourceSample;
    use std::time::Instant;

    #[repr(C)]
    struct ProcessMemoryCountersEx {
        cb: u32,
        page_fault_count: u32,
        peak_working_set_size: usize,
        working_set_size: usize,
        quota_peak_paged_pool_usage: usize,
        quota_paged_pool_usage: usize,
        quota_peak_non_paged_pool_usage: usize,
        quota_non_paged_pool_usage: usize,
        pagefile_usage: usize,
        peak_pagefile_usage: usize,
        private_usage: usize,
    }

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        dw_low_date_time: u32,
        dw_high_date_time: u32,
    }

    impl FileTime {
        fn as_u64(&self) -> u64 {
            ((self.dw_high_date_time as u64) << 32) | (self.dw_low_date_time as u64)
        }
    }

    type Handle = *mut std::ffi::c_void;
    type Bool = i32;

    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;

    extern "system" {
        fn OpenProcess(dwDesiredAccess: u32, bInheritHandle: Bool, dwProcessId: u32) -> Handle;
        fn CloseHandle(hObject: Handle) -> Bool;
        fn K32GetProcessMemoryInfo(
            hProcess: Handle,
            ppsmex: *mut ProcessMemoryCountersEx,
            cb: u32,
        ) -> Bool;
        fn GetProcessTimes(
            hProcess: Handle,
            lpCreationTime: *mut FileTime,
            lpExitTime: *mut FileTime,
            lpKernelTime: *mut FileTime,
            lpUserTime: *mut FileTime,
        ) -> Bool;
    }

    pub fn sample_process(
        pid: u32,
        prev_cpu_time: &mut Option<(u64, Instant)>,
    ) -> Option<ProcessResourceSample> {
        let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if handle.is_null() {
            return None;
        }

        let mut mem = ProcessMemoryCountersEx {
            cb: std::mem::size_of::<ProcessMemoryCountersEx>() as u32,
            page_fault_count: 0,
            peak_working_set_size: 0,
            working_set_size: 0,
            quota_peak_paged_pool_usage: 0,
            quota_paged_pool_usage: 0,
            quota_peak_non_paged_pool_usage: 0,
            quota_non_paged_pool_usage: 0,
            pagefile_usage: 0,
            peak_pagefile_usage: 0,
            private_usage: 0,
        };

        let mem_ok = unsafe { K32GetProcessMemoryInfo(handle, &mut mem as *mut _, mem.cb) != 0 };

        let mut creation = FileTime::default();
        let mut exit = FileTime::default();
        let mut kernel = FileTime::default();
        let mut user = FileTime::default();

        let times_ok = unsafe {
            GetProcessTimes(
                handle,
                &mut creation as *mut _,
                &mut exit as *mut _,
                &mut kernel as *mut _,
                &mut user as *mut _,
            ) != 0
        };

        unsafe {
            CloseHandle(handle);
        }

        let now = Instant::now();
        let mut sample = ProcessResourceSample::new(0, pid);

        if mem_ok {
            sample.working_set_bytes = Some(mem.working_set_size as u64);
            sample.private_memory_bytes = Some(mem.private_usage as u64);
        }

        if times_ok {
            let total_cpu_100ns = kernel.as_u64().saturating_add(user.as_u64());
            if let Some((prev_total, prev_time)) = *prev_cpu_time {
                let elapsed_dur = now.saturating_duration_since(prev_time);
                let elapsed_100ns = (elapsed_dur.as_nanos() / 100) as u64;
                let cpu_diff = total_cpu_100ns.saturating_sub(prev_total);

                if elapsed_100ns > 0 {
                    let cpu_pct = (cpu_diff as f64 / elapsed_100ns as f64) * 100.0;
                    sample.cpu_percent = Some((cpu_pct as f32).clamp(0.0, 100.0));
                }
            }
            *prev_cpu_time = Some((total_cpu_100ns, now));
        }

        Some(sample)
    }
}

/// Samples resource usage for a child process by PID.
pub fn sample_child_process(
    pid: u32,
    prev_cpu_time: &mut Option<(u64, Instant)>,
) -> Option<ProcessResourceSample> {
    #[cfg(windows)]
    {
        sys::sample_process(pid, prev_cpu_time)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        let _ = prev_cpu_time;
        None
    }
}
