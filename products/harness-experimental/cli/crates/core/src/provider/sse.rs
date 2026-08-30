//! Minimal Server-Sent Events framing: feed raw bytes in, get `data:` payloads
//! out. Only the subset the DeepSeek streaming API uses (data lines, `[DONE]`
//! terminator); comments and other fields are ignored.

#[derive(Debug, Default)]
pub struct SseParser {
    buffer: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of bytes; returns the `data:` payloads of every event that
    /// completed with this chunk. Invalid UTF-8 at a chunk boundary is handled
    /// by buffering raw bytes until a full line exists.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<String> {
        self.buffer.push_str(&String::from_utf8_lossy(chunk));
        let mut events = Vec::new();
        // An SSE event ends at a blank line. Normalize CRLF.
        while let Some(boundary) = find_event_boundary(&self.buffer) {
            let raw: String = self.buffer.drain(..boundary.end).collect();
            let event_text = &raw[..boundary.start];
            let mut data_lines = Vec::new();
            for line in event_text.lines() {
                if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
                }
            }
            if !data_lines.is_empty() {
                events.push(data_lines.join("\n"));
            }
        }
        events
    }
}

struct Boundary {
    /// Byte offset where the event text ends.
    start: usize,
    /// Byte offset just past the blank-line separator.
    end: usize,
}

fn find_event_boundary(buffer: &str) -> Option<Boundary> {
    let lf = buffer.find("\n\n").map(|i| Boundary {
        start: i,
        end: i + 2,
    });
    let crlf = buffer.find("\r\n\r\n").map(|i| Boundary {
        start: i,
        end: i + 4,
    });
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.start <= b.start { a } else { b }),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_events_across_chunk_boundaries() {
        let mut parser = SseParser::new();
        assert!(parser.feed(b"data: {\"a\":").is_empty());
        let events = parser.feed(b"1}\n\ndata: [DONE]\n\n");
        assert_eq!(events, vec!["{\"a\":1}".to_string(), "[DONE]".to_string()]);
    }

    #[test]
    fn handles_crlf_and_ignores_comments() {
        let mut parser = SseParser::new();
        let events = parser.feed(b": keep-alive\r\n\r\ndata: x\r\n\r\n");
        assert_eq!(events, vec!["x".to_string()]);
    }
}
