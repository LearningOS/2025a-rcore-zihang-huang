//!Implementation of [`TaskManager`]
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;

/// BigStride constant for stride scheduling
const BIG_STRIDE: usize = 1024;

///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A stride scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take a process out of the ready queue using stride scheduling
    /// Find the task with minimum stride
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        if self.ready_queue.is_empty() {
            return None;
        }

        // Find the index of the task with minimum stride
        let mut min_stride = usize::MAX;
        let mut min_idx = 0;

        for (idx, task) in self.ready_queue.iter().enumerate() {
            let inner = task.inner_exclusive_access();
            if inner.stride < min_stride {
                min_stride = inner.stride;
                min_idx = idx;
            }
        }

        // Remove the task with minimum stride
        if let Some(task) = self.ready_queue.remove(min_idx) {
            // Update the task's stride
            let mut inner = task.inner_exclusive_access();
            let pass = BIG_STRIDE / inner.priority;
            inner.stride += pass;
            drop(inner);
            Some(task)
        } else {
            None
        }
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
