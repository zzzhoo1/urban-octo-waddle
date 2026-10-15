use std::time::Instant;

use bytes::Bytes;

use crate::openai::{ChatCompletionChunk, Usage};

/// Side-channel observer for SSE streams: watches frames as they pass
/// through untouched, recording time-to-first-byte and the final usage
/// frame (tokens, credit, cache hits) for the audit log.
pub struct SseTap {
    started: Instant,
    first_byte_ms: Option<u64>,
    buffer: Vec<u8>,
    final_usage: Option<Usage>,
}

/// Outcome of a finished (or aborted) stream, folded into the log entry.
#[derive(Debug, Clone, Copy, Default)]
pub struct StreamAudit {
    pub ttft_ms: u64,
    pub total_ms: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub credit: Option<f64>,
    pub cached_tokens: Option<u64>,
}

impl SseTap {
    pub fn new() -> Self {
        Self {
            started: Instant::now(),
            first_byte_ms: None,
            buffer: Vec::with_capacity(8192),
            final_usage: None,
        }
    }

    /// Observe one chunk flowing downstream. The bytes are returned
    /// unchanged; this only updates the tap's bookkeeping.
    pub fn observe(&mut self, chunk: &Bytes) {
        if self.first_byte_ms.is_none() && !chunk.is_empty() {
            self.first_byte_ms = Some(self.started.elapsed().as_millis() as u64);
        }
        self.buffer.extend_from_slice(chunk);
        // Keep memory bounded on pathological streams without newlines.
        if self.buffer.len() > 1 << 20 {
            let keep_from = self.buffer.len() - (1 << 19);
            self.buffer.drain(..keep_from);
        }
        self.extract_complete_lines();
    }

    /// Parse any complete `data:` lines accumulated so far.
    fn extract_complete_lines(&mut self) {
        // Only parse up to the last newline; the tail may be a partial line.
        let Some(last_nl) = self.buffer.iter().rposition(|b| *b == b'\n') else {
            return;
        };
        let complete: Vec<u8> = self.buffer.drain(..=last_nl).collect();
        let text = String::from_utf8_lossy(&complete);
        for line in text.lines() {
            let line = line.trim();
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(payload) {
                if let Some(u) = chunk.usage {
                    if u.total_tokens > 0 || u.credit.is_some() || u.cached_tokens().is_some() {
                        self.final_usage = Some(u);
                    }
                }
            }
        }
    }

    /// Clone-based finish used by the Drop-based finalizer (cannot move).
    pub fn clone_finish(&self) -> StreamAudit {
        let usage = self.final_usage.clone().unwrap_or_default();
        StreamAudit {
            ttft_ms: self.first_byte_ms.unwrap_or(0),
            total_ms: self.started.elapsed().as_millis() as u64,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            credit: usage.credit,
            cached_tokens: usage.cached_tokens(),
        }
    }

    /// Fold the observation into the final audit values.
    pub fn finish(mut self) -> StreamAudit {
        // Flush whatever partial line remains (streams often end without a
        // trailing newline on the final data frame).
        if !self.buffer.is_empty() {
            let tail = std::mem::take(&mut self.buffer);
            let text = String::from_utf8_lossy(&tail);
            for line in text.lines() {
                let line = line.trim();
                let Some(payload) = line.strip_prefix("data:") else {
                    continue;
                };
                let payload = payload.trim();
                if payload.is_empty() || payload == "[DONE]" {
                    continue;
                }
                if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(payload) {
                    if let Some(u) = chunk.usage {
                        if u.total_tokens > 0 || u.credit.is_some() || u.cached_tokens().is_some() {
                            self.final_usage = Some(u);
                        }
                    }
                }
            }
        }
        let usage = self.final_usage.unwrap_or_default();
        StreamAudit {
            ttft_ms: self.first_byte_ms.unwrap_or(0),
            total_ms: self.started.elapsed().as_millis() as u64,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: usage.total_tokens,
            credit: usage.credit,
            cached_tokens: usage.cached_tokens(),
        }
    }
}
