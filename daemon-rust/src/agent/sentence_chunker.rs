use std::collections::HashSet;

pub struct SentenceChunker {
    buffer: String,
}

impl Default for SentenceChunker {
    fn default() -> Self {
        Self::new()
    }
}

impl SentenceChunker {
    pub const CLAUSE_SPLIT_THRESHOLD: usize = 140;

    pub fn new() -> Self {
        Self {
            buffer: String::with_capacity(512),
        }
    }

    /// Appends an incremental text delta from the LLM stream and returns any completed speakable sentences/clauses.
    pub fn push_delta(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut chunks = Vec::new();

        while let Some(split_idx) = self.find_next_split_boundary() {
            let chunk_raw = self.buffer[..split_idx].to_string();
            self.buffer = self.buffer[split_idx..].trim_start().to_string();

            let cleaned = Self::clean_markdown(&chunk_raw);
            if !cleaned.is_empty() {
                chunks.push(cleaned);
            }
        }

        chunks
    }

    /// Flushes any trailing speech text upon stream EOF.
    pub fn flush(&mut self) -> Option<String> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return None;
        }

        let raw = std::mem::take(&mut self.buffer);
        let cleaned = Self::clean_markdown(&raw);
        if cleaned.is_empty() {
            None
        } else {
            Some(cleaned)
        }
    }

    /// Cleans Markdown artifacts so Piper TTS speaks natural prose without audible formatting noise.
    pub fn clean_markdown(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            match c {
                '*' => {
                    // Skip single or double asterisks
                    while chars.peek() == Some(&'*') {
                        chars.next();
                    }
                }
                '_' => {
                    // Skip underscores
                    while chars.peek() == Some(&'_') {
                        chars.next();
                    }
                }
                '`' => {
                    // Skip backticks
                    while chars.peek() == Some(&'`') {
                        chars.next();
                    }
                }
                '#' => {
                    // Only strip leading headers (# Header)
                    if out.is_empty() || out.ends_with('\n') {
                        while chars.peek() == Some(&'#') || chars.peek() == Some(&' ') {
                            chars.next();
                        }
                    } else {
                        out.push(c);
                    }
                }
                '[' => {
                    // Markdown link: [text](url) -> keep text, skip url
                    let mut link_text = String::new();
                    let mut closed = false;
                    for lc in chars.by_ref() {
                        if lc == ']' {
                            closed = true;
                            break;
                        }
                        link_text.push(lc);
                    }
                    if closed && chars.peek() == Some(&'(') {
                        chars.next(); // '('
                        for uc in chars.by_ref() {
                            if uc == ')' {
                                break;
                            }
                        }
                    }
                    out.push_str(&link_text);
                }
                _ => {
                    out.push(c);
                }
            }
        }

        // Clean bullet list markers from beginnings of lines
        let cleaned_lines: Vec<String> = out
            .lines()
            .map(|l| {
                let trimmed = l.trim_start();
                if trimmed.starts_with("- ") || trimmed.starts_with("* ") || trimmed.starts_with("+ ") {
                    trimmed[2..].trim_start().to_string()
                } else {
                    l.to_string()
                }
            })
            .collect();

        let joined = cleaned_lines.join(" ");
        // Collapse repeated whitespace
        let words: Vec<&str> = joined.split_whitespace().collect();
        words.join(" ")
    }

    /// Scans self.buffer for the next split boundary.
    fn find_next_split_boundary(&self) -> Option<usize> {
        let bytes = self.buffer.as_bytes();
        let len = bytes.len();
        if len == 0 {
            return None;
        }

        let mut i = 0;
        while i < len {
            let b = bytes[i];

            // 1. Paragraph break "\n\n"
            if b == b'\n' && i + 1 < len && bytes[i + 1] == b'\n' {
                return Some(i + 2);
            }

            // 2. Exclamation or question mark followed by whitespace or EOF
            if (b == b'!' || b == b'?') && (i + 1 == len || bytes[i + 1].is_ascii_whitespace()) {
                return Some(i + 1);
            }

            // 3. Period followed by whitespace: verify not abbreviation / decimal / file / URL
            if b == b'.'
                && (i + 1 == len || bytes[i + 1].is_ascii_whitespace())
                && !self.is_inside_protected_token(i)
            {
                return Some(i + 1);
            }

            // 4. Long clause boundary: if buffer exceeds threshold, split at comma or semicolon
            if (b == b',' || b == b';' || b == b':')
                && (i + 1 < len && bytes[i + 1].is_ascii_whitespace())
                && (i >= Self::CLAUSE_SPLIT_THRESHOLD)
            {
                return Some(i + 1);
            }

            i += 1;
        }

        None
    }

    /// Returns true if the period at byte index `dot_idx` is part of an abbreviation, decimal, filename, or URL.
    fn is_inside_protected_token(&self, dot_idx: usize) -> bool {
        let bytes = self.buffer.as_bytes();
        let len = bytes.len();

        // 1. Decimal or version check: e.g. "3.14", "0.037", "v2.7"
        if dot_idx > 0
            && bytes[dot_idx - 1].is_ascii_digit()
            && dot_idx + 1 < len
            && bytes[dot_idx + 1].is_ascii_digit()
        {
            return true;
        }

        // 2. Word preceding the period
        let mut start = dot_idx;
        while start > 0 && !bytes[start - 1].is_ascii_whitespace() {
            start -= 1;
        }
        let token_before = &self.buffer[start..dot_idx].to_lowercase();

        // Common abbreviation check
        let protected_abbrs: HashSet<&str> = [
            "dr", "mr", "ms", "mrs", "prof", "sr", "jr", "vs", "etc", "inc", "ltd", "corp",
            "e.g", "i.e", "u.s", "approx", "dept", "est", "min", "max",
        ]
        .into_iter()
        .collect();

        if protected_abbrs.contains(token_before.as_str()) {
            return true;
        }

        // Handle multi-dot acronyms like "U.S." or "e.g." where all components are <= 2 chars
        if token_before.contains('.') {
            let parts: Vec<&str> = token_before.split('.').collect();
            if parts.iter().all(|p| p.len() <= 2) {
                return true;
            }
        }

        // 3. Code filename or URL extension check after dot
        let mut end = dot_idx + 1;
        while end < len && !bytes[end].is_ascii_whitespace() && bytes[end] != b',' && bytes[end] != b';' {
            end += 1;
        }
        let token_after = &self.buffer[dot_idx + 1..end].to_lowercase();

        let known_extensions: HashSet<&str> = [
            "rs", "cpp", "hpp", "h", "c", "py", "js", "ts", "json", "toml", "exe", "dll",
            "bin", "wav", "mp3", "com", "org", "net", "io", "gov", "edu", "html", "md",
        ]
        .into_iter()
        .collect();

        if known_extensions.contains(token_after.as_str()) {
            return true;
        }

        // 4. URL detection
        if token_before.starts_with("http") || token_before.starts_with("www") {
            return true;
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sentence_splitting_basic() {
        let mut chunker = SentenceChunker::new();
        let chunks = chunker.push_delta("System online. Ready for command! How can I assist?");
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0], "System online.");
        assert_eq!(chunks[1], "Ready for command!");
        assert_eq!(chunks[2], "How can I assist?");
        assert_eq!(chunker.flush(), None);
    }

    #[test]
    fn test_abbreviation_preservation() {
        let mut chunker = SentenceChunker::new();
        let chunks = chunker.push_delta("Dr. Smith met with Mr. Davis vs. the U.S. board. The meeting concluded.");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "Dr. Smith met with Mr. Davis vs. the U.S. board.");
        assert_eq!(chunks[1], "The meeting concluded.");
    }

    #[test]
    fn test_decimal_and_filename_preservation() {
        let mut chunker = SentenceChunker::new();
        let chunks = chunker.push_delta("Compiled v2.7.0 in 0.037 seconds via main.rs and codec.rs. All tests pass.");
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0], "Compiled v2.7.0 in 0.037 seconds via main.rs and codec.rs.");
        assert_eq!(chunks[1], "All tests pass.");
    }

    #[test]
    fn test_markdown_stripping() {
        let input = "## System Status\n* **Disk space**: `92%` free\n- Run `cargo test` now.";
        let cleaned = SentenceChunker::clean_markdown(input);
        assert_eq!(cleaned, "System Status Disk space: 92% free Run cargo test now.");
    }

    #[test]
    fn test_long_clause_splitting_over_140_chars() {
        let mut chunker = SentenceChunker::new();
        let long_clause = "I have successfully analyzed the system architecture and validated all security boundaries across all active components in the Better_Jarvis workspace, which confirms that the lock-free ring buffer operates with sub-millisecond latency. Done.";
        assert!(long_clause.len() > SentenceChunker::CLAUSE_SPLIT_THRESHOLD);

        let chunks = chunker.push_delta(long_clause);
        assert_eq!(chunks.len(), 3);
        assert!(chunks[0].starts_with("I have successfully analyzed"));
        assert!(chunks[0].ends_with(','));
        assert_eq!(chunks[1], "which confirms that the lock-free ring buffer operates with sub-millisecond latency.");
        assert_eq!(chunks[2], "Done.");
    }

    #[test]
    fn test_flush_emits_trailing_clause() {
        let mut chunker = SentenceChunker::new();
        let chunks = chunker.push_delta("First sentence. Trailing thought without dot");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "First sentence.");

        let flushed = chunker.flush();
        assert_eq!(flushed, Some("Trailing thought without dot".to_string()));
        assert_eq!(chunker.flush(), None);
    }
}
