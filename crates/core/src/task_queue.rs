use std::cmp::Ordering;
use std::collections::{BinaryHeap, VecDeque};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TaskType {
    UserTask,
    InternalTask,
    IoTask,
    DelayedTask,
    PromiseTask,
}

pub struct Task {
    pub id: u64,
    pub task_type: TaskType,
    pub future: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for Task {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("id", &self.id)
            .field("task_type", &self.task_type)
            .finish()
    }
}

impl Clone for Task {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            task_type: self.task_type.clone(),
            future: None,
        }
    }
}

impl Task {
    pub fn new(id: u64, task_type: TaskType) -> Self {
        Self {
            id,
            task_type,
            future: None,
        }
    }

    pub fn with_future(id: u64, task_type: TaskType, future: tokio::task::JoinHandle<()>) -> Self {
        Self {
            id,
            task_type,
            future: Some(future),
        }
    }
}

#[derive(Debug)]
pub struct DelayedTask {
    pub id: u64,
    pub scheduled_at: std::time::Instant,
    pub task: Task,
}

impl Eq for DelayedTask {}

impl PartialEq for DelayedTask {
    fn eq(&self, other: &Self) -> bool {
        self.scheduled_at == other.scheduled_at && self.id == other.id
    }
}

impl Ord for DelayedTask {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .scheduled_at
            .cmp(&self.scheduled_at)
            .then_with(|| other.id.cmp(&self.id))
    }
}

impl PartialOrd for DelayedTask {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub struct TaskQueue {
    high_priority: VecDeque<Task>,
    normal_priority: VecDeque<Task>,
    low_priority: VecDeque<Task>,
    delayed: BinaryHeap<DelayedTask>,
    next_task_id: u64,
}

impl TaskQueue {
    pub fn new() -> Self {
        Self {
            high_priority: VecDeque::new(),
            normal_priority: VecDeque::new(),
            low_priority: VecDeque::new(),
            delayed: BinaryHeap::new(),
            next_task_id: 0,
        }
    }

    pub fn next_id(&mut self) -> u64 {
        let id = self.next_task_id;
        self.next_task_id += 1;
        id
    }

    pub fn push(&mut self, task: Task) {
        match task.task_type {
            TaskType::PromiseTask | TaskType::InternalTask => {
                self.high_priority.push_back(task);
            }
            TaskType::UserTask | TaskType::IoTask => {
                self.normal_priority.push_back(task);
            }
            TaskType::DelayedTask => {
                self.low_priority.push_back(task);
            }
        }
    }

    pub fn push_delayed(&mut self, delay: std::time::Duration, task: Task) {
        let scheduled_at = std::time::Instant::now() + delay;
        self.delayed.push(DelayedTask {
            id: task.id,
            scheduled_at,
            task,
        });
    }

    pub fn pop(&mut self) -> Option<Task> {
        if let Some(delayed) = self.delayed.peek()
            && delayed.scheduled_at <= std::time::Instant::now()
        {
            return self.delayed.pop().map(|dt| dt.task);
        }

        self.high_priority
            .pop_front()
            .or_else(|| self.normal_priority.pop_front())
            .or_else(|| self.low_priority.pop_front())
    }

    pub fn pending_count(&self) -> usize {
        self.high_priority.len()
            + self.normal_priority.len()
            + self.low_priority.len()
            + self.delayed.len()
    }

    pub fn has_ready_delayed(&self) -> bool {
        self.delayed
            .peek()
            .map(|d| d.scheduled_at <= std::time::Instant::now())
            .unwrap_or(false)
    }

    pub fn next_delayed_duration(&self) -> Option<std::time::Duration> {
        self.delayed.peek().map(|d| {
            let now = std::time::Instant::now();
            if d.scheduled_at > now {
                d.scheduled_at - now
            } else {
                std::time::Duration::ZERO
            }
        })
    }
}

impl Default for TaskQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_queue_priority() {
        let mut queue = TaskQueue::new();

        let promise_task = Task::new(queue.next_id(), TaskType::PromiseTask);
        let io_task = Task::new(queue.next_id(), TaskType::IoTask);
        let delayed_task = Task::new(queue.next_id(), TaskType::DelayedTask);

        queue.push(delayed_task.clone());
        queue.push(io_task.clone());
        queue.push(promise_task.clone());

        let first = queue.pop();
        assert!(first.is_some());
        assert_eq!(first.unwrap().task_type, TaskType::PromiseTask);

        let second = queue.pop();
        assert!(second.is_some());
        assert_eq!(second.unwrap().task_type, TaskType::IoTask);
    }

    #[test]
    fn test_delayed_task_scheduling() {
        let mut queue = TaskQueue::new();
        let delay = std::time::Duration::from_millis(100);

        let task = Task::new(queue.next_id(), TaskType::DelayedTask);
        queue.push_delayed(delay, task);

        assert!(!queue.has_ready_delayed());
    }
}
