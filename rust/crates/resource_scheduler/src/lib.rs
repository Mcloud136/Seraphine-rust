use serde::Serialize;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::Notify;

pub const MAX_ACTIVE_TASKS: usize = 32;
pub const MAX_INFLIGHT_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameState {
    #[default]
    NotRunning,
    ClientStarting,
    Lobby,
    ChampSelect,
    InGame,
    EndOfGame,
    Reconnect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    P0,
    P1,
    P2,
    P3,
    P4,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScheduleError {
    DeferredByGameState,
    ActiveTaskLimit,
    ByteBudget,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SchedulerSnapshot {
    pub game_state: GameState,
    pub active_tasks: usize,
    pub active_bytes: usize,
    pub acquired_total: u64,
    pub deferred_total: u64,
    pub rejected_total: u64,
}

struct State {
    game_state: AtomicU8,
    active_tasks: AtomicU64,
    active_bytes: AtomicU64,
    acquired_total: AtomicU64,
    deferred_total: AtomicU64,
    rejected_total: AtomicU64,
    slots: Mutex<[usize; 5]>,
    notify: Notify,
}

impl State {
    fn new() -> Self {
        Self {
            game_state: AtomicU8::new(GameState::NotRunning as u8),
            active_tasks: AtomicU64::new(0),
            active_bytes: AtomicU64::new(0),
            acquired_total: AtomicU64::new(0),
            deferred_total: AtomicU64::new(0),
            rejected_total: AtomicU64::new(0),
            slots: Mutex::new([0; 5]),
            notify: Notify::new(),
        }
    }

    fn game_state(&self) -> GameState {
        match self.game_state.load(Ordering::Relaxed) {
            1 => GameState::ClientStarting,
            2 => GameState::Lobby,
            3 => GameState::ChampSelect,
            4 => GameState::InGame,
            5 => GameState::EndOfGame,
            6 => GameState::Reconnect,
            _ => GameState::NotRunning,
        }
    }

    fn is_allowed(&self, priority: Priority) -> bool {
        matches!(
            (self.game_state(), priority),
            (_, Priority::P0 | Priority::P1 | Priority::P2)
                | (
                    GameState::NotRunning | GameState::ClientStarting | GameState::EndOfGame,
                    _
                )
                | (GameState::Lobby, Priority::P3)
                | (GameState::Reconnect, Priority::P3)
        )
    }

    fn max_active(&self, priority: Priority) -> usize {
        match (self.game_state(), priority) {
            (GameState::InGame, Priority::P2) => 2,
            (GameState::ChampSelect, Priority::P2) => 4,
            (GameState::InGame | GameState::ChampSelect, Priority::P3) => 0,
            (_, Priority::P0) => 8,
            (_, Priority::P1) => 8,
            (_, Priority::P2) => 8,
            (_, Priority::P3) => 2,
            (_, Priority::P4) => 1,
        }
    }
}

#[derive(Clone)]
pub struct ResourceScheduler {
    state: Arc<State>,
}

impl ResourceScheduler {
    pub fn new() -> Self {
        Self {
            state: Arc::new(State::new()),
        }
    }

    pub fn set_game_state(&self, game_state: GameState) {
        self.state
            .game_state
            .store(game_state as u8, Ordering::Relaxed);
    }

    pub fn game_state(&self) -> GameState {
        self.state.game_state()
    }

    pub fn allows_prefetch(&self) -> bool {
        matches!(
            self.game_state(),
            GameState::NotRunning | GameState::ClientStarting | GameState::EndOfGame
        )
    }

    pub fn try_acquire(
        &self,
        priority: Priority,
        estimated_bytes: usize,
    ) -> Result<ResourcePermit, ScheduleError> {
        if !self.state.is_allowed(priority) {
            self.state.deferred_total.fetch_add(1, Ordering::Relaxed);
            return Err(ScheduleError::DeferredByGameState);
        }

        let mut slots = self.state.slots.lock().unwrap();
        let index = priority as usize;
        let active_tasks = self.state.active_tasks.load(Ordering::Relaxed) as usize;
        let active_bytes = self.state.active_bytes.load(Ordering::Relaxed) as usize;
        if active_tasks >= MAX_ACTIVE_TASKS
            || slots[index] >= self.state.max_active(priority)
            || active_bytes.saturating_add(estimated_bytes) > MAX_INFLIGHT_BYTES
        {
            self.state.rejected_total.fetch_add(1, Ordering::Relaxed);
            return Err(
                if active_bytes.saturating_add(estimated_bytes) > MAX_INFLIGHT_BYTES {
                    ScheduleError::ByteBudget
                } else {
                    ScheduleError::ActiveTaskLimit
                },
            );
        }

        slots[index] += 1;
        self.state.active_tasks.fetch_add(1, Ordering::Relaxed);
        self.state
            .active_bytes
            .fetch_add(estimated_bytes as u64, Ordering::Relaxed);
        self.state.acquired_total.fetch_add(1, Ordering::Relaxed);
        drop(slots);

        Ok(ResourcePermit {
            state: Arc::clone(&self.state),
            priority,
            estimated_bytes,
        })
    }

    /// Acquires a permit, waiting for byte capacity when the budget is temporarily full.
    pub async fn acquire(
        &self,
        priority: Priority,
        estimated_bytes: usize,
    ) -> Result<ResourcePermit, ScheduleError> {
        if estimated_bytes > MAX_INFLIGHT_BYTES {
            return Err(ScheduleError::ByteBudget);
        }

        loop {
            let notified = self.state.notify.notified();
            match self.try_acquire(priority, estimated_bytes) {
                Ok(permit) => return Ok(permit),
                Err(ScheduleError::ByteBudget) => notified.await,
                Err(error) => return Err(error),
            }
        }
    }

    pub fn snapshot(&self) -> SchedulerSnapshot {
        SchedulerSnapshot {
            game_state: self.game_state(),
            active_tasks: self.state.active_tasks.load(Ordering::Relaxed) as usize,
            active_bytes: self.state.active_bytes.load(Ordering::Relaxed) as usize,
            acquired_total: self.state.acquired_total.load(Ordering::Relaxed),
            deferred_total: self.state.deferred_total.load(Ordering::Relaxed),
            rejected_total: self.state.rejected_total.load(Ordering::Relaxed),
        }
    }
}

impl Default for ResourceScheduler {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ResourcePermit {
    state: Arc<State>,
    priority: Priority,
    estimated_bytes: usize,
}

impl Drop for ResourcePermit {
    fn drop(&mut self) {
        if let Ok(mut slots) = self.state.slots.lock() {
            let index = self.priority as usize;
            slots[index] = slots[index].saturating_sub(1);
        }
        self.state.active_tasks.fetch_sub(1, Ordering::Relaxed);
        self.state
            .active_bytes
            .fetch_sub(self.estimated_bytes as u64, Ordering::Relaxed);
        self.state.notify.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_game_defers_background_work() {
        let scheduler = ResourceScheduler::new();
        scheduler.set_game_state(GameState::InGame);
        assert!(matches!(
            scheduler.try_acquire(Priority::P4, 1),
            Err(ScheduleError::DeferredByGameState)
        ));
        assert!(scheduler.try_acquire(Priority::P0, 1).is_ok());
        assert!(scheduler.try_acquire(Priority::P1, 1).is_ok());
    }

    #[test]
    fn permit_releases_budget() {
        let scheduler = ResourceScheduler::new();
        {
            let _permit = scheduler.try_acquire(Priority::P2, 100).unwrap();
            assert_eq!(scheduler.snapshot().active_bytes, 100);
        }
        assert_eq!(scheduler.snapshot().active_bytes, 0);
    }

    #[test]
    fn rejects_over_budget_task() {
        let scheduler = ResourceScheduler::new();
        assert!(matches!(
            scheduler.try_acquire(Priority::P2, MAX_INFLIGHT_BYTES + 1),
            Err(ScheduleError::ByteBudget)
        ));
    }

    #[tokio::test]
    async fn acquire_waits_for_byte_budget_release() {
        let scheduler = ResourceScheduler::new();
        let blocker = scheduler
            .try_acquire(Priority::P2, MAX_INFLIGHT_BYTES)
            .unwrap();
        let waiter = scheduler.clone();
        let task = tokio::spawn(async move { waiter.acquire(Priority::P2, 1).await });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());

        drop(blocker);
        let permit = tokio::time::timeout(std::time::Duration::from_millis(100), task)
            .await
            .unwrap()
            .unwrap();
        assert!(permit.is_ok());
    }
}
