//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{block_current_and_run_next, current_task, wakeup_task, TaskControlBlock};
use alloc::{collections::BTreeMap, collections::VecDeque, sync::Arc};

/// semaphore structure
pub struct Semaphore {
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

pub struct SemaphoreInner {
    pub count: isize,
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    pub sem_ownership: BTreeMap<usize, usize>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    sem_ownership: BTreeMap::new(),
                })
            },
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;

        let task = current_task().unwrap();
        let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
        if let Some(count) = inner.sem_ownership.get_mut(&tid) {
            if *count > 0 {
                *count -= 1;
            }
            if *count == 0 {
                inner.sem_ownership.remove(&tid);
            }
        }

        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                let woken_tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
                *inner.sem_ownership.entry(woken_tid).or_insert(0) += 1;
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        if inner.count < 0 {
            inner.wait_queue.push_back(current_task().unwrap());
            drop(inner);
            block_current_and_run_next();
        } else {
            let task = current_task().unwrap();
            let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
            *inner.sem_ownership.entry(tid).or_insert(0) += 1;
        }
    }
}
