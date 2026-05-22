# 01 - EVENT LOOP AND SCHEDULER

## 1.1 Overview

3va's event loop is implemented on top of Rust's Tokio async runtime, providing superior performance compared to Node.js thanks to the efficiency of the actor model and cooperative execution.

## 1.2 Event Loop Architecture

### 1.2.1 Main Components

```
┌─────────────────────────────────────────────────────────────┐
│                      Tokio Runtime                         │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │  Scheduler  │  │   Reactor   │  │    Timer    │        │
│  │  (Waker)    │  │  (IO Ops)   │  │  (Delayed)  │        │
│  └─────────────┘  └─────────────┘  └─────────────┘        │
├─────────────────────────────────────────────────────────────┤
│                      3va Event Loop                         │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │   Task      │  │   Promise   │  │    Event    │        │
│  │  Queue      │  │   Queue     │  │   Handler   │        │
│  └─────────────┘  └─────────────┘  └─────────────┘        │
└─────────────────────────────────────────────────────────────┘
```

### 1.2.2 Execution Cycle

```
┌────────────────┐
│    polling     │ ◄── Phase 1: I/O events
│   (poll IO)    │
└───────┬────────┘
        │ I/O ready
        ▼
┌────────────────┐
│   callbacks    │ ◄── Phase 2: Execute callbacks
│  (microtasks)  │
└───────┬────────┘
        │ empty
        ▼
┌────────────────┐
│    timers      │ ◄── Phase 3: setTimeout/setImmediate
│  (macrotasks)  │
└───────┬────────┘
        │ complete
        ▼
┌────────────────┐
│    idle        │ ◄── Phase 4: Idle (nextTick)
│   (nextTick)   │
└───────┬────────┘
        │ loop
        ▼
┌────────────────┐
│    polling     │ ◄── Back to Phase 1
└────────────────┘
```

## 1.3 Task Scheduler

### 1.3.1 Task Types

```rust
pub enum TaskType {
    /// JavaScript user tasks
    UserTask,
    /// Runtime internal tasks
    InternalTask,
    /// Async I/O tasks
    IoTask,
    /// Delayed tasks (setTimeout)
    DelayedTask,
    /// Promise tasks
    PromiseTask,
}
```

### 1.3.2 Priority Queue

```rust
pub struct TaskQueue {
    // High priority: Promises (microtasks)
    high_priority: VecDeque<Task>,
    // Normal priority: I/O callbacks
    normal_priority: VecDeque<Task>,
    // Low priority: setTimeout (macrotasks)
    low_priority: VecDeque<Task>,
    // Delayed tasks
    delayed: BinaryHeap<DelayedTask>,
}
```

### 1.3.3 Scheduling Algorithm

```
1. Receive new task
2. Classify by TaskType
3. Add to appropriate queue
4. If running task yields:
   - Check high_priority first
   - Then normal_priority
   - Finally low_priority
5. Execute in FIFO order within queue
```

## 1.4 Timer Management

### 1.4.1 Timer Wheel

Optimized timer implementation using "timer wheel" for O(1) complexity:

```rust
pub struct TimerWheel {
    // 6 wheels for different granularities
    wheel_ms: VecDeque<Timer>,      // 1ms - 64ms
    wheel_s: VecDeque<Timer>,       // 64ms - 4s
    wheel_m: VecDeque<Timer>,       // 4s - 4min
    wheel_h: VecDeque<Timer>,       // 4min - 4hr
    wheel_d: VecDeque<Timer>,       // 4hr - 4days
    wheel_large: VecDeque<Timer>,   // > 4 days (heap)
}
```

### 1.4.2 Timer API

```javascript
// setTimeout
setTimeout(callback, delay, ...args)

// setInterval
setInterval(callback, delay, ...args)

// setImmediate (phase-specific)
setImmediate(callback)

// process.nextTick (highest priority)
process.nextTick(callback)
```

## 1.5 I/O Reactor

### 1.5.1 Async Operations

| Operation | Rust Async | JS API |
|-----------|------------|--------|
| File I/O | tokio::fs | fs promisified |
| TCP | tokio::net | net, http |
| UDP | tokio::net | dgram |
| DNS | tokio::dns | dns |
| Pipes | tokio::process | child_process |

### 1.5.2 multiplexing

```rust
pub struct IoReactor {
    // Reactor polls multiple I/O sources
    // Returns when any is ready
    // Dispatches to appropriate handler
}
```

## 1.6 Performance Metrics

### 1.6.1 Expected Benchmarks

| Metric | Node.js | Bun | 3va Target |
|--------|---------|-----|------------|
| Cold start | 50ms | 12ms | <15ms |
| Hello world | 25ms | 6ms | <10ms |
| Eval 1M ops | 180ms | 45ms | <40ms |
| Memory baseline | 30MB | 17MB | <20MB |

### 1.6.2 Monitoring

```javascript
// Runtime metrics
console.performance.memory;  // JS heap
process.resourceUsage();     // CPU, memory, IO
process.uptime();            // Runtime
```

## 1.7 Regulatory Compliance (ISO/IEC)

The design and implementation of the async Event Loop and task management respects the principles defined in:
- **ISO/IEC 24765:2017** (Systems and Software Engineering Vocabulary), guaranteeing determinism in concurrent execution and prevention of race conditions at the runtime level.
- **RFC 7230**, operating connections and async tasks under low-latency stream semantics.

---

*Implemented in `crates/core/src/task_queue.rs` and `crates/core/src/timer.rs`.*
