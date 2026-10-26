//! Guidance history, runtime runs and project analytics.

use std::collections::{BTreeMap, BTreeSet};

use limina_core::{LiminaError, Result, Timestamp, new_uuid, now};
use limina_store::models::{Artifact, Event, InboxMessage, RuntimeRun};
use limina_store::{Arg, args};

use super::paging::{self, Page};
use super::{
    Analytics, AnalyticsWindow, CollaborationService, DailyActivity, GuidanceAnalytics,
    GuidanceItem, KnowledgeAnalytics, Run, RunAnalytics, RunDetail, RunEvent, RunOutcome, project,
};

impl CollaborationService {
    /// Guidance messages, newest first, optionally filtered by status.
    pub async fn guidance(
        &self,
        slug: &str,
        status: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Page<GuidanceItem>> {
        let offset = paging::page_offset(cursor)?;
        let limit = paging::clamp_limit(limit);
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let (filter, args) = status_filter(&challenge.id, status);
        let total: i64 = tx
            .scalar(
                &format!("SELECT CAST(COUNT(*) AS BIGINT) FROM inbox_messages WHERE {filter}"),
                args.clone(),
            )
            .await?;
        let (sql, args) = paged(
            format!("SELECT * FROM inbox_messages WHERE {filter} ORDER BY sequence DESC"),
            args,
            limit,
            offset,
        );
        let rows: Vec<InboxMessage> = tx.fetch_all(&sql, args).await?;
        tx.commit().await?;
        Ok(Page {
            next_cursor: paging::next_cursor(offset, rows.len(), total),
            items: rows.into_iter().map(GuidanceItem::from).collect(),
            total,
        })
    }

    /// Records the start of a runtime turn and returns its run ID.
    pub async fn start_run(
        &self,
        slug: &str,
        runtime_engine: &str,
        model: Option<&str>,
        retry_count: i32,
    ) -> Result<String> {
        let mut tx = self.db.begin_write().await?;
        let challenge = project(&mut tx, slug).await?;
        let id = new_uuid();
        tx.execute(
            "INSERT INTO runtime_runs (id, challenge_id, runtime_engine, model, status, summary, \
             tool_calls, retry_count, started_at) VALUES ($1, $2, $3, $4, 'RUNNING', '', 0, $5, $6)",
            args![&id, &challenge.id, runtime_engine, model, retry_count, now()],
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Counts a tool call against a run. Other events need no bookkeeping.
    pub async fn note_run_event(&self, run_id: &str, tool_call: bool) -> Result<()> {
        if !tool_call {
            return Ok(());
        }
        let mut tx = self.db.begin_write().await?;
        tx.execute(
            "UPDATE runtime_runs SET tool_calls = tool_calls + 1 WHERE id = $1",
            args![run_id],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Records how a run ended. Unknown runs are ignored.
    pub async fn finish_run(&self, run_id: &str, outcome: &RunOutcome) -> Result<()> {
        let completed_at = now();
        let mut tx = self.db.begin_write().await?;
        let started_at: Option<Timestamp> = tx
            .scalar_optional("SELECT started_at FROM runtime_runs WHERE id = $1", args![run_id])
            .await?;
        let Some(started_at) = started_at else {
            return Ok(());
        };
        // Python: max(0, int((completed_at - started_at).total_seconds() * 1000)).
        let duration_ms = ((elapsed_seconds(started_at, completed_at) * 1000.0) as i64).max(0);
        let usage = &outcome.usage;
        tx.execute(
            "UPDATE runtime_runs SET status = $1, summary = $2, error_code = $3, \
             error_message = $4, turn_id = $5, input_tokens = $6, output_tokens = $7, \
             cached_input_tokens = $8, reasoning_output_tokens = $9, total_tokens = $10, \
             cost_microusd = $11, usage_source = $12, cost_source = $13, completed_at = $14, \
             duration_ms = $15 WHERE id = $16",
            args![
                &outcome.status,
                &outcome.summary,
                outcome.error_code.as_deref(),
                outcome.error_message.as_deref(),
                outcome.turn_id.as_deref(),
                usage.input_tokens,
                usage.output_tokens,
                usage.cached_input_tokens,
                usage.reasoning_output_tokens,
                usage.total_tokens,
                usage.cost_microusd,
                usage.usage_source.as_deref(),
                usage.cost_source.as_deref(),
                completed_at,
                duration_ms,
                run_id
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    /// Runtime runs, newest first, optionally filtered by status.
    pub async fn runs(
        &self,
        slug: &str,
        status: Option<&str>,
        cursor: Option<&str>,
        limit: i64,
    ) -> Result<Page<Run>> {
        let offset = paging::page_offset(cursor)?;
        let limit = paging::clamp_limit(limit);
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let (filter, args) = status_filter(&challenge.id, status);
        let total: i64 = tx
            .scalar(
                &format!("SELECT CAST(COUNT(*) AS BIGINT) FROM runtime_runs WHERE {filter}"),
                args.clone(),
            )
            .await?;
        let (sql, args) = paged(
            format!("SELECT * FROM runtime_runs WHERE {filter} ORDER BY started_at DESC"),
            args,
            limit,
            offset,
        );
        let rows: Vec<RuntimeRun> = tx.fetch_all(&sql, args).await?;
        tx.commit().await?;
        Ok(Page {
            next_cursor: paging::next_cursor(offset, rows.len(), total),
            items: rows.into_iter().map(Run::from).collect(),
            total,
        })
    }

    /// One run with the events it recorded (those whose payload carries its `run_id`).
    pub async fn run(&self, slug: &str, run_id: &str) -> Result<RunDetail> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let run: Option<RuntimeRun> = tx
            .fetch_optional(
                "SELECT * FROM runtime_runs WHERE challenge_id = $1 AND id = $2",
                args![&challenge.id, run_id],
            )
            .await?;
        let Some(run) = run else {
            return Err(LiminaError::not_found("Runtime run does not exist."));
        };
        let events: Vec<Event> = tx
            .fetch_all(
                "SELECT * FROM events WHERE challenge_id = $1 AND payload ->> 'run_id' = $2 \
                 ORDER BY sequence",
                args![&challenge.id, run_id],
            )
            .await?;
        tx.commit().await?;
        Ok(RunDetail { run: run.into(), events: events.into_iter().map(RunEvent::from).collect() })
    }

    /// The retry ordinal for the next run, recovered from the database after a restart: one more
    /// than the latest run's when that run failed, otherwise 0.
    pub async fn next_retry_count(&self, slug: &str) -> Result<i32> {
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        let latest: Option<RuntimeRun> = tx
            .fetch_optional(
                "SELECT * FROM runtime_runs WHERE challenge_id = $1 ORDER BY started_at DESC LIMIT 1",
                args![&challenge.id],
            )
            .await?;
        tx.commit().await?;
        Ok(match latest {
            Some(run) if run.status == "FAILED" => run.retry_count.saturating_add(1),
            _ => 0,
        })
    }

    /// Runs, knowledge and guidance over the last `days` days (clamped to 1–365).
    pub async fn analytics(&self, slug: &str, days: i64) -> Result<Analytics> {
        let since = now() - chrono::Duration::days(days.clamp(1, 365));
        let mut tx = self.db.begin().await?;
        let challenge = project(&mut tx, slug).await?;
        // No ORDER BY, like the Python runtime: count keys appear in the database's row order.
        let runs: Vec<RuntimeRun> = tx
            .fetch_all(
                "SELECT * FROM runtime_runs WHERE challenge_id = $1 AND started_at >= $2",
                args![&challenge.id, since],
            )
            .await?;
        let artifacts: Vec<Artifact> = tx
            .fetch_all(
                "SELECT * FROM artifacts WHERE challenge_id = $1 AND created_at >= $2",
                args![&challenge.id, since],
            )
            .await?;
        let guidance: Vec<InboxMessage> = tx
            .fetch_all(
                "SELECT * FROM inbox_messages WHERE challenge_id = $1 AND created_at >= $2",
                args![&challenge.id, since],
            )
            .await?;
        tx.commit().await?;
        Ok(summarize(days, since, &runs, &artifacts, &guidance))
    }
}

/// `challenge_id = $1`, plus `status = $2` when a status is given (upper-cased).
fn status_filter(challenge_id: &str, status: Option<&str>) -> (String, Vec<Arg>) {
    match status.filter(|status| !status.is_empty()) {
        Some(status) => (
            "challenge_id = $1 AND status = $2".to_owned(),
            args![challenge_id, status.to_uppercase()],
        ),
        None => ("challenge_id = $1".to_owned(), args![challenge_id]),
    }
}

/// Appends `LIMIT`/`OFFSET` placeholders after the existing arguments.
fn paged(sql: String, mut args: Vec<Arg>, limit: i64, offset: i64) -> (String, Vec<Arg>) {
    let next = args.len() + 1;
    args.push(limit.into());
    args.push(offset.into());
    (format!("{sql} LIMIT ${next} OFFSET ${}", next + 1), args)
}

/// Python's `timedelta.total_seconds()`: microseconds divided as a float.
fn elapsed_seconds(from: Timestamp, to: Timestamp) -> f64 {
    let micros = (to - from).num_microseconds().unwrap_or(i64::MAX);
    micros as f64 / 1_000_000.0
}

fn summarize(
    days: i64,
    since: Timestamp,
    runs: &[RuntimeRun],
    artifacts: &[Artifact],
    guidance: &[InboxMessage],
) -> Analytics {
    let mut durations: Vec<i64> = runs.iter().filter_map(|run| run.duration_ms).collect();
    durations.sort_unstable();
    // Python truncates each latency towards zero with int(total_seconds()).
    let acknowledgement_seconds: Vec<i64> = guidance
        .iter()
        .filter_map(|item| {
            item.acknowledged_at.map(|at| elapsed_seconds(item.created_at, at) as i64)
        })
        .collect();

    // Buckets by UTC day, in date order.
    let mut by_day: BTreeMap<String, DailyActivity> = BTreeMap::new();
    for run in runs {
        let entry = day(&mut by_day, &run.started_at);
        entry.runs += 1;
        match run.status.as_str() {
            "COMPLETED" => entry.completed_runs += 1,
            "FAILED" => entry.failed_runs += 1,
            _ => {}
        }
    }
    for artifact in artifacts {
        day(&mut by_day, &artifact.created_at).artifacts += 1;
    }
    for item in guidance {
        day(&mut by_day, &item.created_at).guidance += 1;
    }

    let completed = runs.iter().filter(|run| run.status == "COMPLETED").count();
    let total = runs.len();
    Analytics {
        window: AnalyticsWindow { days, since },
        runs: RunAnalytics {
            total: count(total),
            by_status: runs.iter().map(|run| run.status.clone()).collect(),
            success_rate: (total > 0).then(|| round_to(completed as f64 / total as f64, 4)),
            average_duration_ms: rounded_mean(&durations),
            p95_duration_ms: percentile(&durations, 0.95),
            input_tokens: sum_nullable(runs.iter().map(|run| run.input_tokens)),
            output_tokens: sum_nullable(runs.iter().map(|run| run.output_tokens)),
            reasoning_output_tokens: sum_nullable(
                runs.iter().map(|run| run.reasoning_output_tokens),
            ),
            total_tokens: sum_nullable(runs.iter().map(|run| run.total_tokens)),
            cost_microusd: sum_nullable(runs.iter().map(|run| run.cost_microusd)),
            usage_sources: distinct_sorted(runs.iter().map(|run| run.usage_source.as_deref())),
            cost_sources: distinct_sorted(runs.iter().map(|run| run.cost_source.as_deref())),
            tool_calls: runs
                .iter()
                .fold(0_i64, |sum, run| sum.saturating_add(i64::from(run.tool_calls))),
        },
        knowledge: KnowledgeAnalytics {
            created: count(artifacts.len()),
            by_kind: artifacts.iter().map(|artifact| artifact.kind.clone()).collect(),
            by_status: artifacts.iter().map(|artifact| artifact.status.clone()).collect(),
        },
        guidance: GuidanceAnalytics {
            total: count(guidance.len()),
            pending: count(guidance.iter().filter(|item| item.status == "PENDING").count()),
            average_acknowledgement_seconds: rounded_mean(&acknowledgement_seconds),
        },
        timeseries: by_day.into_values().collect(),
    }
}

fn day<'a>(
    by_day: &'a mut BTreeMap<String, DailyActivity>,
    at: &Timestamp,
) -> &'a mut DailyActivity {
    let date = at.date_naive().format("%Y-%m-%d").to_string();
    by_day.entry(date.clone()).or_insert_with(|| DailyActivity { date, ..DailyActivity::default() })
}

fn count(value: usize) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// `None` when every value is missing, otherwise the sum of the present ones.
fn sum_nullable(values: impl Iterator<Item = Option<i64>>) -> Option<i64> {
    values
        .flatten()
        .fold(None, |sum: Option<i64>, value| Some(sum.unwrap_or(0).saturating_add(value)))
}

/// Non-empty values, distinct and sorted.
fn distinct_sorted<'a>(values: impl Iterator<Item = Option<&'a str>>) -> Vec<String> {
    let set: BTreeSet<&str> = values.flatten().filter(|value| !value.is_empty()).collect();
    set.into_iter().map(str::to_owned).collect()
}

/// Python's `round(sum(values) / len(values))`: a float mean rounded half to even.
fn rounded_mean(values: &[i64]) -> Option<i64> {
    if values.is_empty() {
        return None;
    }
    let sum = values.iter().fold(0_i128, |sum, value| sum + i128::from(*value));
    Some((sum as f64 / values.len() as f64).round_ties_even() as i64)
}

/// The nearest-rank percentile of sorted values, exactly as the Python runtime computed it.
fn percentile(sorted: &[i64], percentile: f64) -> Option<i64> {
    let last = sorted.len().checked_sub(1)?;
    let rank = ((last as f64) * percentile) as usize;
    sorted.get(rank.min(last)).copied()
}

/// Python's `round(value, digits)`: correctly rounded, ties to even.
fn round_to(value: f64, digits: usize) -> f64 {
    format!("{value:.digits$}").parse().unwrap_or(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statistics_match_python() {
        // Values produced by Python's round(), int() and the runtime's _percentile.
        assert_eq!(round_to(0.03125, 4), 0.0312);
        assert_eq!(round_to(0.00005, 4), 0.0001);
        assert_eq!(round_to(2.0 / 3.0, 4), 0.6667);
        assert_eq!(round_to(0.99995, 4), 1.0);
        assert_eq!(rounded_mean(&[1, 2]), Some(2));
        assert_eq!(rounded_mean(&[1, 4]), Some(2));
        assert_eq!(rounded_mean(&[3, 4]), Some(4));
        assert_eq!(rounded_mean(&[]), None);
        assert_eq!(percentile(&[], 0.95), None);
        assert_eq!(percentile(&[7], 0.95), Some(7));
        let values: Vec<i64> = (1..=20).collect();
        assert_eq!(percentile(&values, 0.95), Some(19));
        let values: Vec<i64> = (1..=21).collect();
        assert_eq!(percentile(&values, 0.95), Some(20));
        assert_eq!(sum_nullable([None, None].into_iter()), None);
        assert_eq!(sum_nullable([None, Some(2), Some(3)].into_iter()), Some(5));
        assert_eq!(
            distinct_sorted(
                [Some("provider"), None, Some(""), Some("estimate"), Some("provider")].into_iter()
            ),
            vec!["estimate", "provider"]
        );
    }

    #[test]
    fn elapsed_time_truncates_like_python() {
        let start = now();
        let cases =
            [(2_999_i64, 2_i64), (1_001_500, 1001), (4_100_000, 4100), (0, 0), (-5_000, -5)];
        for (micros, millis) in cases {
            let end = start + chrono::Duration::microseconds(micros);
            assert_eq!((elapsed_seconds(start, end) * 1000.0) as i64, millis);
        }
    }
}
