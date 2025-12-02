//! Process management syscalls
//!
use alloc::sync::Arc;

use crate::{
    fs::{open_file, OpenFlags},
    loader::get_app_data_by_name,
    mm::{translated_refmut, translated_str, translated_byte_buffer, PageTable, VirtAddr, VirtPageNum},
    task::{
        add_task, current_task, current_user_token, exit_current_and_run_next,
        suspend_current_and_run_next, get_syscall_count,
    },
};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

pub fn sys_exit(exit_code: i32) -> ! {
    trace!("kernel:pid[{}] sys_exit", current_task().unwrap().pid.0);
    exit_current_and_run_next(exit_code);
    panic!("Unreachable in sys_exit!");
}

pub fn sys_yield() -> isize {
    //trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

pub fn sys_getpid() -> isize {
    trace!("kernel: sys_getpid pid:{}", current_task().unwrap().pid.0);
    current_task().unwrap().pid.0 as isize
}

pub fn sys_fork() -> isize {
    trace!("kernel:pid[{}] sys_fork", current_task().unwrap().pid.0);
    let current_task = current_task().unwrap();
    let new_task = current_task.fork();
    let new_pid = new_task.pid.0;
    // modify trap context of new_task, because it returns immediately after switching
    let trap_cx = new_task.inner_exclusive_access().get_trap_cx();
    // we do not have to move to next instruction since we have done it before
    // for child process, fork returns 0
    trap_cx.x[10] = 0;
    // add new task to scheduler
    add_task(new_task);
    new_pid as isize
}

pub fn sys_exec(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_exec", current_task().unwrap().pid.0);
    let token = current_user_token();
    let path = translated_str(token, path);
    if let Some(app_inode) = open_file(path.as_str(), OpenFlags::RDONLY) {
        let all_data = app_inode.read_all();
        let task = current_task().unwrap();
        task.exec(all_data.as_slice());
        0
    } else {
        -1
    }
}

/// If there is not a child process whose pid is same as given, return -1.
/// Else if there is a child process but it is still running, return -2.
pub fn sys_waitpid(pid: isize, exit_code_ptr: *mut i32) -> isize {
    //trace!("kernel: sys_waitpid");
    let task = current_task().unwrap();
    // find a child process

    // ---- access current PCB exclusively
    let mut inner = task.inner_exclusive_access();
    if !inner
        .children
        .iter()
        .any(|p| pid == -1 || pid as usize == p.getpid())
    {
        return -1;
        // ---- release current PCB
    }
    let pair = inner.children.iter().enumerate().find(|(_, p)| {
        // ++++ temporarily access child PCB exclusively
        p.inner_exclusive_access().is_zombie() && (pid == -1 || pid as usize == p.getpid())
        // ++++ release child PCB
    });
    if let Some((idx, _)) = pair {
        let child = inner.children.remove(idx);
        // confirm that child will be deallocated after being removed from children list
        assert_eq!(Arc::strong_count(&child), 1);
        let found_pid = child.getpid();
        // ++++ temporarily access child PCB exclusively
        let exit_code = child.inner_exclusive_access().exit_code;
        // ++++ release child PCB
        *translated_refmut(inner.memory_set.token(), exit_code_ptr) = exit_code;
        found_pid as isize
    } else {
        -2
    }
    // ---- release current PCB automatically
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel:pid[{}] sys_get_time", current_task().unwrap().pid.0);
    let us = crate::timer::get_time_us();
    let token = current_user_token();

    // Handle the case where TimeVal might be split across two pages
    let timeval = TimeVal {
        sec: us / 1_000_000,
        usec: us % 1_000_000,
    };

    // We need to write the TimeVal struct to user space
    // Handle potential page boundary crossing
    use core::slice;

    let timeval_bytes = unsafe {
        slice::from_raw_parts(
            &timeval as *const TimeVal as *const u8,
            core::mem::size_of::<TimeVal>(),
        )
    };

    let mut buffers = translated_byte_buffer(token, ts as *const u8, core::mem::size_of::<TimeVal>());
    let mut offset = 0;
    for buffer in buffers.iter_mut() {
        let len = buffer.len();
        buffer.copy_from_slice(&timeval_bytes[offset..offset + len]);
        offset += len;
    }

    0
}

/// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel:pid[{}] sys_mmap", current_task().unwrap().pid.0);

    use crate::config::PAGE_SIZE;
    use crate::mm::{MapPermission, VirtAddr, VirtPageNum};

    // Check alignment
    if start % PAGE_SIZE != 0 {
        return -1;
    }

    // Check length
    if len == 0 {
        return -1;
    }

    // Check port bits (should contain valid permission bits)
    if port & !0x7 != 0 {
        return -1;
    }

    // Ensure at least one of R/W/X is set
    if port & 0x7 == 0 {
        return -1;
    }

    // Convert port to MapPermission
    let mut map_perm = MapPermission::U;
    if port & 0x1 != 0 {
        map_perm |= MapPermission::R;
    }
    if port & 0x2 != 0 {
        map_perm |= MapPermission::W;
    }
    if port & 0x4 != 0 {
        map_perm |= MapPermission::X;
    }

    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();

    // Check if the range is already mapped
    let start_vpn: VirtPageNum = start_va.floor();
    let end_vpn: VirtPageNum = end_va.ceil();

    for vpn in start_vpn.0..end_vpn.0 {
        if let Some(pte) = inner.memory_set.translate(VirtPageNum(vpn)) {
            if pte.is_valid() {
                return -1;
            }
        }
    }

    // Map the area
    inner.memory_set.insert_framed_area(start_va, end_va, map_perm);

    0
}

/// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel:pid[{}] sys_munmap", current_task().unwrap().pid.0);

    use crate::config::PAGE_SIZE;
    use crate::mm::{VirtAddr, VirtPageNum};

    // Check alignment
    if start % PAGE_SIZE != 0 {
        return -1;
    }

    // Check length
    if len == 0 {
        return -1;
    }

    let start_va = VirtAddr::from(start);
    let end_va = VirtAddr::from(start + len);
    let start_vpn: VirtPageNum = start_va.floor();
    let end_vpn: VirtPageNum = end_va.ceil();

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();

    // Check if all pages in the range are mapped and valid
    for vpn in start_vpn.0..end_vpn.0 {
        match inner.memory_set.translate(VirtPageNum(vpn)) {
            Some(pte) if pte.is_valid() => {}
            _ => return -1,
        }
    }

    // Remove the area starting at start_vpn
    // Note: This assumes the entire mapped region from mmap forms a single MapArea
    inner.memory_set.remove_area_with_start_vpn(start_vpn);

    0
}

/// TODO: Finish sys_trace to pass testcases
/// HINT: You might reimplement it with virtual memory management.
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    match trace_request {
        0 => {
            // Read
            let addr = id;
            let token = current_user_token();
            let page_table = PageTable::from_token(token);
            let va = VirtAddr::from(addr);
            let vpn: VirtPageNum = va.floor();

            if let Some(pte) = page_table.translate(vpn) {
                if pte.is_valid() && pte.readable() && (pte.flags() & crate::mm::PTEFlags::U) != crate::mm::PTEFlags::empty() {
                    let ppn = pte.ppn();
                    let offset = va.page_offset();
                    let byte = ppn.get_bytes_array()[offset];
                    return byte as isize;
                }
            }
            -1
        }
        1 => {
            // Write
            let addr = id;
            let token = current_user_token();
            let page_table = PageTable::from_token(token);
            let va = VirtAddr::from(addr);
            let vpn: VirtPageNum = va.floor();

            if let Some(pte) = page_table.translate(vpn) {
                if pte.is_valid() && pte.writable() && (pte.flags() & crate::mm::PTEFlags::U) != crate::mm::PTEFlags::empty() {
                    let ppn = pte.ppn();
                    let offset = va.page_offset();
                    ppn.get_bytes_array()[offset] = data as u8;
                    return 0;
                }
            }
            -1
        }
        2 => {
            // Syscall counting
            let syscall_id = id;
            get_syscall_count(syscall_id) as isize
        }
        _ => {
            // Other requests
            -1
        }
    }
}

/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel:pid[{}] sys_sbrk", current_task().unwrap().pid.0);
    if let Some(old_brk) = current_task().unwrap().change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}

/// YOUR JOB: Implement spawn.
/// HINT: fork + exec =/= spawn
pub fn sys_spawn(path: *const u8) -> isize {
    trace!("kernel:pid[{}] sys_spawn", current_task().unwrap().pid.0);

    let token = current_user_token();
    let path = translated_str(token, path);

    if let Some(data) = get_app_data_by_name(path.as_str()) {
        let current = current_task().unwrap();
        // Create a new task directly from the ELF data
        // Unlike fork, we don't copy the parent's address space
        let new_task = Arc::new(crate::task::TaskControlBlock::new(data));
        let new_pid = new_task.pid.0;

        // Set parent relationship
        let mut new_inner = new_task.inner_exclusive_access();
        new_inner.parent = Some(Arc::downgrade(&current));
        drop(new_inner);

        // Add to parent's children
        let mut current_inner = current.inner_exclusive_access();
        current_inner.children.push(new_task.clone());
        drop(current_inner);

        // Add new task to scheduler
        add_task(new_task);
        new_pid as isize
    } else {
        -1
    }
}

// YOUR JOB: Set task priority.
pub fn sys_set_priority(prio: isize) -> isize {
    trace!("kernel:pid[{}] sys_set_priority", current_task().unwrap().pid.0);

    // Priority must be >= 2
    if prio < 2 {
        return -1;
    }

    let task = current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    inner.priority = prio as usize;

    prio
}
