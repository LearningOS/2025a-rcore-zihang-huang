## 功能实现总结

本次实验实现了 `sys_trace` 系统调用（ID 410），用于追踪和操作任务的系统调用信息。主要修改包括：

1. 在 `TaskControlBlock` 中添加了 `syscall_count` 数组（500个元素）来记录每个系统调用的调用次数
2. 在 `syscall` 函数中添加计数逻辑，每次系统调用前自动增加对应计数器
3. 实现 `sys_trace` 的三种功能模式：
   - 模式0：读取指定地址的一个字节
   - 模式1：向指定地址写入一个字节
   - 模式2：查询指定系统调用ID的调用次数（包含本次调用）
4. 添加了辅助函数用于访问任务的系统调用统计信息和内存空间

实现通过了所有基础测例和实验测例的测试。

---

## 简答

### 1. U态特权级别测试

在正确进入U态后，程序如果使用S态特权指令或访问S态寄存器会触发异常。运行三个bad测例的结果如下：

测试环境：

- SBI：RustSBI version 0.3.0-alpha.2
- 实现：RustSBI-QEMU Version 0.2.0-alpha.2
- SBI规范版本：RISC-V SBI v1.0.0

测试结果：

1. ch2b_bad_address：尝试访问非法地址（0x0）

   ```
   [kernel] PageFault in application, bad addr = 0x0, bad instruction = 0x804003a4, kernel killed it.
   ```

   - 错误类型：PageFault（页面错误）
   - 错误地址：0x0（空指针）
   - 错误指令地址：0x804003a4
   - 处理结果：内核捕获异常并终止应用程序

2. ch2b_bad_instructions：尝试执行特权指令

   ```
   [kernel] IllegalInstruction in application, kernel killed it.
   ```

   - 错误类型：IllegalInstruction（非法指令异常）
   - 原因：用户态程序尝试执行S态特权指令
   - 处理结果：内核捕获异常并终止应用程序

3. ch2b_bad_register：尝试访问S态寄存器

   ```
   [kernel] IllegalInstruction in application, kernel killed it.
   ```

   - 错误类型：IllegalInstruction（非法指令异常）
   - 原因：用户态程序尝试访问S态CSR寄存器
   - 处理结果：内核捕获异常并终止应用程序

这些测试验证了特权级别隔离机制的有效性：用户态程序无法执行特权指令或访问特权寄存器，任何违规操作都会触发异常并被内核捕获处理。

---

### 2. trap.S 深入分析

sp 的值：
刚进入 `__restore` 时，`sp` 指向内核栈上的 `TrapContext` 结构起始地址（已分配34*8字节空间后的位置）。

两种使用场景

1. 从trap_handler返回（异常/中断处理后）：
   - 在 `__alltraps` 中保存了用户态上下文到内核栈
   - 调用 `trap_handler` 处理异常
   - `trap_handler` 返回后直接执行 `__restore`
   - 此时 sp 指向刚才保存的 TrapContext

2. 任务切换后首次执行（通过 `__switch` 跳转）：
   - 在任务初始化或切换时，通过 `__switch` 切换到新任务
   - 新任务的 `TaskContext` 的 ra 被设置为 `__restore` 的地址
   - `__switch` 返回时会跳转到 `__restore`
   - 此时 sp 指向该任务预先设置好的 TrapContext

#### 2.2 L43-L48：特殊寄存器的处理及意义

```riscv
ld t0, 32*8(sp)    # 从TrapContext加载sstatus
ld t1, 33*8(sp)    # 从TrapContext加载sepc
ld t2, 2*8(sp)     # 从TrapContext加载用户栈指针
csrw sstatus, t0   # 恢复sstatus
csrw sepc, t1      # 恢复sepc
csrw sscratch, t2  # 恢复sscratch（保存用户栈指针）
```

1. sstatus（Supervisor Status Register）：
   - 保存处理器状态信息，包括特权级别、中断使能等
   - 最关键的是 SPP（Previous Privilege）位：记录trap前的特权级别
   - 当 SPP=0 时，`sret` 指令会切换到用户态（U模式）
   - 当 SPP=1 时，`sret` 指令会保持在内核态（S模式）
   - 恢复 sstatus 确保返回到正确的特权级别

2. sepc（Supervisor Exception Program Counter）：
   - 保存触发异常的指令地址或返回地址
   - 对于系统调用：保存 `ecall` 指令的下一条指令地址
   - `sret` 指令会跳转到 sepc 指向的地址继续执行
   - 恢复 sepc 确保程序从正确的位置继续执行

3. sscratch（Supervisor Scratch Register）：
   - 用于在内核栈和用户栈指针之间切换的临时寄存器
   - 在内核态时保存用户栈指针
   - 在用户态时保存内核栈指针
   - 通过 `csrrw` 指令实现栈指针的原子交换
   - 恢复 sscratch（x2的值）为用户栈指针，为后续交换做准备

#### 2.3 L50-L56：为何跳过 x2 和 x4

```riscv
ld x1, 1*8(sp)
ld x3, 3*8(sp)
.set n, 5
.rept 27
   LOAD_GP %n
   .set n, n+1
.endr
```

跳过的原因：

1. x2（sp，栈指针）：
   - x2 需要特殊处理，因为它在整个trap处理过程中用于访问 TrapContext
   - 如果现在恢复 x2，后续指令将无法通过 sp 访问 TrapContext 中的数据
   - x2 的恢复通过 L60 的 `csrrw sp, sscratch, sp` 完成，这是最后一步操作
   - 在 L60 之前，sp 必须保持指向 TrapContext（释放后的内核栈顶）

2. x4（tp，线程指针）：
   - tp 寄存器用于保存线程局部存储（TLS）指针
   - 在当前的 ch3 实现中，用户程序不使用线程指针
   - 跳过可以节省保存/恢复开销
   - 注释明确说明："skip tp(x4), application does not use it"

#### 2.4 L60：csrrw 指令后 sp 和 sscratch 的意义

```riscv
csrrw sp, sscratch, sp
```

执行后的状态：

- sp（x2）：指向用户栈
  - 交换前 sp 指向内核栈顶（TrapContext已释放）
  - 交换后 sp 恢复为用户栈指针，用户程序可以正常使用栈

- sscratch：指向内核栈
  - 交换前 sscratch 保存用户栈指针
  - 交换后 sscratch 保存内核栈指针，为下次trap做准备
  - 当下次trap发生时，`__alltraps` 的第一条指令 `csrrw sp, sscratch, sp` 会再次交换，切换回内核栈

意义：

- 完成了内核栈和用户栈的切换
- 恢复了用户态的执行环境
- 为下次trap做好了准备（sscratch保存了内核栈指针）

#### 2.5 `__restore` 中状态切换的指令

```riscv
sret
```

1. sret（Supervisor Return）指令的功能：
   - 从S态返回到先前的特权级别
   - 将 pc 设置为 sepc 的值（跳转到用户程序）
   - 根据 sstatus 的 SPP 位决定返回到哪个特权级别

2. 进入用户态的原因：
   - 在之前恢复的 sstatus 中，SPP 位被设置为 0
   - SPP=0 表示 trap 前处于用户态
   - sret 读取 SPP=0，因此将处理器切换到用户态（U模式）
   - 同时 sstatus.SIE（中断使能）从 SPIE 恢复

3. 完整的状态切换：
   - 特权级别：S模式（内核态）→ U模式（用户态）
   - PC：当前指令 → sepc 保存的地址
   - 栈指针：已在 L60 切换为用户栈
   - 通用寄存器：已全部恢复为用户态的值

#### 2.6 L13：csrrw 指令后 sp 和 sscratch 的意义

```riscv
csrrw sp, sscratch, sp
```

执行后的状态：

- sp（x2）：指向内核栈
  - 交换前 sp 指向用户栈（trap 发生时正在使用）
  - 交换后 sp 指向内核栈，可以安全地保存 TrapContext
  - 后续指令使用内核栈来保存用户态的所有寄存器

- sscratch：保存用户栈指针
  - 交换前 sscratch 保存内核栈指针（在上次 `__restore` 时设置）
  - 交换后 sscratch 保存用户栈指针，以便后续恢复
  - 在 L34-L35 会将 sscratch 的值保存到 TrapContext 的 x2 位置

意义：

- 完成了用户栈到内核栈的切换，确保trap处理在内核栈上进行
- 保存了用户栈指针，以便在 `__restore` 时恢复
- 这是trap处理的第一步，为后续保存上下文提供了安全的栈空间

#### 2.7 从 U 态进入 S 态的指令

答案：用户程序中的 `ecall` 指令
---

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

> 无

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

> 无

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。
