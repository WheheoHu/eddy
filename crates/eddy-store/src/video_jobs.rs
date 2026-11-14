use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::{Result, Store};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VideoJobRecord {
    pub id: String,
    pub source: String,
    pub title: String,
    pub pipeline: String,
    pub transcript_policy: String,
    pub target_language: String,
    pub phase: String,
    /// `queued`, `running`, `succeeded`, `failed`, `canceled`
    pub status: String,
    pub completed_chapters: u32,
    pub total_chapters: u32,
    pub detail: String,
    pub error: Option<String>,
    pub result_series_id: Option<String>,
    pub result_item_id: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

fn map_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<VideoJobRecord> {
    Ok(VideoJobRecord {
        id: r.get(0)?,
        source: r.get(1)?,
        title: r.get(2)?,
        pipeline: r.get(3)?,
        transcript_policy: r.get(4)?,
        target_language: r.get(5)?,
        phase: r.get(6)?,
        status: r.get(7)?,
        completed_chapters: r.get(8)?,
        total_chapters: r.get(9)?,
        detail: r.get(10)?,
        error: r.get(11)?,
        result_series_id: r.get(12)?,
        result_item_id: r.get(13)?,
        created_at: r.get(14)?,
        updated_at: r.get(15)?,
    })
}

const COLS: &str = "id,source,title,pipeline,transcript_policy,target_language,phase,status,completed_chapters,total_chapters,detail,error,result_series_id,result_item_id,created_at,updated_at";

impl Store {
    pub fn upsert_video_job(&self, j: &VideoJobRecord) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO video_jobs(id,source,title,pipeline,transcript_policy,target_language,phase,status,completed_chapters,total_chapters,detail,error,result_series_id,result_item_id,created_at,updated_at)
                 VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)
                 ON CONFLICT(id) DO UPDATE SET
                    title=excluded.title,
                    pipeline=excluded.pipeline,
                    transcript_policy=excluded.transcript_policy,
                    target_language=excluded.target_language,
                    phase=excluded.phase,
                    status=excluded.status,
                    completed_chapters=excluded.completed_chapters,
                    total_chapters=excluded.total_chapters,
                    detail=excluded.detail,
                    error=excluded.error,
                    result_series_id=excluded.result_series_id,
                    result_item_id=excluded.result_item_id,
                    updated_at=excluded.updated_at",
                params![
                    j.id,
                    j.source,
                    j.title,
                    j.pipeline,
                    j.transcript_policy,
                    j.target_language,
                    j.phase,
                    j.status,
                    j.completed_chapters,
                    j.total_chapters,
                    j.detail,
                    j.error,
                    j.result_series_id,
                    j.result_item_id,
                    j.created_at,
                    j.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn video_job(&self, id: &str) -> Result<Option<VideoJobRecord>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLS} FROM video_jobs WHERE id=?"),
                [id],
                map_row,
            )
            .optional()?)
        })
    }

    pub fn video_jobs(&self, limit: u32) -> Result<Vec<VideoJobRecord>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {COLS} FROM video_jobs ORDER BY created_at DESC LIMIT ?"
            ))?;
            let rows = st.query_map([limit], map_row)?;
            rows.collect::<rusqlite::Result<Vec<_>>>().map_err(Into::into)
        })
    }

    pub fn delete_video_job(&self, id: &str) -> Result<bool> {
        self.with(|c| Ok(c.execute("DELETE FROM video_jobs WHERE id=?", [id])? > 0))
    }

    /// Marks any jobs left in `running` or `queued` state from a previous session as `failed`
    /// so the user can resume them cleanly from saved checkpoints.
    pub fn recover_interrupted_video_jobs(&self, now_ms: i64) -> Result<u32> {
        self.with(|c| {
            let n = c.execute(
                "UPDATE video_jobs SET status='failed', error='Interrupted when Eddy closed; click Retry to resume from saved checkpoints.', updated_at=? WHERE status IN ('queued', 'running')",
                [now_ms],
            )?;
            Ok(n as u32)
        })
    }
}
