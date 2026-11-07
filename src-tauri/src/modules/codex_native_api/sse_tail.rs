//! Incremental completeness probe for optional missing-separator repair.
//! The existing frame parser remains responsible for JSON validation.
#[derive(Default)]
pub(super) struct Tail {
    cursor: usize,
    prefix: Vec<u8>,
    data: bool,
    other: bool,
    started: bool,
    depth: usize,
    quoted: bool,
    escaped: bool,
    complete: bool,
    #[cfg(test)]
    pub(super) visited: usize,
}
impl Tail {
    pub(super) fn complete(&mut self, bytes: &[u8]) -> bool {
        for &byte in &bytes[self.cursor..] {
            #[cfg(test)]
            {
                self.visited += 1;
            }
            if byte == b'\n' {
                self.prefix.clear();
                self.data = false;
                self.other = false;
                continue;
            }
            if self.other {
                continue;
            }
            if !self.data {
                if self.prefix.is_empty() && byte.is_ascii_whitespace() {
                    continue;
                }
                self.prefix.push(byte);
                if b"data:".starts_with(&self.prefix) {
                    self.data = self.prefix.len() == 5;
                } else {
                    self.other = true;
                }
                continue;
            }
            if self.quoted {
                if self.escaped {
                    self.escaped = false;
                } else if byte == b'\\' {
                    self.escaped = true;
                } else if byte == b'"' {
                    self.quoted = false;
                    self.complete = self.depth == 0;
                }
                continue;
            }
            if byte.is_ascii_whitespace() {
                continue;
            }
            match byte {
                b'{' | b'[' => {
                    self.depth += 1;
                    self.complete = false;
                }
                b'}' | b']' => {
                    self.depth = self.depth.saturating_sub(1);
                    self.complete = self.depth == 0;
                }
                b'"' => {
                    self.quoted = true;
                    self.complete = false;
                }
                _ if !self.started => self.complete = true,
                _ => {}
            }
            self.started = true;
        }
        self.cursor = bytes.len();
        self.started && self.complete && !self.quoted
    }
}
