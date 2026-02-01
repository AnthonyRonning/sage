//! Scheduler for delayed and recurring tasks
//!
//! Supports:
//! - One-off scheduled messages or tool calls
//! - Recurring tasks via cron expressions
//! - PostgreSQL-backed persistence

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use cron::Schedule;
use diesel::pg::PgConnection;
use diesel::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::schema::scheduled_tasks;

// ============================================================================
// Types
// ============================================================================

/// Task type - what kind of action to perform
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskType {
    Message,
    ToolCall,
}

impl TaskType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskType::Message => "message",
            TaskType::ToolCall => "tool_call",
        }
    }
}

impl FromStr for TaskType {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "message" => Ok(TaskType::Message),
            "tool_call" => Ok(TaskType::ToolCall),
            _ => Err(anyhow::anyhow!(
                "Invalid task type: {}. Must be 'message' or 'tool_call'",
                s
            )),
        }
    }
}

/// Task status
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }
}

impl FromStr for TaskStatus {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(TaskStatus::Pending),
            "running" => Ok(TaskStatus::Running),
            "completed" => Ok(TaskStatus::Completed),
            "failed" => Ok(TaskStatus::Failed),
            "cancelled" => Ok(TaskStatus::Cancelled),
            _ => Err(anyhow::anyhow!("Invalid task status: {}", s)),
        }
    }
}

/// Payload for a message task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessagePayload {
    pub message: String,
}

/// Payload for a tool call task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallPayload {
    pub tool: String,
    pub args: HashMap<String, String>,
}

/// Union of possible payloads
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TaskPayload {
    Message(MessagePayload),
    ToolCall(ToolCallPayload),
}

/// A scheduled task
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScheduledTask {
    pub id: Uuid,
    pub agent_id: Uuid,
    pub task_type: TaskType,
    pub payload: TaskPayload,
    pub next_run_at: DateTime<Utc>,
    pub cron_expression: Option<String>,
    pub timezone: String,
    pub status: TaskStatus,
    pub last_run_at: Option<DateTime<Utc>>,
    pub run_count: i32,
    pub last_error: Option<String>,
    pub description: String,
    pub created_at: DateTime<Utc>,
}

/// Diesel model for inserting a new task
#[derive(Insertable)]
#[diesel(table_name = scheduled_tasks)]
struct NewScheduledTask {
    id: Uuid,
    agent_id: Uuid,
    task_type: String,
    payload: serde_json::Value,
    next_run_at: DateTime<Utc>,
    cron_expression: Option<String>,
    timezone: String,
    status: String,
    description: String,
}

/// Diesel model for querying tasks
#[derive(Queryable, Debug)]
struct ScheduledTaskRow {
    id: Uuid,
    agent_id: Uuid,
    task_type: String,
    payload: serde_json::Value,
    next_run_at: DateTime<Utc>,
    cron_expression: Option<String>,
    timezone: String,
    status: String,
    last_run_at: Option<DateTime<Utc>>,
    run_count: i32,
    last_error: Option<String>,
    description: String,
    created_at: DateTime<Utc>,
}

impl TryFrom<ScheduledTaskRow> for ScheduledTask {
    type Error = anyhow::Error;

    fn try_from(row: ScheduledTaskRow) -> Result<Self> {
        let task_type = TaskType::from_str(&row.task_type)?;
        let payload: TaskPayload =
            serde_json::from_value(row.payload).context("Failed to parse task payload")?;
        let status = TaskStatus::from_str(&row.status)?;

        Ok(ScheduledTask {
            id: row.id,
            agent_id: row.agent_id,
            task_type,
            payload,
            next_run_at: row.next_run_at,
            cron_expression: row.cron_expression,
            timezone: row.timezone,
            status,
            last_run_at: row.last_run_at,
            run_count: row.run_count,
            last_error: row.last_error,
            description: row.description,
            created_at: row.created_at,
        })
    }
}

// ============================================================================
// Database Operations
// ============================================================================

pub struct SchedulerDb {
    conn: Arc<Mutex<PgConnection>>,
}

#[allow(dead_code)]
impl SchedulerDb {
    /// Create a new SchedulerDb with a shared connection
    pub fn new(conn: Arc<Mutex<PgConnection>>) -> Self {
        Self { conn }
    }

    /// Create a new SchedulerDb with its own connection
    pub fn connect(db_url: &str) -> Result<Self> {
        let conn = PgConnection::establish(db_url).context("Failed to connect to database")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create a new scheduled task
    #[allow(clippy::too_many_arguments)]
    pub fn create_task(
        &self,
        agent_id: Uuid,
        task_type: TaskType,
        payload: TaskPayload,
        next_run_at: DateTime<Utc>,
        cron_expression: Option<String>,
        timezone: String,
        description: String,
    ) -> Result<ScheduledTask> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let id = Uuid::new_v4();
        let payload_json = serde_json::to_value(&payload)?;

        let new_task = NewScheduledTask {
            id,
            agent_id,
            task_type: task_type.as_str().to_string(),
            payload: payload_json,
            next_run_at,
            cron_expression: cron_expression.clone(),
            timezone: timezone.clone(),
            status: TaskStatus::Pending.as_str().to_string(),
            description: description.clone(),
        };

        diesel::insert_into(scheduled_tasks::table)
            .values(&new_task)
            .execute(&mut *conn)
            .context("Failed to insert scheduled task")?;

        Ok(ScheduledTask {
            id,
            agent_id,
            task_type,
            payload,
            next_run_at,
            cron_expression,
            timezone,
            status: TaskStatus::Pending,
            last_run_at: None,
            run_count: 0,
            last_error: None,
            description,
            created_at: Utc::now(),
        })
    }

    /// Get all due tasks (pending and next_run_at <= now)
    pub fn get_due_tasks(&self) -> Result<Vec<ScheduledTask>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let rows: Vec<ScheduledTaskRow> = scheduled_tasks::table
            .filter(scheduled_tasks::status.eq("pending"))
            .filter(scheduled_tasks::next_run_at.le(Utc::now()))
            .order(scheduled_tasks::next_run_at.asc())
            .load(&mut *conn)
            .context("Failed to query due tasks")?;

        rows.into_iter().map(ScheduledTask::try_from).collect()
    }

    /// Get tasks by agent and optional status filter
    pub fn get_tasks_by_agent(
        &self,
        agent_id: Uuid,
        status_filter: Option<&str>,
    ) -> Result<Vec<ScheduledTask>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let mut query = scheduled_tasks::table
            .filter(scheduled_tasks::agent_id.eq(agent_id))
            .into_boxed();

        if let Some(status) = status_filter {
            query = query.filter(scheduled_tasks::status.eq(status));
        }

        let rows: Vec<ScheduledTaskRow> = query
            .order(scheduled_tasks::next_run_at.asc())
            .load(&mut *conn)
            .context("Failed to query tasks")?;

        rows.into_iter().map(ScheduledTask::try_from).collect()
    }

    /// Get a task by ID
    pub fn get_task(&self, task_id: Uuid) -> Result<Option<ScheduledTask>> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let row: Option<ScheduledTaskRow> = scheduled_tasks::table
            .filter(scheduled_tasks::id.eq(task_id))
            .first(&mut *conn)
            .optional()
            .context("Failed to query task")?;

        row.map(ScheduledTask::try_from).transpose()
    }

    /// Mark a task as running
    pub fn mark_running(&self, task_id: Uuid) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        diesel::update(scheduled_tasks::table.filter(scheduled_tasks::id.eq(task_id)))
            .set(scheduled_tasks::status.eq("running"))
            .execute(&mut *conn)
            .context("Failed to mark task as running")?;

        Ok(())
    }

    /// Mark a task as completed (for one-off tasks)
    pub fn mark_completed(&self, task_id: Uuid) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        diesel::update(scheduled_tasks::table.filter(scheduled_tasks::id.eq(task_id)))
            .set((
                scheduled_tasks::status.eq("completed"),
                scheduled_tasks::last_run_at.eq(Utc::now()),
                scheduled_tasks::run_count.eq(scheduled_tasks::run_count + 1),
            ))
            .execute(&mut *conn)
            .context("Failed to mark task as completed")?;

        Ok(())
    }

    /// Update a recurring task with next run time
    pub fn update_next_run(&self, task_id: Uuid, next_run_at: DateTime<Utc>) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        diesel::update(scheduled_tasks::table.filter(scheduled_tasks::id.eq(task_id)))
            .set((
                scheduled_tasks::status.eq("pending"),
                scheduled_tasks::next_run_at.eq(next_run_at),
                scheduled_tasks::last_run_at.eq(Utc::now()),
                scheduled_tasks::run_count.eq(scheduled_tasks::run_count + 1),
            ))
            .execute(&mut *conn)
            .context("Failed to update next run time")?;

        Ok(())
    }

    /// Mark a task as failed
    pub fn mark_failed(&self, task_id: Uuid, error: &str) -> Result<()> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        diesel::update(scheduled_tasks::table.filter(scheduled_tasks::id.eq(task_id)))
            .set((
                scheduled_tasks::status.eq("failed"),
                scheduled_tasks::last_run_at.eq(Utc::now()),
                scheduled_tasks::last_error.eq(error),
                scheduled_tasks::run_count.eq(scheduled_tasks::run_count + 1),
            ))
            .execute(&mut *conn)
            .context("Failed to mark task as failed")?;

        Ok(())
    }

    /// Cancel a task
    pub fn cancel_task(&self, task_id: Uuid) -> Result<bool> {
        let mut conn = self
            .conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock error: {}", e))?;

        let updated = diesel::update(
            scheduled_tasks::table
                .filter(scheduled_tasks::id.eq(task_id))
                .filter(scheduled_tasks::status.eq("pending")),
        )
        .set(scheduled_tasks::status.eq("cancelled"))
        .execute(&mut *conn)
        .context("Failed to cancel task")?;

        Ok(updated > 0)
    }
}

// ============================================================================
// Cron Utilities
// ============================================================================

/// Parse a cron expression and validate it
pub fn parse_cron(expression: &str) -> Result<Schedule> {
    Schedule::from_str(expression)
        .map_err(|e| anyhow::anyhow!("Invalid cron expression '{}': {}", expression, e))
}

/// Calculate the next run time from a cron expression in a specific timezone
pub fn next_cron_time(cron_expr: &str, timezone: &str) -> Result<DateTime<Utc>> {
    let schedule = parse_cron(cron_expr)?;
    let tz: Tz = timezone
        .parse()
        .map_err(|_| anyhow::anyhow!("Invalid timezone: {}", timezone))?;

    // Get current time in the specified timezone
    let now_in_tz = Utc::now().with_timezone(&tz);

    // Find next occurrence
    let next = schedule
        .after(&now_in_tz)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No future occurrences for cron expression"))?;

    // Convert back to UTC
    Ok(next.with_timezone(&Utc))
}

/// Parse an ISO datetime string
pub fn parse_datetime(s: &str) -> Result<DateTime<Utc>> {
    // Try parsing with timezone
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Ok(dt.with_timezone(&Utc));
    }

    // Try parsing as naive datetime and assume UTC
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S") {
        return Ok(dt.and_utc());
    }

    // Try other common formats
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Ok(dt.and_utc());
    }

    Err(anyhow::anyhow!(
        "Invalid datetime format: '{}'. Use ISO 8601 format (e.g., 2026-01-26T15:30:00Z or 2026-01-26T15:30:00-06:00)",
        s
    ))
}

/// Determine if a string is a cron expression or datetime
pub fn is_cron_expression(s: &str) -> bool {
    // Cron expressions have 5-7 space-separated fields
    let parts: Vec<&str> = s.split_whitespace().collect();
    parts.len() >= 5 && parts.len() <= 7
}
// ============================================================================
// Background Scheduler Runner
// ============================================================================

use tokio::sync::mpsc;

/// A scheduled task execution event sent to the main loop
#[derive(Debug, Clone)]
pub struct ScheduledTaskEvent {
    pub task: ScheduledTask,
}

/// Spawn the background scheduler polling task
/// Returns a channel receiver for scheduled task events
pub fn spawn_scheduler(
    scheduler_db: Arc<SchedulerDb>,
    poll_interval_secs: u64,
) -> mpsc::Receiver<ScheduledTaskEvent> {
    let (tx, rx) = mpsc::channel::<ScheduledTaskEvent>(100);

    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(tokio::time::Duration::from_secs(poll_interval_secs));

        loop {
            interval.tick().await;

            // Get due tasks
            match scheduler_db.get_due_tasks() {
                Ok(tasks) => {
                    for task in tasks {
                        tracing::debug!("Found due task: {} ({})", task.description, task.id);

                        // Mark as running
                        if let Err(e) = scheduler_db.mark_running(task.id) {
                            tracing::error!("Failed to mark task {} as running: {}", task.id, e);
                            continue;
                        }

                        // Send to main loop for processing
                        if tx.send(ScheduledTaskEvent { task }).await.is_err() {
                            tracing::warn!(
                                "Scheduler channel closed, stopping background scheduler"
                            );
                            return;
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to poll scheduled tasks: {}", e);
                }
            }
        }
    });

    rx
}

/// Complete a task after successful execution
#[allow(dead_code)]
pub fn complete_task(scheduler_db: &SchedulerDb, task: &ScheduledTask) -> Result<()> {
    if let Some(ref cron_expr) = task.cron_expression {
        // Recurring task - calculate next run time
        let next_run = next_cron_time(cron_expr, &task.timezone)?;
        scheduler_db.update_next_run(task.id, next_run)?;
        tracing::info!(
            "Rescheduled recurring task '{}' for {}",
            task.description,
            next_run.format("%Y-%m-%d %H:%M:%S UTC")
        );
    } else {
        // One-off task - mark as completed
        scheduler_db.mark_completed(task.id)?;
        tracing::info!("Completed one-off task '{}'", task.description);
    }
    Ok(())
}

/// Mark a task as failed
#[allow(dead_code)]
pub fn fail_task(scheduler_db: &SchedulerDb, task: &ScheduledTask, error: &str) -> Result<()> {
    scheduler_db.mark_failed(task.id, error)?;
    tracing::error!("Task '{}' failed: {}", task.description, error);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cron() {
        // Valid expressions (cron crate uses 6 fields: sec min hour day month dow)
        assert!(parse_cron("0 0 9 * * 1-5").is_ok()); // Weekdays at 9am
        assert!(parse_cron("0 */15 * * * *").is_ok()); // Every 15 minutes
        assert!(parse_cron("0 0 0 1 * *").is_ok()); // First of month

        // Invalid expressions
        assert!(parse_cron("invalid").is_err());
        assert!(parse_cron("0 99 * * *").is_err()); // Invalid hour
    }

    #[test]
    fn test_parse_datetime() {
        // ISO 8601 with timezone
        assert!(parse_datetime("2026-01-26T15:30:00Z").is_ok());
        assert!(parse_datetime("2026-01-26T15:30:00-06:00").is_ok());

        // Without timezone (assumes UTC)
        assert!(parse_datetime("2026-01-26T15:30:00").is_ok());

        // Invalid
        assert!(parse_datetime("not a date").is_err());
    }

    #[test]
    fn test_is_cron_expression() {
        assert!(is_cron_expression("0 9 * * MON-FRI"));
        assert!(is_cron_expression("*/15 * * * *"));
        assert!(!is_cron_expression("2026-01-26T15:30:00Z"));
        assert!(!is_cron_expression("in 2 hours"));
    }
}
