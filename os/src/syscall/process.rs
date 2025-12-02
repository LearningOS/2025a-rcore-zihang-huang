//! Process management syscalls
use crate::task::{change_program_brk, exit_current_and_run_next, suspend_current_and_run_next, current_user_token, mmap, munmap, get_syscall_count};
use crate::mm::{translated_byte_buffer, PageTable, VirtAddr, VirtPageNum};
use crate::timer::{get_time_us};

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// YOUR JOB: get time with second and microsecond
/// HINT: You might reimplement it with virtual memory management.
/// HINT: What if [`TimeVal`] is splitted by two pages ?
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let us = get_time_us();
    let sec = us / 1_000_000;
    let usec = us % 1_000_000;
    let token = current_user_token();
    let buffers = translated_byte_buffer(token, ts as *const u8, core::mem::size_of::<TimeVal>());
    // TimeVal may be split across two pages, so we need to write to buffers carefully
    let mut offset = 0;
    let timeval_bytes = [
        (sec & 0xff) as u8,
        ((sec >> 8) & 0xff) as u8,
        ((sec >> 16) & 0xff) as u8,
        ((sec >> 24) & 0xff) as u8,
        ((sec >> 32) & 0xff) as u8,
        ((sec >> 40) & 0xff) as u8,
        ((sec >> 48) & 0xff) as u8,
        ((sec >> 56) & 0xff) as u8,
        (usec & 0xff) as u8,
        ((usec >> 8) & 0xff) as u8,
        ((usec >> 16) & 0xff) as u8,
        ((usec >> 24) & 0xff) as u8,
        ((usec >> 32) & 0xff) as u8,
        ((usec >> 40) & 0xff) as u8,
        ((usec >> 48) & 0xff) as u8,
        ((usec >> 56) & 0xff) as u8,
    ];
    for buffer in buffers {
        let len = buffer.len();
        buffer.copy_from_slice(&timeval_bytes[offset..offset + len]);
        offset += len;
    }
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

// YOUR JOB: Implement mmap.
pub fn sys_mmap(start: usize, len: usize, prot: usize) -> isize {
    trace!("kernel: sys_mmap");
    mmap(start, len, prot)
}

// YOUR JOB: Implement munmap.
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    munmap(start, len)
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
