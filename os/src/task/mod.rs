//! Task management implementation
//!
//! Everything about task management, like starting and switching tasks is
//! implemented here.
//!
//! A single global instance of [`TaskManager`] called `TASK_MANAGER` controls
//! all the tasks in the operating system.
//!
//! Be careful when you see `__switch` ASM function in `switch.S`. Control flow around this function
//! might not be what you expect.

mod context;
mod switch;
#[allow(clippy::module_inception)]
mod task;

use crate::loader::{get_app_data, get_num_app};
use crate::sync::UPSafeCell;
use crate::trap::TrapContext;
use alloc::vec::Vec;
use lazy_static::*;
use switch::__switch;
pub use task::{TaskControlBlock, TaskStatus};

pub use context::TaskContext;

/// The task manager, where all the tasks are managed.
///
/// Functions implemented on `TaskManager` deals with all task state transitions
/// and task context switching. For convenience, you can find wrappers around it
/// in the module level.
///
/// Most of `TaskManager` are hidden behind the field `inner`, to defer
/// borrowing checks to runtime. You can see examples on how to use `inner` in
/// existing functions on `TaskManager`.
pub struct TaskManager {
    /// total number of tasks
    num_app: usize,
    /// use inner value to get mutable access
    inner: UPSafeCell<TaskManagerInner>,
}

/// The task manager inner in 'UPSafeCell'
struct TaskManagerInner {
    /// task list
    tasks: Vec<TaskControlBlock>,
    /// id of current `Running` task
    current_task: usize,
}

lazy_static! {
    /// a `TaskManager` global instance through lazy_static!
    pub static ref TASK_MANAGER: TaskManager = {
        println!("init TASK_MANAGER");
        let num_app = get_num_app();
        println!("num_app = {}", num_app);
        let mut tasks: Vec<TaskControlBlock> = Vec::new();
        for i in 0..num_app {
            tasks.push(TaskControlBlock::new(get_app_data(i), i));
        }
        TaskManager {
            num_app,
            inner: unsafe {
                UPSafeCell::new(TaskManagerInner {
                    tasks,
                    current_task: 0,
                })
            },
        }
    };
}

impl TaskManager {
    /// Run the first task in task list.
    ///
    /// Generally, the first task in task list is an idle task (we call it zero process later).
    /// But in ch4, we load apps statically, so the first task is a real app.
    fn run_first_task(&self) -> ! {
        let mut inner = self.inner.exclusive_access();
        let next_task = &mut inner.tasks[0];
        next_task.task_status = TaskStatus::Running;
        let next_task_cx_ptr = &next_task.task_cx as *const TaskContext;
        drop(inner);
        let mut _unused = TaskContext::zero_init();
        // before this, we should drop local variables that must be dropped manually
        unsafe {
            __switch(&mut _unused as *mut _, next_task_cx_ptr);
        }
        panic!("unreachable in run_first_task!");
    }

    /// Change the status of current `Running` task into `Ready`.
    fn mark_current_suspended(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Ready;
    }

    /// Change the status of current `Running` task into `Exited`.
    fn mark_current_exited(&self) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].task_status = TaskStatus::Exited;
    }

    /// Find next task to run and return task id.
    ///
    /// In this case, we only return the first `Ready` task in task list.
    fn find_next_task(&self) -> Option<usize> {
        let inner = self.inner.exclusive_access();
        let current = inner.current_task;
        (current + 1..current + self.num_app + 1)
            .map(|id| id % self.num_app)
            .find(|id| inner.tasks[*id].task_status == TaskStatus::Ready)
    }

    /// Get the current 'Running' task's token.
    fn get_current_token(&self) -> usize {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_user_token()
    }

    /// Get the current 'Running' task's trap contexts.
    fn get_current_trap_cx(&self) -> &'static mut TrapContext {
        let inner = self.inner.exclusive_access();
        inner.tasks[inner.current_task].get_trap_cx()
    }

    /// Change the current 'Running' task's program break
    pub fn change_current_program_brk(&self, size: i32) -> Option<usize> {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        inner.tasks[cur].change_program_brk(size)
    }

    /// Map memory for the current task
    pub fn mmap_current(&self, start: usize, len: usize, prot: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let task = &mut inner.tasks[cur];

        use crate::mm::{VirtAddr, VirtPageNum, MapPermission};

        let start_va = VirtAddr::from(start);

        // Validate: start must be page-aligned
        if !start_va.aligned() {
            return -1;
        }

        // Validate: prot & !0x7 == 0 (only bits 0-2 are valid)
        if prot & !0x7 != 0 {
            return -1;
        }

        // Validate: prot & 0x7 != 0 (must have at least one permission)
        if prot & 0x7 == 0 {
            return -1;
        }

        // Calculate end address (round up len to page size)
        let end_va = VirtAddr::from(start + len);
        let end_vpn = if len == 0 {
            start_va.floor()
        } else {
            end_va.ceil()
        };
        let start_vpn = start_va.floor();

        // Check if any pages in [start_vpn, end_vpn) are already mapped
        for vpn_val in start_vpn.0..end_vpn.0 {
            let vpn = VirtPageNum(vpn_val);
            if let Some(pte) = task.memory_set.translate(vpn) {
                if pte.is_valid() {
                    return -1; // Page already mapped
                }
            }
        }

        // Convert prot to MapPermission
        let mut map_perm = MapPermission::U; // User accessible
        if prot & 0x1 != 0 {
            map_perm |= MapPermission::R;
        }
        if prot & 0x2 != 0 {
            map_perm |= MapPermission::W;
        }
        if prot & 0x4 != 0 {
            map_perm |= MapPermission::X;
        }

        // Insert framed area
        task.memory_set.insert_framed_area(
            start_va,
            end_vpn.into(),
            map_perm,
        );

        0
    }

    /// Unmap memory for the current task
    pub fn munmap_current(&self, start: usize, len: usize) -> isize {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        let task = &mut inner.tasks[cur];

        use crate::mm::{VirtAddr, VirtPageNum};

        let start_va = VirtAddr::from(start);

        // Validate: start must be page-aligned
        if !start_va.aligned() {
            return -1;
        }

        // Calculate end address (round up len to page size)
        let end_va = VirtAddr::from(start + len);
        let end_vpn = if len == 0 {
            start_va.floor()
        } else {
            end_va.ceil()
        };
        let start_vpn = start_va.floor();

        // Check if all pages in [start_vpn, end_vpn) are mapped
        for vpn_val in start_vpn.0..end_vpn.0 {
            let vpn = VirtPageNum(vpn_val);
            if let Some(pte) = task.memory_set.translate(vpn) {
                if !pte.is_valid() {
                    return -1; // Page not mapped
                }
            } else {
                return -1; // Page not mapped
            }
        }

        // Find and remove the MapArea(s) that cover this range
        // For simplicity, we'll iterate through and unmap each page
        let areas_to_modify: Vec<usize> = task.memory_set.areas
            .iter()
            .enumerate()
            .filter(|(_, area)| {
                // Check if this area overlaps with [start_vpn, end_vpn)
                let area_start = area.vpn_range.get_start();
                let area_end = area.vpn_range.get_end();
                !(area_end.0 <= start_vpn.0 || area_start.0 >= end_vpn.0)
            })
            .map(|(idx, _)| idx)
            .collect();

        if areas_to_modify.is_empty() {
            return -1;
        }

        // For now, we only support unmapping entire areas
        // Check if we're unmapping exactly the areas
        for &idx in areas_to_modify.iter().rev() {
            let area = &task.memory_set.areas[idx];
            if area.vpn_range.get_start() == start_vpn && area.vpn_range.get_end() == end_vpn {
                // Exact match, we can remove this area
                let mut removed_area = task.memory_set.areas.remove(idx);
                removed_area.unmap(&mut task.memory_set.page_table);
                return 0;
            }
        }

        // If we get here, we need to unmap parts of areas or multiple areas
        // This is more complex, so for now we'll just unmap the pages directly
        for vpn_val in start_vpn.0..end_vpn.0 {
            let vpn = VirtPageNum(vpn_val);
            // Find the area containing this vpn and unmap it
            for area in task.memory_set.areas.iter_mut() {
                if area.vpn_range.get_start().0 <= vpn.0 && vpn.0 < area.vpn_range.get_end().0 {
                    area.unmap_one(&mut task.memory_set.page_table, vpn);
                    break;
                }
            }
        }

        0
    }

    /// Increment syscall count for current task
    pub fn increment_syscall_count(&self, syscall_id: usize) {
        let mut inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        if syscall_id < 512 {
            inner.tasks[cur].syscall_count[syscall_id] += 1;
        }
    }

    /// Get syscall count for current task
    pub fn get_syscall_count(&self, syscall_id: usize) -> u32 {
        let inner = self.inner.exclusive_access();
        let cur = inner.current_task;
        if syscall_id < 512 {
            inner.tasks[cur].syscall_count[syscall_id]
        } else {
            0
        }
    }

    /// Switch current `Running` task to the task we have found,
    /// or there is no `Ready` task and we can exit with all applications completed
    fn run_next_task(&self) {
        if let Some(next) = self.find_next_task() {
            let mut inner = self.inner.exclusive_access();
            let current = inner.current_task;
            inner.tasks[next].task_status = TaskStatus::Running;
            inner.current_task = next;
            let current_task_cx_ptr = &mut inner.tasks[current].task_cx as *mut TaskContext;
            let next_task_cx_ptr = &inner.tasks[next].task_cx as *const TaskContext;
            drop(inner);
            // before this, we should drop local variables that must be dropped manually
            unsafe {
                __switch(current_task_cx_ptr, next_task_cx_ptr);
            }
            // go back to user mode
        } else {
            panic!("All applications completed!");
        }
    }
}

/// Run the first task in task list.
pub fn run_first_task() {
    TASK_MANAGER.run_first_task();
}

/// Switch current `Running` task to the task we have found,
/// or there is no `Ready` task and we can exit with all applications completed
fn run_next_task() {
    TASK_MANAGER.run_next_task();
}

/// Change the status of current `Running` task into `Ready`.
fn mark_current_suspended() {
    TASK_MANAGER.mark_current_suspended();
}

/// Change the status of current `Running` task into `Exited`.
fn mark_current_exited() {
    TASK_MANAGER.mark_current_exited();
}

/// Suspend the current 'Running' task and run the next task in task list.
pub fn suspend_current_and_run_next() {
    mark_current_suspended();
    run_next_task();
}

/// Exit the current 'Running' task and run the next task in task list.
pub fn exit_current_and_run_next() {
    mark_current_exited();
    run_next_task();
}

/// Get the current 'Running' task's token.
pub fn current_user_token() -> usize {
    TASK_MANAGER.get_current_token()
}

/// Get the current 'Running' task's trap contexts.
pub fn current_trap_cx() -> &'static mut TrapContext {
    TASK_MANAGER.get_current_trap_cx()
}

/// Change the current 'Running' task's program break
pub fn change_program_brk(size: i32) -> Option<usize> {
    TASK_MANAGER.change_current_program_brk(size)
}

/// Map memory for the current task
pub fn mmap(start: usize, len: usize, prot: usize) -> isize {
    TASK_MANAGER.mmap_current(start, len, prot)
}

/// Unmap memory for the current task
pub fn munmap(start: usize, len: usize) -> isize {
    TASK_MANAGER.munmap_current(start, len)
}

/// Increment syscall count for current task
pub fn increment_syscall_count(syscall_id: usize) {
    TASK_MANAGER.increment_syscall_count(syscall_id)
}

/// Get syscall count for current task
pub fn get_syscall_count(syscall_id: usize) -> u32 {
    TASK_MANAGER.get_syscall_count(syscall_id)
}
