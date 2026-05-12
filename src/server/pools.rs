//! Bounded blocking-task pools.
//!
//! `tokio::task::spawn_blocking` defaults to a 512-thread blocking
//! pool. Without bounding, one slow git operation (think:
//! `git clone` on a large repo) can fill the pool and starve fast
//! request handling. Wrapping `spawn_blocking` with a per-class
//! semaphore bounds concurrency so heavy work has its own headroom
//! and the request path stays responsive.
//!
//! Two classes:
//!   * `git`  — `git2` operations (clone, commit, push, pull, log).
//!   * `cpu`  — CPU-heavy work (zip pack/unpack, sha256 over big
//!              files, `ffmpeg-sidecar` invocations).
//!
//! Submission is async: callers `acquire_owned().await` the
//! semaphore permit, then dispatch to `spawn_blocking`. The permit
//! is held by the spawned task and released when the task returns.

use std::sync::Arc;
use tokio::sync::{AcquireError, Semaphore};
use tokio::task::JoinHandle;

pub struct BlockingPools {
    pub git: Arc<Semaphore>,
    pub cpu: Arc<Semaphore>,
}

impl BlockingPools {
    pub fn new() -> Self {
        Self::with_capacities(16, num_cpus::get().max(2))
    }

    pub fn with_capacities(git: usize, cpu: usize) -> Self {
        Self {
            git: Arc::new(Semaphore::new(git)),
            cpu: Arc::new(Semaphore::new(cpu)),
        }
    }

    /// Run a synchronous function on the git pool. Awaits a permit
    /// (queues if all slots are full), then spawns on Tokio's
    /// blocking pool.
    pub async fn run_git<F, R>(&self, f: F) -> Result<JoinHandle<R>, AcquireError>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let permit = self.git.clone().acquire_owned().await?;
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f()
        }))
    }

    /// Run a synchronous function on the CPU pool. Same shape as
    /// `run_git`, separate semaphore so git contention doesn't
    /// block CPU-bound work and vice versa.
    pub async fn run_cpu<F, R>(&self, f: F) -> Result<JoinHandle<R>, AcquireError>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let permit = self.cpu.clone().acquire_owned().await?;
        Ok(tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f()
        }))
    }

    /// Available permits, for metrics. Cheap; lock-free.
    pub fn git_available(&self) -> usize {
        self.git.available_permits()
    }
    pub fn cpu_available(&self) -> usize {
        self.cpu.available_permits()
    }
}

impl Default for BlockingPools {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn git_pool_runs_work() {
        let pools = BlockingPools::with_capacities(2, 2);
        let handle = pools.run_git(|| 42).await.unwrap();
        assert_eq!(handle.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn cpu_pool_runs_work() {
        let pools = BlockingPools::with_capacities(2, 2);
        let handle = pools.run_cpu(|| 7 * 6).await.unwrap();
        assert_eq!(handle.await.unwrap(), 42);
    }

    #[tokio::test]
    async fn pool_bounds_in_flight() {
        let pools = BlockingPools::with_capacities(1, 1);
        // Block the single git slot.
        let blocker = pools
            .run_git(|| std::thread::sleep(std::time::Duration::from_millis(50)))
            .await
            .unwrap();
        // Available now zero.
        assert_eq!(pools.git_available(), 0);
        // Free it.
        blocker.await.unwrap();
        // Permit released.
        assert_eq!(pools.git_available(), 1);
    }
}
