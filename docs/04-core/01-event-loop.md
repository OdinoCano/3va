# 01 - EVENT LOOP Y SCHEDULER

## 1.1 Visión General

El event loop de 3va está implementado sobre el runtime asíncrono Tokio de Rust, proporcionando un rendimiento superior al de Node.js gracias a la eficiencia del modelo de actores y la ejecución cooperativo.

## 1.2 Arquitectura del Event Loop

### 1.2.1 Componentes Principales

```
┌─────────────────────────────────────────────────────────────┐
│                      Tokio Runtime                         │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐        │
│  │  Scheduler  │  │   Reactor   │  │    Timer    │        │
│  │  (Waker)    │  │  (IO Ops)    │  │  (Delayed)  │        │
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

### 1.2.2 Ciclo de Ejecución

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

### 1.3.1 Tipos de Tareas

```rust
pub enum TaskType {
    /// Tareas de usuario JavaScript
    UserTask,
    /// Tareas internas del runtime
    InternalTask,
    /// Tareas de I/O asíncrono
    IoTask,
    /// Tareas diferidas (setTimeout)
    DelayedTask,
    /// Tareas de promesas
    PromiseTask,
}
```

### 1.3.2 Cola de Prioridades

```rust
pub struct TaskQueue {
    // Alta prioridad: Promesas (microtasks)
    high_priority: VecDeque<Task>,
    // Prioridad normal: I/O callbacks
    normal_priority: VecDeque<Task>,
    // Baja prioridad: setTimeout (macrotasks)
    low_priority: VecDeque<Task>,
    // Tareas diferidas
    delayed: BinaryHeap<DelayedTask>,
}
```

### 1.3.3 Algoritmo de Scheduling

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

## 1.4 Gestión de timers

### 1.4.1 Timer Wheel

Implementación optimizada de timers usando "timer wheel" para complejidad O(1):

```rust
pub struct TimerWheel {
    // 6 wheels para diferentes granularidades
    wheel_ms: VecDeque<Timer>,      // 1ms - 64ms
    wheel_s: VecDeque<Timer>,       // 64ms - 4s
    wheel_m: VecDeque<Timer>,       // 4s - 4min
    wheel_h: VecDeque<Timer>,       // 4min - 4hr
    wheel_d: VecDeque<Timer>,       // 4hr - 4days
    wheel_large: VecDeque<Timer>,   // > 4 days (heap)
}
```

### 1.4.2 API de Timers

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

## 1.5 Reactor de I/O

### 1.5.1 Operaciones Asíncronas

| Operacion | Rust Async | JS API |
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

## 1.6 Métricas de Rendimiento

### 1.6.1 Benchmarks Esperados

| Métrica | Node.js | Bun | 3va Target |
|---------|---------|-----|------------|
| Cold start | 50ms | 12ms | <15ms |
| Hello world | 25ms | 6ms | <10ms |
| Eval 1M ops | 180ms | 45ms | <40ms |
| Memory baseline | 30MB | 17MB | <20MB |

### 1.6.2 Monitoreo

```javascript
// Métricas del runtime
console.performance.memory;  // JS heap
process.resourceUsage();     // CPU, memory, IO
process.uptime();            // Tiempo de ejecución
```

## 1.7 Cumplimiento Normativo (ISO/IEC)

El diseño e implementación del Event Loop asíncrono y la gestión de tareas respeta los principios definidos en:
- **ISO/IEC 24765:2017** (Vocabulario de Ingeniería de Sistemas y Software), garantizando el determinismo en la ejecución de concurrencia y prevención de *race conditions* a nivel de runtime.
- **RFC 7230**, operando las conexiones y tareas asíncronas bajo semántica de streams de baja latencia.

---

*Implementado en `crates/core/src/task_queue.rs` y `crates/core/src/timer.rs`.*