# Lab 3 实验报告

## 功能实现总结

本次实验首先迁移了上一章的sys_get_time、sys_mmap、sys_munmap系统调用以适应新的进程结构，不再维护sys_trace调用。实现了spawn系统调用（ID 400），该调用可直接创建新进程并执行指定程序，与fork不同的是spawn不复制父进程地址空间，而是为子进程建立全新的地址空间并加载目标程序。实现了stride调度算法，在TaskControlBlock中新增priority和stride字段，初始优先级设为16，初始stride设为0，调度时选择stride最小的进程执行并将其stride增加pass值（pass = BIG_STRIDE / priority）。实现了sys_set_priority系统调用（ID 140），支持动态设置进程优先级，要求优先级大于等于2，成功返回设置的优先级值，参数非法返回-1。

## 问答作业

### stride 算法深入

实际情况与溢出问题

实际情况下不会轮到p1执行，而是p2会继续执行。因为使用8位无符号整数存储stride时，p2的stride从250变为260会发生溢出，实际存储的值变成260 mod 256 = 4。此时比较p1的stride（255）和p2的stride（4），由于4小于255，调度器会选择p2执行，这与理论预期不符。这就是stride算法在溢出情况下可能出现的调度错误。

stride差值的上界证明

在不考虑溢出的情况下，可以证明STRIDE_MAX - STRIDE_MIN不超过BigStride / 2。简单说明如下：假设当前所有可运行进程中stride最大的是A，stride最小的是B。因为每次调度都选择stride最小的进程执行，所以B刚刚被选中执行。在B被选中之前，B的stride一定是最小的，此时A和B的stride差值记为diff_before。B执行后stride增加pass_B = BigStride / priority_B，因为priority_B大于等于2，所以pass_B最多为BigStride / 2。此时A和B的stride差值diff_after = diff_before - pass_B。如果diff_after仍为正，说明A的stride仍然更大；如果diff_after为负，则B的stride变成了最大的。在整个调度过程中，因为每次选择最小stride执行，最大stride和最小stride的差值只会在最小stride被调度时可能增加，增加量最多为BigStride / 2，而在其他情况下差值会减小。因此最大差值不会超过BigStride / 2。

考虑溢出的Stride比较器实现

基于STRIDE_MAX - STRIDE_MIN不超过BigStride / 2这个性质，可以设计特殊的比较器处理溢出。当两个stride的差值小于BigStride / 2时，可以认为较小的那个确实更小；当差值大于BigStride / 2时，说明发生了溢出，此时应该反转比较结果。具体实现如下：

```rust
use core::cmp::Ordering;

struct Stride(u64);

impl PartialOrd for Stride {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        const BIG_STRIDE: u64 = u64::MAX;
        let diff = self.0.wrapping_sub(other.0);
        if diff == 0 {
            Some(Ordering::Equal)
        } else if diff < BIG_STRIDE / 2 {
            Some(Ordering::Greater)
        } else {
            Some(Ordering::Less)
        }
    }
}

impl PartialEq for Stride {
    fn eq(&self, other: &Self) -> bool {
        false
    }
}
```

这个实现使用wrapping_sub计算两个stride的差值，避免溢出导致panic。当self - other的结果小于BIG_STRIDE / 2时，说明self更大；当差值大于等于BIG_STRIDE / 2时，说明发生了溢出，实际上other更大。对于8位存储的例子，BIG_STRIDE为255，125与255的差值为125 - 255 = -130，取模后为126，大于127（255/2），因此125 < 255判断为false；129与255的差值为129 - 255 = -126，取模后为130，大于127，但在另一个方向上，255 - 129 = 126，小于127，因此255 > 129判断为true，即129 < 255判断为true。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

> 无

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

> 无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按"-100"分计。
